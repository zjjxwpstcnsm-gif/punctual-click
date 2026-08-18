use std::{
    path::PathBuf,
    sync::{Arc, mpsc as std_mpsc},
    thread::{self, JoinHandle},
};

use anyhow::{Context as _, Result, anyhow};
use crossbeam_channel::{Receiver, Sender, unbounded};
use punctual_core::{
    EngineCommand, EngineEvent, ExecutionPlanConfig, PreciseTimerConfig,
};
use punctual_storage::SqliteTaskRepository;
use tokio::{runtime::Builder, sync::mpsc};

use crate::engine::Engine;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub profile_dir: PathBuf,
    pub browser_executable: Option<PathBuf>,
    pub browser_preference: Option<String>,
    pub resources_dir: Option<PathBuf>,
    pub scheduler_tick_ms: u64,
    pub page_settle_ms: u64,
    pub completion_timeout_ms: u64,
    pub completion_poll_ms: u64,
    pub click_probe_interval_ms: u64,
    pub execution_plan: ExecutionPlanConfig,
    pub precise_timer: PreciseTimerConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            profile_dir: PathBuf::from(".punctual/browser-profile"),
            browser_executable: std::env::var_os("PUNCTUAL_CHROMIUM").map(PathBuf::from),
            browser_preference: std::env::var("PUNCTUAL_BROWSER").ok(),
            resources_dir: None,
            scheduler_tick_ms: 250,
            page_settle_ms: 800,
            completion_timeout_ms: 15_000,
            completion_poll_ms: 100,
            click_probe_interval_ms: 8,
            execution_plan: ExecutionPlanConfig::default(),
            precise_timer: PreciseTimerConfig::default(),
        }
    }
}

/// Owns the dedicated engine thread and the two typed communication channels.
///
/// The event receiver is cloneable and can be polled by GPUI without blocking
/// its render thread. Dropping this handle requests a graceful browser shutdown
/// and joins the runtime thread.
pub struct EngineHandle {
    commands: mpsc::UnboundedSender<EngineCommand>,
    events: Receiver<EngineEvent>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn start(
        repository: Arc<SqliteTaskRepository>,
        config: EngineConfig,
    ) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx): (Sender<EngineEvent>, Receiver<EngineEvent>) = unbounded();
        let (startup_tx, startup_rx) =
            std_mpsc::sync_channel::<std::result::Result<(), String>>(1);

        let thread = thread::Builder::new()
            .name("punctual-engine".into())
            .spawn(move || {
                let runtime = match Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .thread_name("punctual-runtime")
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error.to_string()));
                        return;
                    }
                };

                let _ = startup_tx.send(Ok(()));
                runtime.block_on(async move {
                    Engine::new(repository, config, command_rx, event_tx)
                        .run()
                        .await;
                });
            })
            .context("failed to start the Punctual engine thread")?;

        match startup_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands: command_tx,
                events: event_rx,
                thread: Some(thread),
            }),
            Ok(Err(message)) => {
                let _ = thread.join();
                Err(anyhow!(message)).context("failed to initialize the Tokio runtime")
            }
            Err(error) => {
                let _ = thread.join();
                Err(anyhow!(error)).context("engine thread stopped before initialization")
            }
        }
    }

    pub fn send(&self, command: EngineCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| anyhow!("Punctual background engine is no longer running"))
    }

    pub fn events(&self) -> Receiver<EngineEvent> {
        self.events.clone()
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(EngineCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
