use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, bail};
use crossbeam_channel::Sender;
use punctual_browser::{BrowserPage, ManualValidation, validate_manual_text};
use punctual_core::{
    EngineCommand, EngineEvent, ManualTargetValidation, TargetCandidate, TaskStatus,
    utc_now_millis,
};
use punctual_storage::SqliteTaskRepository;
use tokio::{sync::mpsc, task::JoinHandle, time::MissedTickBehavior};
use url::Url;
use uuid::Uuid;

use crate::{EngineConfig, browser_hub::BrowserHub, worker};

struct InspectionSession {
    url: Url,
    page: BrowserPage,
    candidates: Vec<TargetCandidate>,
}

pub(crate) struct Engine {
    repository: Arc<SqliteTaskRepository>,
    config: EngineConfig,
    commands: mpsc::UnboundedReceiver<EngineCommand>,
    events: Sender<EngineEvent>,
    browser: Arc<BrowserHub>,
    inspections: HashMap<Uuid, InspectionSession>,
    workers: HashMap<Uuid, JoinHandle<()>>,
}

impl Engine {
    pub(crate) fn new(
        repository: Arc<SqliteTaskRepository>,
        config: EngineConfig,
        commands: mpsc::UnboundedReceiver<EngineCommand>,
        events: Sender<EngineEvent>,
    ) -> Self {
        let browser = Arc::new(BrowserHub::new(&config, events.clone()));
        Self {
            repository,
            config,
            commands,
            events,
            browser,
            inspections: HashMap::new(),
            workers: HashMap::new(),
        }
    }

    pub(crate) async fn run(mut self) {
        let mut ticker = tokio::time::interval(Duration::from_millis(
            self.config.scheduler_tick_ms.max(25),
        ));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        if let Err(error) = self.reconcile_workers().await {
            self.emit_failure(None, None, "scheduler_startup", error);
        }

        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        break;
                    };
                    if matches!(command, EngineCommand::Shutdown) {
                        break;
                    }

                    let request_id = command.request_id();
                    let task_id = command.task_id();
                    let operation = command.operation().to_owned();
                    if let Err(error) = self.handle_command(command).await {
                        self.emit_failure(request_id, task_id, operation, error);
                    }
                }
                _ = ticker.tick() => {
                    if let Err(error) = self.reconcile_workers().await {
                        self.emit_failure(None, None, "scheduler_tick", error);
                    }
                }
            }
        }

        self.shutdown().await;
    }

    async fn handle_command(&mut self, command: EngineCommand) -> Result<()> {
        match command {
            EngineCommand::DetectTargets { request_id, url } => {
                let page = self.browser.open(&url).await?;
                tokio::time::sleep(Duration::from_millis(self.config.page_settle_ms)).await;
                let candidates = self.browser.detect_targets(&page).await?;

                // The current UI has one active editor. Keep only the latest
                // control reference; the visible Chromium tab may remain open
                // so the user can review it or close it manually.
                self.inspections.clear();
                self.inspections.insert(
                    request_id,
                    InspectionSession {
                        url: url.clone(),
                        page,
                        candidates: candidates.clone(),
                    },
                );
                let _ = self.events.send(EngineEvent::TargetsDetected {
                    request_id,
                    url,
                    candidates,
                });
            }
            EngineCommand::ValidateManualTarget { request_id, text } => {
                let (page, expected_url) = self
                    .inspections
                    .get(&request_id)
                    .map(|session| (session.page.clone(), session.url.clone()))
                    .context("请先打开 URL 并检测页面按钮")?;
                let candidates = self.browser.detect_targets(&page).await?;
                if let Some(session) = self.inspections.get_mut(&request_id) {
                    session.candidates = candidates.clone();
                    session.url = expected_url;
                }

                let validation = match validate_manual_text(&text, &candidates) {
                    ManualValidation::Unique(value) => ManualTargetValidation::Unique(value),
                    ManualValidation::Multiple(values) => {
                        ManualTargetValidation::Multiple(values)
                    }
                    ManualValidation::NotClickable(values) => {
                        ManualTargetValidation::NotClickable(values)
                    }
                    ManualValidation::NotFound => ManualTargetValidation::NotFound,
                };
                let _ = self.events.send(EngineEvent::ManualTargetValidated {
                    request_id,
                    text,
                    validation,
                });
            }
            EngineCommand::HighlightTarget { request_id, target } => {
                let page = self
                    .inspections
                    .get(&request_id)
                    .map(|session| session.page.clone())
                    .context("请先打开 URL 并检测页面按钮")?;
                let found = self.browser.highlight(&page, &target).await?;
                let _ = self.events.send(EngineEvent::TargetHighlighted {
                    request_id,
                    found,
                });
            }
            EngineCommand::SaveTask { request_id, task } => {
                if task.status != TaskStatus::Pending {
                    bail!("只能保存处于待执行状态的任务");
                }
                if !task.target.is_verified() {
                    bail!("任务缺少经过页面验证的目标按钮");
                }
                if task.scheduled_at_utc <= utc_now_millis() {
                    bail!("任务执行时间必须晚于当前时间");
                }

                if self
                    .repository
                    .get(task.id)?
                    .is_some_and(|existing| existing.status == TaskStatus::Executing)
                {
                    bail!("任务已经进入点击派发阶段，不能编辑或重新安排，以免重复提交");
                }
                self.stop_worker(task.id).await;
                // The worker runs concurrently with the command loop. Check a
                // second time after cancellation so a task that crossed into
                // Executing during the first check is never overwritten by a
                // stale editor snapshot.
                if self
                    .repository
                    .get(task.id)?
                    .is_some_and(|existing| existing.status == TaskStatus::Executing)
                {
                    bail!("任务已经进入点击派发阶段，不能编辑或重新安排，以免重复提交");
                }
                self.repository.save(&task)?;
                let _ = self.events.send(EngineEvent::TaskSaved {
                    request_id,
                    task: task.clone(),
                });
                self.reconcile_workers().await?;
            }
            EngineCommand::DeleteTask {
                request_id,
                task_id,
            } => {
                if self
                    .repository
                    .get(task_id)?
                    .is_some_and(|existing| existing.status == TaskStatus::Executing)
                {
                    bail!("任务已经进入点击派发阶段，不能删除；执行结果将保留用于审计");
                }
                self.stop_worker(task_id).await;
                if self
                    .repository
                    .get(task_id)?
                    .is_some_and(|existing| existing.status == TaskStatus::Executing)
                {
                    bail!("任务已经进入点击派发阶段，不能删除；执行结果将保留用于审计");
                }
                self.repository.delete(task_id)?;
                let _ = self.events.send(EngineEvent::TaskDeleted {
                    request_id,
                    task_id,
                });
            }
            EngineCommand::LoadExecutionLogs {
                request_id,
                task_id,
            } => {
                let logs = self.repository.list_execution_logs(task_id)?;
                let _ = self.events.send(EngineEvent::ExecutionLogsLoaded {
                    request_id,
                    task_id,
                    logs,
                });
            }
            EngineCommand::Shutdown => {}
        }
        Ok(())
    }

    async fn reconcile_workers(&mut self) -> Result<()> {
        let finished = self
            .workers
            .iter()
            .filter_map(|(task_id, handle)| handle.is_finished().then_some(*task_id))
            .collect::<Vec<_>>();
        for task_id in finished {
            if let Some(handle) = self.workers.remove(&task_id) {
                if let Err(error) = handle.await {
                    if !error.is_cancelled() {
                        self.emit_failure(
                            None,
                            Some(task_id),
                            "scheduler_join",
                            error.into(),
                        );
                    }
                }
            }
        }

        for task in self.repository.list_non_terminal()? {
            if task.status == TaskStatus::Draft || self.workers.contains_key(&task.id) {
                continue;
            }

            let task_id = task.id;
            let repository = Arc::clone(&self.repository);
            let browser = Arc::clone(&self.browser);
            let events = self.events.clone();
            let config = self.config.clone();
            let handle = tokio::spawn(async move {
                worker::run_task(task_id, repository, browser, events, config).await;
            });
            self.workers.insert(task_id, handle);
        }
        Ok(())
    }

    async fn stop_worker(&mut self, task_id: Uuid) {
        if let Some(handle) = self.workers.remove(&task_id) {
            handle.abort();
            let _ = handle.await;
        }
    }

    fn emit_failure(
        &self,
        request_id: Option<Uuid>,
        task_id: Option<Uuid>,
        operation: impl Into<String>,
        error: anyhow::Error,
    ) {
        let _ = self.events.send(EngineEvent::CommandFailed {
            request_id,
            task_id,
            operation: operation.into(),
            message: format!("{error:#}"),
        });
    }

    async fn shutdown(&mut self) {
        let workers = self.workers.drain().map(|(_, handle)| handle).collect::<Vec<_>>();
        for handle in &workers {
            handle.abort();
        }
        for handle in workers {
            let _ = handle.await;
        }
        self.inspections.clear();
        if let Err(error) = self.browser.shutdown().await {
            self.emit_failure(None, None, "browser_shutdown", error);
        }
    }
}
