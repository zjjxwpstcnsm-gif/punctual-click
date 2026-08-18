use thiserror::Error;

use crate::TaskStatus;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("task title cannot be empty")]
    EmptyTitle,
    #[error("only http and https URLs are supported")]
    UnsupportedUrlScheme,
    #[error("manual button text cannot be empty")]
    EmptyManualText,
    #[error("a task cannot be scheduled until its target button has been verified")]
    TargetNotVerified,
    #[error("unknown IANA timezone: {0}")]
    InvalidTimezone(String),
    #[error("invalid task state transition: {from:?} -> {to:?}")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },
    #[error("terminal execution status {0:?} requires an ExecutionResult")]
    ExecutionResultRequired(TaskStatus),
    #[error("an execution result cannot be attached while task status is {status:?}")]
    ResultStatusMismatch { status: TaskStatus },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("unknown IANA timezone: {0}")]
    UnknownTimezone(String),
    #[error("invalid calendar date")]
    InvalidDate,
    #[error("invalid clock time")]
    InvalidTime,
    #[error("millisecond must be between 0 and 999")]
    InvalidMillisecond,
    #[error("local time does not exist in timezone {0}, usually because of a DST transition")]
    NonexistentLocalTime(String),
    #[error("local time is ambiguous in timezone {0}, usually because of a DST transition")]
    AmbiguousLocalTime(String),
}
