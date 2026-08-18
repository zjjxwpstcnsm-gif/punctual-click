use std::{sync::Arc, time::Duration as StdDuration};

use anyhow::{bail, Context as _, Result};
use chrono::{Duration, Utc};
use crossbeam_channel::Sender;
use punctual_browser::{
    BrowserPage, ClickDispatch, CompletionVerification, FingerprintLocator, RelocationResult,
};
use punctual_core::{
    truncate_to_millis, utc_now_millis, ClickAttemptGuard, ClickMode, ClickTask, CompletionSignal,
    EngineEvent, ExecutionLog, ExecutionOutcome, ExecutionPlan, ExecutionResult, PreciseTimer,
    TargetFingerprint, TaskStatus,
};
use punctual_storage::SqliteTaskRepository;
use url::Url;
use uuid::Uuid;

use crate::{browser_hub::BrowserHub, EngineConfig};

pub(crate) async fn run_task(
    task_id: Uuid,
    repository: Arc<SqliteTaskRepository>,
    browser: Arc<BrowserHub>,
    events: Sender<EngineEvent>,
    config: EngineConfig,
) {
    if let Err(error) = run_task_inner(
        task_id,
        Arc::clone(&repository),
        Arc::clone(&browser),
        events.clone(),
        config,
    )
    .await
    {
        let message = format!("任务执行器异常退出：{error:#}");
        let fallback = repository.get(task_id).ok().flatten().and_then(|mut task| {
            if task.status.is_terminal() || task.status == TaskStatus::Draft {
                return None;
            }
            let outcome = if task.status == TaskStatus::Executing {
                ExecutionOutcome::Uncertain
            } else {
                ExecutionOutcome::Failed
            };
            let result = ExecutionResult {
                outcome,
                scheduled_at: task.scheduled_at_utc,
                dispatched_at: None,
                observed_click_at: None,
                dispatch_delay_ms: None,
                observed_delay_ms: None,
                final_url: Some(task.url.clone()),
                message: message.clone(),
                error_code: Some("worker_internal_error".into()),
                screenshot_path: None,
            };
            persist_terminal(&repository, &events, &mut task, result).err()
        });
        let failure_message = fallback
            .map(|persist_error| format!("{message}；同时无法写入终态：{persist_error:#}"))
            .unwrap_or(message);
        let _ = events.send(EngineEvent::CommandFailed {
            request_id: None,
            task_id: Some(task_id),
            operation: "scheduler_worker".into(),
            message: failure_message,
        });
    }
}

async fn run_task_inner(
    task_id: Uuid,
    repository: Arc<SqliteTaskRepository>,
    browser: Arc<BrowserHub>,
    events: Sender<EngineEvent>,
    config: EngineConfig,
) -> Result<()> {
    let Some(mut task) = repository.get(task_id)? else {
        return Ok(());
    };

    if task.status.is_terminal() || task.status == TaskStatus::Draft {
        return Ok(());
    }

    // Preparing and armed tasks are safe to prepare again after a process
    // restart. An executing task is not safe to repeat because the previous
    // process may already have dispatched its only click.
    if task.recover_to_pending() {
        repository.save(&task)?;
        emit_status(&events, &task);
    } else if task.status == TaskStatus::Executing {
        let result = ExecutionResult {
            outcome: ExecutionOutcome::Uncertain,
            scheduled_at: task.scheduled_at_utc,
            dispatched_at: None,
            observed_click_at: None,
            dispatch_delay_ms: None,
            observed_delay_ms: None,
            final_url: Some(task.url.clone()),
            message: "上次执行在点击阶段中断；为避免重复提交，本次不会再次点击".into(),
            error_code: Some("execution_interrupted".into()),
            screenshot_path: None,
        };
        persist_terminal(&repository, &events, &mut task, result)?;
        return Ok(());
    }

    let plan = match ExecutionPlan::new(
        task.scheduled_at_utc,
        &task.click_mode,
        config.execution_plan,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "invalid_execution_plan",
                format!("执行计划无效：{error}"),
                None,
            )?;
            return Ok(());
        }
    };

    if utc_now_millis() > plan.click_deadline {
        fail_before_click(
            &repository,
            &events,
            &mut task,
            "deadline_missed",
            "应用启动或任务保存时，允许点击的时间窗口已经结束".into(),
            None,
        )?;
        return Ok(());
    }

    sleep_until(plan.prewarm_at).await;
    if utc_now_millis() > plan.click_deadline {
        fail_before_click(
            &repository,
            &events,
            &mut task,
            "deadline_missed",
            "浏览器准备开始前，允许点击的时间窗口已经结束".into(),
            None,
        )?;
        return Ok(());
    }

    task.transition(TaskStatus::Preparing)?;
    repository.save(&task)?;
    emit_status(&events, &task);

    let page = match browser.open(&task.url).await {
        Ok(page) => page,
        Err(error) => {
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "browser_open_failed",
                format!("打开任务页面失败：{error:#}"),
                None,
            )?;
            return Ok(());
        }
    };
    tokio::time::sleep(StdDuration::from_millis(config.page_settle_ms)).await;

    sleep_until(plan.resolve_at).await;
    if utc_now_millis() > plan.click_deadline {
        let final_url = current_page_url(&browser, &page, &task.url).await;
        fail_before_click(
            &repository,
            &events,
            &mut task,
            "deadline_missed",
            "按钮重新定位完成前，允许点击的时间窗口已经结束".into(),
            final_url,
        )?;
        return Ok(());
    }

    let mut target = match resolve_target(&browser, &page, &task).await {
        Ok(target) => target,
        Err(error) => {
            let final_url = current_page_url(&browser, &page, &task.url).await;
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "target_relocation_failed",
                format!("执行前无法唯一定位目标按钮：{error:#}"),
                final_url,
            )?;
            return Ok(());
        }
    };

    sleep_until(plan.arm_at).await;
    if utc_now_millis() > plan.click_deadline {
        let final_url = current_page_url(&browser, &page, &task.url).await;
        fail_before_click(
            &repository,
            &events,
            &mut task,
            "deadline_missed",
            "任务进入布防状态前，允许点击的时间窗口已经结束".into(),
            final_url,
        )?;
        return Ok(());
    }

    // Re-resolve once more at T-1s so a framework re-render between the T-10s
    // scan and the Armed phase cannot leave us holding a detached DOM target.
    target = match resolve_target(&browser, &page, &task).await {
        Ok(target) => target,
        Err(error) => {
            let final_url = current_page_url(&browser, &page, &task.url).await;
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "target_arm_relocation_failed",
                format!("布防前无法唯一定位目标按钮：{error:#}"),
                final_url,
            )?;
            return Ok(());
        }
    };

    // Scroll before the target instant. The exact-deadline probe does not
    // perform layout-changing scroll work; it only verifies the prepared point.
    match browser.prepare_target(&page, &target).await {
        Ok(probe) if probe.found => {}
        Ok(probe) => {
            let final_url = current_page_url(&browser, &page, &task.url).await;
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "target_arm_prepare_failed",
                format!(
                    "布防前无法准备目标按钮：{}",
                    probe.reason.as_deref().unwrap_or("target_not_found")
                ),
                final_url,
            )?;
            return Ok(());
        }
        Err(error) => {
            let final_url = current_page_url(&browser, &page, &task.url).await;
            fail_before_click(
                &repository,
                &events,
                &mut task,
                "target_arm_prepare_failed",
                format!("布防前滚动并复检目标失败：{error:#}"),
                final_url,
            )?;
            return Ok(());
        }
    }

    if utc_now_millis() > plan.click_deadline {
        let final_url = current_page_url(&browser, &page, &task.url).await;
        fail_before_click(
            &repository,
            &events,
            &mut task,
            "deadline_missed",
            "目标布防完成时，允许点击的时间窗口已经结束".into(),
            final_url,
        )?;
        return Ok(());
    }

    task.transition(TaskStatus::Armed)?;
    repository.save(&task)?;
    emit_status(&events, &task);

    // Persist Executing shortly before the deadline so the exact target instant
    // is not spent on SQLite I/O. If the process stops after this transition,
    // startup recovery deliberately refuses to retry the click.
    sleep_until(plan.scheduled_at - Duration::milliseconds(100)).await;
    task.transition(TaskStatus::Executing)?;
    repository.save(&task)?;
    emit_status(&events, &task);

    let guard = ClickAttemptGuard::new();
    let dispatch = dispatch_at_deadline(
        &browser,
        &page,
        &target,
        &task.click_mode,
        &task.completion_signals,
        plan.scheduled_at,
        plan.click_deadline,
        &guard,
        &config,
    )
    .await;

    let dispatch = match dispatch {
        Ok(dispatch) => dispatch,
        Err(error) => {
            let result = ExecutionResult {
                outcome: if guard.is_claimed() {
                    ExecutionOutcome::Uncertain
                } else {
                    ExecutionOutcome::Failed
                },
                scheduled_at: task.scheduled_at_utc,
                dispatched_at: None,
                observed_click_at: None,
                dispatch_delay_ms: None,
                observed_delay_ms: None,
                final_url: current_page_url(&browser, &page, &task.url).await,
                message: if guard.is_claimed() {
                    format!("点击尝试已经进入派发阶段，但浏览器未确认结果：{error:#}")
                } else {
                    format!("目标时刻未能派发点击：{error:#}")
                },
                error_code: Some(if guard.is_claimed() {
                    "click_dispatch_uncertain".into()
                } else {
                    "click_not_dispatched".into()
                }),
                screenshot_path: None,
            };
            persist_terminal(&repository, &events, &mut task, result)?;
            return Ok(());
        }
    };

    let observed_click_at = utc_now_millis();
    let result =
        verify_after_click(&browser, &page, &task, dispatch, observed_click_at, &config).await;
    persist_terminal(&repository, &events, &mut task, result)?;
    Ok(())
}

async fn resolve_target(
    browser: &BrowserHub,
    page: &BrowserPage,
    task: &ClickTask,
) -> Result<TargetFingerprint> {
    let fingerprint = task
        .target
        .fingerprint()
        .context("task does not contain a verified target fingerprint")?;
    let candidates = browser.detect_targets(page).await?;

    match FingerprintLocator::default().relocate_for_execution(fingerprint, &candidates) {
        RelocationResult::Unique(value) => Ok(value.candidate.to_fingerprint()),
        RelocationResult::Ambiguous(values) => bail!(
            "{} candidates matched within the ambiguity threshold",
            values.len()
        ),
        RelocationResult::NotFound => bail!("no candidate matched the saved target fingerprint"),
    }
}

async fn dispatch_at_deadline(
    browser: &BrowserHub,
    page: &BrowserPage,
    target: &TargetFingerprint,
    click_mode: &ClickMode,
    completion_signals: &[CompletionSignal],
    scheduled_at: chrono::DateTime<Utc>,
    click_deadline: chrono::DateTime<Utc>,
    guard: &ClickAttemptGuard,
    config: &EngineConfig,
) -> Result<ClickDispatch> {
    PreciseTimer::new(config.precise_timer)
        .wait_until(scheduled_at)
        .await;

    match click_mode {
        ClickMode::Strict => {
            browser
                .click_once(page, target, guard, completion_signals)
                .await
        }
        ClickMode::WaitUntilClickable { .. } => loop {
            let probe = browser.probe_target(page, target).await?;
            if probe.found && probe.clickable {
                return browser
                    .click_once(page, target, guard, completion_signals)
                    .await;
            }
            if utc_now_millis() >= click_deadline {
                bail!(
                    "button did not become clickable before the grace period ended: {}",
                    probe.reason.as_deref().unwrap_or("unknown_reason")
                );
            }
            tokio::time::sleep(StdDuration::from_millis(
                config.click_probe_interval_ms.max(1),
            ))
            .await;
        },
    }
}

async fn verify_after_click(
    browser: &BrowserHub,
    page: &BrowserPage,
    task: &ClickTask,
    dispatch: ClickDispatch,
    observed_click_at: chrono::DateTime<Utc>,
    config: &EngineConfig,
) -> ExecutionResult {
    let browser_name = dispatch.browser_name.clone();
    let dispatched_at = truncate_to_millis(dispatch.dispatched_at);
    let observed_click_at = truncate_to_millis(observed_click_at);
    let deadline =
        tokio::time::Instant::now() + StdDuration::from_millis(config.completion_timeout_ms.max(1));
    let mut final_url = Some(dispatch.completion_baseline.url.clone());
    let mut last_reason = "点击已派发，但尚未观察到配置的成功信号".to_owned();

    loop {
        match browser
            .verify_completion(page, &dispatch, &task.completion_signals)
            .await
        {
            Ok(CompletionVerification::Succeeded {
                final_url: url,
                evidence,
            }) => {
                return ExecutionResult {
                    outcome: ExecutionOutcome::Succeeded,
                    scheduled_at: task.scheduled_at_utc,
                    dispatched_at: Some(dispatched_at),
                    observed_click_at: Some(observed_click_at),
                    dispatch_delay_ms: Some(
                        (dispatched_at - task.scheduled_at_utc).num_milliseconds(),
                    ),
                    observed_delay_ms: Some(
                        (observed_click_at - task.scheduled_at_utc).num_milliseconds(),
                    ),
                    final_url: Some(url),
                    message: format!("浏览器：{browser_name}；点击已派发并确认成功：{evidence}"),
                    error_code: None,
                    screenshot_path: None,
                };
            }
            Ok(CompletionVerification::Uncertain {
                current_url,
                reason,
            }) => {
                final_url = Some(current_url);
                last_reason = reason;
            }
            Err(error) => {
                last_reason = format!("读取页面完成信号失败：{error:#}");
            }
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(config.completion_poll_ms.max(1))).await;
    }

    ExecutionResult {
        outcome: ExecutionOutcome::Uncertain,
        scheduled_at: task.scheduled_at_utc,
        dispatched_at: Some(dispatched_at),
        observed_click_at: Some(observed_click_at),
        dispatch_delay_ms: Some((dispatched_at - task.scheduled_at_utc).num_milliseconds()),
        observed_delay_ms: Some((observed_click_at - task.scheduled_at_utc).num_milliseconds()),
        final_url,
        message: format!("浏览器：{browser_name}；{last_reason}"),
        error_code: Some("completion_not_confirmed".into()),
        screenshot_path: None,
    }
}

fn fail_before_click(
    repository: &SqliteTaskRepository,
    events: &Sender<EngineEvent>,
    task: &mut ClickTask,
    code: &str,
    message: String,
    final_url: Option<Url>,
) -> Result<()> {
    let result = ExecutionResult {
        outcome: ExecutionOutcome::Failed,
        scheduled_at: task.scheduled_at_utc,
        dispatched_at: None,
        observed_click_at: None,
        dispatch_delay_ms: None,
        observed_delay_ms: None,
        final_url: final_url.or_else(|| Some(task.url.clone())),
        message,
        error_code: Some(code.into()),
        screenshot_path: None,
    };
    persist_terminal(repository, events, task, result)
}

fn persist_terminal(
    repository: &SqliteTaskRepository,
    events: &Sender<EngineEvent>,
    task: &mut ClickTask,
    result: ExecutionResult,
) -> Result<()> {
    task.finish(result.clone())?;
    let log = ExecutionLog::from_result(task.id, &result);
    repository.save_task_and_log(task, &log)?;
    emit_status(events, task);
    let _ = events.send(EngineEvent::TaskCompleted {
        task: task.clone(),
        log,
    });
    Ok(())
}

fn emit_status(events: &Sender<EngineEvent>, task: &ClickTask) {
    let _ = events.send(EngineEvent::TaskStatusChanged {
        task_id: task.id,
        status: task.status,
    });
}

async fn sleep_until(deadline: chrono::DateTime<Utc>) {
    let now = Utc::now();
    if let Ok(duration) = (deadline - now).to_std() {
        if !duration.is_zero() {
            tokio::time::sleep(duration).await;
        }
    }
}

async fn current_page_url(browser: &BrowserHub, page: &BrowserPage, fallback: &Url) -> Option<Url> {
    match browser.current_url(page).await {
        Ok(Some(value)) => Some(value),
        _ => Some(fallback.clone()),
    }
}
