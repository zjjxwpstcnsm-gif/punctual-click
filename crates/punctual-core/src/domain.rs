use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{truncate_to_millis, utc_now_millis, DomainError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOverviewStatus {
    Pending,
    Executed,
}

impl TaskOverviewStatus {
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::Pending => "待执行",
            Self::Executed => "已执行",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft,
    Pending,
    Preparing,
    Armed,
    Executing,
    Succeeded,
    Failed,
    Uncertain,
    Missed,
    Cancelled,
}

impl TaskStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Uncertain | Self::Missed | Self::Cancelled
        )
    }

    pub const fn overview(self) -> TaskOverviewStatus {
        if self.is_terminal() {
            TaskOverviewStatus::Executed
        } else {
            TaskOverviewStatus::Pending
        }
    }

    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::Draft => "草稿",
            Self::Pending => "等待执行",
            Self::Preparing => "正在准备",
            Self::Armed => "已布防",
            Self::Executing => "正在点击",
            Self::Succeeded => "成功",
            Self::Failed => "失败",
            Self::Uncertain => "已点击但未确认",
            Self::Missed => "已错过",
            Self::Cancelled => "已取消",
        }
    }

    pub const fn can_transition_to(self, next: Self) -> bool {
        use TaskStatus::*;
        matches!(
            (self, next),
            (Draft, Pending)
                | (Draft, Cancelled)
                | (Pending, Preparing)
                | (Pending, Missed)
                | (Pending, Failed)
                | (Pending, Cancelled)
                | (Preparing, Armed)
                | (Preparing, Failed)
                | (Preparing, Cancelled)
                | (Armed, Executing)
                | (Armed, Failed)
                | (Armed, Cancelled)
                | (Executing, Succeeded)
                | (Executing, Failed)
                | (Executing, Uncertain)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ClickMode {
    Strict,
    WaitUntilClickable { grace_period_ms: u64 },
}

impl Default for ClickMode {
    fn default() -> Self {
        Self::WaitUntilClickable {
            grace_period_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TargetRule {
    Auto {
        selected: Option<TargetFingerprint>,
    },
    ManualText {
        text: String,
        verified_target: TargetFingerprint,
    },
}

impl TargetRule {
    pub fn display_text(&self) -> &str {
        match self {
            Self::Auto { selected } => selected
                .as_ref()
                .map(TargetFingerprint::best_name)
                .unwrap_or("自动推理"),
            Self::ManualText { text, .. } => text,
        }
    }

    pub fn fingerprint(&self) -> Option<&TargetFingerprint> {
        match self {
            Self::Auto { selected } => selected.as_ref(),
            Self::ManualText {
                verified_target, ..
            } => Some(verified_target),
        }
    }

    pub fn is_verified(&self) -> bool {
        self.fingerprint().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetFingerprint {
    pub role: String,
    pub accessible_name: String,
    pub visible_text: String,
    pub stable_attributes: BTreeMap<String, String>,
    pub context_text: Option<String>,
    pub selector_hint: Option<String>,
    #[serde(default)]
    pub shadow_path: Vec<String>,
    #[serde(default)]
    pub frame_path: Vec<String>,
}

impl TargetFingerprint {
    pub fn best_name(&self) -> &str {
        if !self.accessible_name.trim().is_empty() {
            self.accessible_name.as_str()
        } else if !self.visible_text.trim().is_empty() {
            self.visible_text.as_str()
        } else {
            "未命名按钮"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetCandidate {
    pub candidate_id: String,
    pub tag_name: String,
    pub role: String,
    pub input_type: Option<String>,
    pub accessible_name: String,
    pub visible_text: String,
    pub context_text: Option<String>,
    pub selector_hint: Option<String>,
    #[serde(default)]
    pub shadow_path: Vec<String>,
    pub stable_attributes: BTreeMap<String, String>,
    pub rect: ElementRect,
    pub visible: bool,
    pub enabled: bool,
    pub pointer_events: bool,
    pub covered: bool,
    pub semantic_clickable: bool,
    #[serde(default)]
    pub score: i32,
    #[serde(default)]
    pub confidence: u8,
    #[serde(default)]
    pub score_reasons: Vec<String>,
}

impl TargetCandidate {
    pub fn is_clickable_now(&self) -> bool {
        self.visible
            && self.enabled
            && self.pointer_events
            && !self.covered
            && self.semantic_clickable
            && self.rect.width > 0.0
            && self.rect.height > 0.0
    }

    pub fn to_fingerprint(&self) -> TargetFingerprint {
        TargetFingerprint {
            role: self.role.clone(),
            accessible_name: self.accessible_name.clone(),
            visible_text: self.visible_text.clone(),
            stable_attributes: self.stable_attributes.clone(),
            context_text: self.context_text.clone(),
            selector_hint: self.selector_hint.clone(),
            shadow_path: self.shadow_path.clone(),
            frame_path: Vec::new(),
        }
    }

    pub fn best_name(&self) -> &str {
        if !self.accessible_name.trim().is_empty() {
            &self.accessible_name
        } else if !self.visible_text.trim().is_empty() {
            &self.visible_text
        } else {
            "未命名按钮"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionSignal {
    UrlChanged,
    UrlMatches { pattern: String },
    TextAppears { text: String },
    SelectorAppears { selector: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Succeeded,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub outcome: ExecutionOutcome,
    pub scheduled_at: DateTime<Utc>,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub observed_click_at: Option<DateTime<Utc>>,
    pub dispatch_delay_ms: Option<i64>,
    pub observed_delay_ms: Option<i64>,
    pub final_url: Option<Url>,
    pub message: String,
    pub error_code: Option<String>,
    pub screenshot_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClickTask {
    pub id: Uuid,
    pub title: String,
    pub url: Url,
    pub scheduled_at_utc: DateTime<Utc>,
    pub timezone: String,
    pub click_mode: ClickMode,
    pub target: TargetRule,
    pub completion_signals: Vec<CompletionSignal>,
    pub status: TaskStatus,
    pub result: Option<ExecutionResult>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ClickTask {
    pub fn new(
        title: impl Into<String>,
        url: Url,
        scheduled_at_utc: DateTime<Utc>,
        timezone: impl Into<String>,
        target: TargetRule,
    ) -> Result<Self, DomainError> {
        let title = title.into().trim().to_owned();
        if title.is_empty() {
            return Err(DomainError::EmptyTitle);
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err(DomainError::UnsupportedUrlScheme);
        }
        let timezone = timezone.into();
        if timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(DomainError::InvalidTimezone(timezone));
        }
        if let TargetRule::ManualText { text, .. } = &target {
            if text.trim().is_empty() {
                return Err(DomainError::EmptyManualText);
            }
        }

        let now = utc_now_millis();
        let scheduled_at_utc = truncate_to_millis(scheduled_at_utc);
        Ok(Self {
            id: Uuid::new_v4(),
            title,
            url,
            scheduled_at_utc,
            timezone,
            click_mode: ClickMode::default(),
            target,
            completion_signals: vec![CompletionSignal::UrlChanged],
            status: TaskStatus::Draft,
            result: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn transition(&mut self, next: TaskStatus) -> Result<(), DomainError> {
        if matches!(
            next,
            TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Uncertain
        ) {
            return Err(DomainError::ExecutionResultRequired(next));
        }
        if next == TaskStatus::Pending && !self.target.is_verified() {
            return Err(DomainError::TargetNotVerified);
        }
        self.apply_transition(next)
    }

    fn apply_transition(&mut self, next: TaskStatus) -> Result<(), DomainError> {
        if !self.status.can_transition_to(next) {
            return Err(DomainError::InvalidTransition {
                from: self.status,
                to: next,
            });
        }
        self.status = next;
        self.updated_at = utc_now_millis();
        Ok(())
    }

    /// Attaches a terminal execution result and moves the task to the matching
    /// state.
    ///
    /// A verified success or an uncertain post-click result can only follow
    /// `Executing`. A failure may also be attached while the task is preparing
    /// or armed, for example when Chromium disconnects or the target disappears
    /// before the click deadline.
    pub fn finish(&mut self, result: ExecutionResult) -> Result<(), DomainError> {
        let target_status = match result.outcome {
            ExecutionOutcome::Succeeded => TaskStatus::Succeeded,
            ExecutionOutcome::Failed => TaskStatus::Failed,
            ExecutionOutcome::Uncertain => TaskStatus::Uncertain,
        };

        if !self.status.can_transition_to(target_status) {
            return Err(DomainError::ResultStatusMismatch {
                status: self.status,
            });
        }

        self.apply_transition(target_status)?;
        self.result = Some(result);
        Ok(())
    }

    /// Backwards-compatible name for finishing a task after click dispatch.
    pub fn complete(&mut self, result: ExecutionResult) -> Result<(), DomainError> {
        self.finish(result)
    }

    /// Restores a task that was preparing or armed when the desktop process
    /// stopped. The scheduler will repeat page preparation and target
    /// resolution, but will still preserve the original one-click guarantee.
    pub fn recover_to_pending(&mut self) -> bool {
        if matches!(self.status, TaskStatus::Preparing | TaskStatus::Armed) {
            self.status = TaskStatus::Pending;
            self.result = None;
            self.updated_at = utc_now_millis();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn selected_target() -> TargetFingerprint {
        TargetFingerprint {
            role: "button".into(),
            accessible_name: "提交订单".into(),
            visible_text: "提交订单".into(),
            stable_attributes: BTreeMap::new(),
            context_text: None,
            selector_hint: Some("#submit".into()),
            shadow_path: Vec::new(),
            frame_path: Vec::new(),
        }
    }

    fn task() -> ClickTask {
        ClickTask::new(
            "提交订单",
            Url::parse("https://example.com/checkout").unwrap(),
            Utc::now() + Duration::minutes(1),
            "Asia/Tokyo",
            TargetRule::Auto {
                selected: Some(selected_target()),
            },
        )
        .unwrap()
    }

    #[test]
    fn enforces_state_machine() {
        let mut task = task();
        assert!(task.transition(TaskStatus::Pending).is_ok());
        assert!(task.transition(TaskStatus::Preparing).is_ok());
        assert!(task.transition(TaskStatus::Armed).is_ok());
        assert!(task.transition(TaskStatus::Executing).is_ok());
        assert_eq!(
            task.transition(TaskStatus::Succeeded).unwrap_err(),
            DomainError::ExecutionResultRequired(TaskStatus::Succeeded)
        );
    }

    #[test]
    fn attaches_failure_details_before_click_dispatch() {
        let mut task = task();
        task.transition(TaskStatus::Pending).unwrap();
        task.transition(TaskStatus::Preparing).unwrap();

        let result = ExecutionResult {
            outcome: ExecutionOutcome::Failed,
            scheduled_at: task.scheduled_at_utc,
            dispatched_at: None,
            observed_click_at: None,
            dispatch_delay_ms: None,
            observed_delay_ms: None,
            final_url: Some(task.url.clone()),
            message: "浏览器连接失败".into(),
            error_code: Some("browser_disconnected".into()),
            screenshot_path: None,
        };

        task.finish(result.clone()).unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.result, Some(result));
    }

    #[test]
    fn maps_internal_states_to_two_level_overview() {
        assert_eq!(TaskStatus::Armed.overview(), TaskOverviewStatus::Pending);
        assert_eq!(
            TaskStatus::Succeeded.overview(),
            TaskOverviewStatus::Executed
        );
        assert_eq!(TaskStatus::Failed.overview(), TaskOverviewStatus::Executed);
    }

    #[test]
    fn rejects_unknown_timezone() {
        let result = ClickTask::new(
            "提交订单",
            Url::parse("https://example.com").unwrap(),
            Utc::now(),
            "Mars/Olympus_Mons",
            TargetRule::Auto { selected: None },
        );
        assert_eq!(
            result.unwrap_err(),
            DomainError::InvalidTimezone("Mars/Olympus_Mons".into())
        );
    }

    #[test]
    fn rejects_non_http_urls() {
        let result = ClickTask::new(
            "本地文件",
            Url::parse("file:///tmp/demo.html").unwrap(),
            Utc::now(),
            "UTC",
            TargetRule::Auto { selected: None },
        );
        assert_eq!(result.unwrap_err(), DomainError::UnsupportedUrlScheme);
    }
    #[test]
    fn refuses_to_schedule_an_unverified_auto_target() {
        let mut task = ClickTask::new(
            "自动识别",
            Url::parse("https://example.com").unwrap(),
            Utc::now() + Duration::minutes(1),
            "UTC",
            TargetRule::Auto { selected: None },
        )
        .unwrap();

        assert_eq!(
            task.transition(TaskStatus::Pending).unwrap_err(),
            DomainError::TargetNotVerified
        );
    }
}
