use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{truncate_to_millis, utc_now_millis, ExecutionOutcome, ExecutionResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLog {
    pub id: Uuid,
    pub task_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub observed_click_at: Option<DateTime<Utc>>,
    pub dispatch_delay_ms: Option<i64>,
    pub observed_delay_ms: Option<i64>,
    pub outcome: ExecutionOutcome,
    pub final_url: Option<Url>,
    pub message: String,
    pub error_code: Option<String>,
    pub screenshot_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ExecutionLog {
    pub fn from_result(task_id: Uuid, result: &ExecutionResult) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            scheduled_at: truncate_to_millis(result.scheduled_at),
            dispatched_at: result.dispatched_at.map(truncate_to_millis),
            observed_click_at: result.observed_click_at.map(truncate_to_millis),
            dispatch_delay_ms: result.dispatch_delay_ms,
            observed_delay_ms: result.observed_delay_ms,
            outcome: result.outcome,
            final_url: result.final_url.clone(),
            message: result.message.clone(),
            error_code: result.error_code.clone(),
            screenshot_path: result.screenshot_path.clone(),
            created_at: utc_now_millis(),
        }
    }
}
