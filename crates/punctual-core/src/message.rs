use url::Url;
use uuid::Uuid;

use crate::{ClickTask, ExecutionLog, TargetCandidate, TargetFingerprint, TaskStatus};

/// Commands sent by the GPUI frontend to the background engine.
///
/// `request_id` correlates an asynchronous response with the editor operation
/// that initiated it. Button discovery, manual validation and highlighting use
/// the same request id so the engine can retain the corresponding browser page.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    DetectTargets {
        request_id: Uuid,
        url: Url,
    },
    ValidateManualTarget {
        request_id: Uuid,
        text: String,
    },
    HighlightTarget {
        request_id: Uuid,
        target: TargetFingerprint,
    },
    SaveTask {
        request_id: Uuid,
        task: ClickTask,
    },
    DeleteTask {
        request_id: Uuid,
        task_id: Uuid,
    },
    LoadExecutionLogs {
        request_id: Uuid,
        task_id: Uuid,
    },
    Shutdown,
}

impl EngineCommand {
    pub const fn request_id(&self) -> Option<Uuid> {
        match self {
            Self::DetectTargets { request_id, .. }
            | Self::ValidateManualTarget { request_id, .. }
            | Self::HighlightTarget { request_id, .. }
            | Self::SaveTask { request_id, .. }
            | Self::DeleteTask { request_id, .. }
            | Self::LoadExecutionLogs { request_id, .. } => Some(*request_id),
            Self::Shutdown => None,
        }
    }

    pub const fn task_id(&self) -> Option<Uuid> {
        match self {
            Self::SaveTask { task, .. } => Some(task.id),
            Self::DeleteTask { task_id, .. } | Self::LoadExecutionLogs { task_id, .. } => {
                Some(*task_id)
            }
            _ => None,
        }
    }

    pub const fn operation(&self) -> &'static str {
        match self {
            Self::DetectTargets { .. } => "detect_targets",
            Self::ValidateManualTarget { .. } => "validate_manual_target",
            Self::HighlightTarget { .. } => "highlight_target",
            Self::SaveTask { .. } => "save_task",
            Self::DeleteTask { .. } => "delete_task",
            Self::LoadExecutionLogs { .. } => "load_execution_logs",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManualTargetValidation {
    Unique(TargetCandidate),
    Multiple(Vec<TargetCandidate>),
    NotClickable(Vec<TargetCandidate>),
    NotFound,
}

/// Events emitted by the engine. They are delivered through a non-blocking
/// channel and applied on GPUI's foreground executor.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    BrowserStateChanged {
        connected: bool,
        browser_name: Option<String>,
        message: String,
    },
    TargetsDetected {
        request_id: Uuid,
        url: Url,
        candidates: Vec<TargetCandidate>,
    },
    ManualTargetValidated {
        request_id: Uuid,
        text: String,
        validation: ManualTargetValidation,
    },
    TargetHighlighted {
        request_id: Uuid,
        found: bool,
    },
    TaskSaved {
        request_id: Uuid,
        task: ClickTask,
    },
    TaskDeleted {
        request_id: Uuid,
        task_id: Uuid,
    },
    TaskStatusChanged {
        task_id: Uuid,
        status: TaskStatus,
    },
    TaskCompleted {
        task: ClickTask,
        log: ExecutionLog,
    },
    ExecutionLogsLoaded {
        request_id: Uuid,
        task_id: Uuid,
        logs: Vec<ExecutionLog>,
    },
    CommandFailed {
        request_id: Option<Uuid>,
        task_id: Option<Uuid>,
        operation: String,
        message: String,
    },
}
