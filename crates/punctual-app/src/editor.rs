use chrono::{Datelike, NaiveDateTime, Timelike, Utc};
use gpui::{App, AppContext as _, Context, Entity, Window};
use gpui_component::input::InputState;
use punctual_core::{
    format_in_timezone, ClickMode, ClickTask, CompletionSignal, LocalScheduleInput,
    TargetCandidate, TargetFingerprint, TargetRule, TaskStatus,
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorClickMode {
    Strict,
    Wait,
}

pub struct TaskEditor {
    pub inspection_id: Uuid,
    pub editing_task: Option<ClickTask>,
    pub title: Entity<InputState>,
    pub url: Entity<InputState>,
    pub local_datetime: Entity<InputState>,
    pub timezone: Entity<InputState>,
    pub manual_text: Entity<InputState>,
    pub success_text: Entity<InputState>,
    pub grace_period_ms: Entity<InputState>,
    pub target_mode: TargetMode,
    pub click_mode: EditorClickMode,
    pub candidates: Vec<TargetCandidate>,
    pub selected_candidate_id: Option<String>,
    pub selected_target: Option<TargetFingerprint>,
    /// URL whose DOM produced `selected_target`.
    ///
    /// A target detected on one page must never be silently reused after the
    /// user edits the URL field.
    pub inspected_url: Option<Url>,
    /// Exact manual label that most recently passed page validation.
    pub validated_manual_text: Option<String>,
    pub busy: bool,
    pub message: String,
}

impl TaskEditor {
    pub fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Self {
        let (datetime, timezone) = default_schedule();
        Self {
            inspection_id: Uuid::new_v4(),
            editing_task: None,
            title: new_input(window, cx, "例如：提交订单", ""),
            url: new_input(window, cx, "https://example.com/checkout", ""),
            local_datetime: new_input(window, cx, "YYYY-MM-DD HH:MM:SS.mmm", &datetime),
            timezone: new_input(window, cx, "Asia/Shanghai", &timezone),
            manual_text: new_input(window, cx, "例如：提交订单", ""),
            success_text: new_input(window, cx, "可选，例如：订单提交成功", ""),
            grace_period_ms: new_input(window, cx, "3000", "3000"),
            target_mode: TargetMode::Auto,
            click_mode: EditorClickMode::Wait,
            candidates: Vec::new(),
            selected_candidate_id: None,
            selected_target: None,
            inspected_url: None,
            validated_manual_text: None,
            busy: false,
            message: "填写 URL 后检测页面按钮".into(),
        }
    }

    pub fn reset<T: 'static>(&mut self, window: &mut Window, cx: &mut Context<T>) {
        let replacement = Self::new(window, cx);
        *self = replacement;
    }

    pub fn load<T: 'static>(&mut self, task: ClickTask, window: &mut Window, cx: &mut Context<T>) {
        self.inspection_id = Uuid::new_v4();
        self.editing_task = Some(task.clone());
        self.candidates.clear();
        self.selected_candidate_id = None;
        self.selected_target = task.target.fingerprint().cloned();
        self.inspected_url = Some(task.url.clone());
        self.validated_manual_text = None;
        self.busy = false;
        self.message = "已载入任务；重新检测页面可确认按钮仍然有效".into();

        set_input(&self.title, task.title.clone(), window, cx);
        set_input(&self.url, task.url.to_string(), window, cx);
        let local = format_in_timezone(task.scheduled_at_utc, &task.timezone)
            .map(|value| {
                value
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|_| {
                task.scheduled_at_utc
                    .format("%Y-%m-%d %H:%M:%S%.3f")
                    .to_string()
            });
        set_input(&self.local_datetime, local, window, cx);
        set_input(&self.timezone, task.timezone.clone(), window, cx);

        match &task.target {
            TargetRule::Auto { .. } => {
                self.target_mode = TargetMode::Auto;
                set_input(&self.manual_text, "", window, cx);
            }
            TargetRule::ManualText { text, .. } => {
                self.target_mode = TargetMode::Manual;
                self.validated_manual_text = Some(text.trim().to_owned());
                set_input(&self.manual_text, text.clone(), window, cx);
            }
        }

        match task.click_mode {
            ClickMode::Strict => {
                self.click_mode = EditorClickMode::Strict;
                set_input(&self.grace_period_ms, "0", window, cx);
            }
            ClickMode::WaitUntilClickable { grace_period_ms } => {
                self.click_mode = EditorClickMode::Wait;
                set_input(
                    &self.grace_period_ms,
                    grace_period_ms.to_string(),
                    window,
                    cx,
                );
            }
        }

        let success_text = task
            .completion_signals
            .iter()
            .find_map(|signal| match signal {
                CompletionSignal::TextAppears { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        set_input(&self.success_text, success_text, window, cx);
    }

    pub fn url_value(&self, cx: &App) -> Result<Url, String> {
        let raw = input_value(&self.url, cx);
        let url = Url::parse(raw.trim()).map_err(|error| format!("URL 无效：{error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("只支持 http 或 https URL".into());
        }
        Ok(url)
    }

    pub fn manual_text_value(&self, cx: &App) -> String {
        input_value(&self.manual_text, cx).trim().to_owned()
    }

    pub fn choose_candidate(&mut self, candidate: &TargetCandidate) {
        self.selected_candidate_id = Some(candidate.candidate_id.clone());
        self.selected_target = Some(candidate.to_fingerprint());
        self.message = format!(
            "已选择“{}”，置信度 {}%",
            candidate.best_name(),
            candidate.confidence
        );
    }

    pub fn accept_detected_candidates(&mut self, url: Url, candidates: Vec<TargetCandidate>) {
        self.inspected_url = Some(url);
        self.validated_manual_text = None;
        self.candidates = candidates;
        self.busy = false;

        if self.candidates.is_empty() {
            self.selected_candidate_id = None;
            self.selected_target = None;
            self.message = "页面中没有发现可交互按钮".into();
            return;
        }

        if self.target_mode == TargetMode::Auto && self.selected_target.is_none() {
            let clickable = self
                .candidates
                .iter()
                .filter(|candidate| candidate.is_clickable_now())
                .collect::<Vec<_>>();
            let clear_winner = clickable.first().copied().filter(|first| {
                first.confidence >= 70
                    && clickable
                        .get(1)
                        .map(|second| first.score - second.score >= 12)
                        .unwrap_or(true)
            });
            if let Some(candidate) = clear_winner {
                self.selected_candidate_id = Some(candidate.candidate_id.clone());
                self.selected_target = Some(candidate.to_fingerprint());
                self.message = format!(
                    "已自动推理目标“{}”；仍可从候选中改选",
                    candidate.best_name()
                );
                return;
            }
        }

        self.message = format!("发现 {} 个候选；请选择目标按钮", self.candidates.len());
    }

    pub fn build_task(&self, cx: &App) -> Result<ClickTask, String> {
        let title = input_value(&self.title, cx);
        let url = self.url_value(cx)?;
        if self.inspected_url.as_ref() != Some(&url) {
            return Err("页面 URL 已改变，请重新检测按钮，避免点击到其他页面的旧目标".into());
        }
        let timezone = input_value(&self.timezone, cx).trim().to_owned();
        let raw_datetime = input_value(&self.local_datetime, cx);
        let naive = NaiveDateTime::parse_from_str(raw_datetime.trim(), "%Y-%m-%d %H:%M:%S%.3f")
            .map_err(|_| "执行时间格式应为 YYYY-MM-DD HH:MM:SS.mmm".to_owned())?;
        let scheduled_at_utc = LocalScheduleInput {
            year: naive.year(),
            month: naive.month(),
            day: naive.day(),
            hour: naive.hour(),
            minute: naive.minute(),
            second: naive.second(),
            millisecond: naive.and_utc().timestamp_subsec_millis(),
            timezone: timezone.clone(),
        }
        .to_utc()
        .map_err(|error| format!("执行时间无效：{error}"))?;
        if scheduled_at_utc <= Utc::now() {
            return Err("执行时间必须晚于当前时间".into());
        }

        let selected = self
            .selected_target
            .clone()
            .ok_or_else(|| "请先检测并选择一个真实可点击按钮".to_owned())?;
        if selected.selector_hint.is_none() {
            return Err("所选按钮缺少可执行定位信息，请重新检测".into());
        }
        let target = match self.target_mode {
            TargetMode::Auto => TargetRule::Auto {
                selected: Some(selected),
            },
            TargetMode::Manual => {
                let text = self.manual_text_value(cx);
                if text.is_empty() {
                    return Err("手动模式必须填写按钮文案".into());
                }
                let validated = self
                    .validated_manual_text
                    .as_deref()
                    .ok_or_else(|| "请先验证手动填写的按钮文案".to_owned())?;
                if validated != text {
                    return Err("按钮文案已改变，请重新验证后再保存".into());
                }
                TargetRule::ManualText {
                    text,
                    verified_target: selected,
                }
            }
        };

        let mut task = ClickTask::new(title, url, scheduled_at_utc, timezone, target)
            .map_err(|error| format!("任务无效：{error}"))?;
        if let Some(existing) = &self.editing_task {
            task.id = existing.id;
            task.created_at = existing.created_at;
        }

        task.click_mode = match self.click_mode {
            EditorClickMode::Strict => ClickMode::Strict,
            EditorClickMode::Wait => {
                let value = input_value(&self.grace_period_ms, cx)
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "宽限期必须是 0 到 60000 的整数毫秒".to_owned())?;
                if value > 60_000 {
                    return Err("宽限期不能超过 60000 毫秒".into());
                }
                ClickMode::WaitUntilClickable {
                    grace_period_ms: value,
                }
            }
        };

        task.completion_signals = vec![CompletionSignal::UrlChanged];
        let success_text = input_value(&self.success_text, cx).trim().to_owned();
        if !success_text.is_empty() {
            task.completion_signals
                .push(CompletionSignal::TextAppears { text: success_text });
        }
        task.transition(TaskStatus::Pending)
            .map_err(|error| format!("无法安排任务：{error}"))?;
        Ok(task)
    }
}

fn new_input<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    placeholder: &str,
    value: &str,
) -> Entity<InputState> {
    let placeholder = placeholder.to_owned();
    let value = value.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

fn set_input<T: 'static>(
    input: &Entity<InputState>,
    value: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let value = value.into();
    input.update(cx, |state, cx| state.set_value(value, window, cx));
}

pub fn input_value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn default_schedule() -> (String, String) {
    let timezone = "Asia/Shanghai".to_owned();
    let utc = Utc::now() + chrono::Duration::minutes(5);
    let local = LocalScheduleInput::from_utc(utc, &timezone).expect("static timezone is valid");
    (
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
            local.millisecond
        ),
        timezone,
    )
}
