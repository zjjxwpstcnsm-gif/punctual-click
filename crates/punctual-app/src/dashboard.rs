use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use gpui::{
    div, prelude::*, px, rgb, ClipboardItem, Context, Div, Entity, Render, SharedString, Stateful,
    Task, Window,
};
use gpui_component::{
    input::{Input, InputState},
    scroll::ScrollableElement as _,
};
use punctual_core::{
    format_in_timezone, ClickTask, EngineCommand, EngineEvent, ExecutionLog, ExecutionOutcome,
    ManualTargetValidation, TargetCandidate, TargetFingerprint, TaskStatus,
};
use punctual_engine::EngineHandle;
use punctual_storage::SqliteTaskRepository;
use uuid::Uuid;

use crate::editor::{EditorClickMode, TargetMode, TaskEditor};

const BG: u32 = 0xf4f6f8;
const PANEL: u32 = 0xffffff;
const BORDER: u32 = 0xe3e7ee;
const TEXT: u32 = 0x172033;
const MUTED: u32 = 0x667085;
const PRIMARY: u32 = 0x2563eb;
const PRIMARY_SOFT: u32 = 0xeaf1ff;
const SUCCESS: u32 = 0x15803d;
const SUCCESS_SOFT: u32 = 0xecfdf3;
const WARNING: u32 = 0xb54708;
const WARNING_SOFT: u32 = 0xfffaeb;
const DANGER: u32 = 0xb42318;
const DANGER_SOFT: u32 = 0xfef3f2;

pub struct PunctualDashboard {
    repository: Arc<SqliteTaskRepository>,
    engine: EngineHandle,
    tasks: Vec<ClickTask>,
    selected: Option<Uuid>,
    logs: Vec<ExecutionLog>,
    logs_task_id: Option<Uuid>,
    notice: SharedString,
    browser_connected: bool,
    browser_name: Option<SharedString>,
    editor_open: bool,
    editor: TaskEditor,
    _event_task: Task<()>,
}

impl PunctualDashboard {
    pub fn new(
        repository: Arc<SqliteTaskRepository>,
        engine: EngineHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tasks = repository.list().unwrap_or_default();
        let selected = tasks.first().map(|task| task.id);
        let events = engine.events();
        let event_task = cx.spawn(async move |this, cx| loop {
            gpui::Timer::after(Duration::from_millis(75)).await;
            let batch = events.try_iter().collect::<Vec<_>>();
            if batch.is_empty() {
                continue;
            }
            if this
                .update(cx, |this, cx| {
                    this.apply_events(batch);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        });

        let mut dashboard = Self {
            repository,
            engine,
            tasks,
            selected,
            logs: Vec::new(),
            logs_task_id: None,
            notice: "本地数据库与后台调度器已启动".into(),
            browser_connected: false,
            browser_name: None,
            editor_open: false,
            editor: TaskEditor::new(window, cx),
            _event_task: event_task,
        };
        if let Some(task_id) = dashboard.selected {
            dashboard.request_logs(task_id);
        }
        dashboard
    }

    fn apply_events(&mut self, events: Vec<EngineEvent>) {
        for event in events {
            match event {
                EngineEvent::BrowserStateChanged {
                    connected,
                    browser_name,
                    message,
                } => {
                    self.browser_connected = connected;
                    self.browser_name = browser_name.map(Into::into);
                    self.notice = message.into();
                }
                EngineEvent::TargetsDetected {
                    request_id,
                    url,
                    candidates,
                } if request_id == self.editor.inspection_id => {
                    self.editor.accept_detected_candidates(url, candidates);
                    self.notice = self.editor.message.clone().into();
                }
                EngineEvent::ManualTargetValidated {
                    request_id,
                    text,
                    validation,
                } if request_id == self.editor.inspection_id => {
                    self.editor.busy = false;
                    match validation {
                        ManualTargetValidation::Unique(candidate) => {
                            self.editor.validated_manual_text = Some(text.trim().to_owned());
                            self.editor.candidates = vec![candidate.clone()];
                            self.editor.choose_candidate(&candidate);
                            self.editor.message =
                                format!("验证通过：“{text}”唯一对应当前可点击按钮");
                        }
                        ManualTargetValidation::Multiple(candidates) => {
                            self.editor.validated_manual_text = Some(text.trim().to_owned());
                            self.editor.candidates = candidates;
                            self.editor.selected_candidate_id = None;
                            self.editor.selected_target = None;
                            self.editor.message =
                                format!("找到多个可点击的“{text}”，请结合页面上下文选择");
                        }
                        ManualTargetValidation::NotClickable(candidates) => {
                            self.editor.validated_manual_text = None;
                            self.editor.candidates = candidates;
                            self.editor.selected_candidate_id = None;
                            self.editor.selected_target = None;
                            self.editor.message =
                                format!("页面存在“{text}”，但当前没有匹配项能够接收点击");
                        }
                        ManualTargetValidation::NotFound => {
                            self.editor.validated_manual_text = None;
                            self.editor.candidates.clear();
                            self.editor.selected_candidate_id = None;
                            self.editor.selected_target = None;
                            self.editor.message = format!("没有找到文案精确为“{text}”的可点击元素");
                        }
                    }
                    self.notice = self.editor.message.clone().into();
                }
                EngineEvent::TargetHighlighted { request_id, found }
                    if request_id == self.editor.inspection_id =>
                {
                    self.notice = if found {
                        "已在浏览器中滚动并高亮目标按钮".into()
                    } else {
                        "页面已经变化，原按钮定位当前无效；请重新检测".into()
                    };
                }
                EngineEvent::TaskSaved { request_id, task } => {
                    self.editor.busy = false;
                    self.editor_open = false;
                    self.selected = Some(task.id);
                    self.notice = if request_id == self.editor.inspection_id {
                        "任务已保存并交给后台调度器".into()
                    } else {
                        "任务已保存".into()
                    };
                    self.reload_tasks();
                    self.request_logs(task.id);
                }
                EngineEvent::TaskDeleted { task_id, .. } => {
                    if self.selected == Some(task_id) {
                        self.selected = None;
                    }
                    self.logs.clear();
                    self.logs_task_id = None;
                    self.notice = "任务及其执行记录已删除".into();
                    self.reload_tasks();
                }
                EngineEvent::TaskStatusChanged { task_id, status } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) {
                        task.status = status;
                    } else {
                        self.reload_tasks();
                    }
                    self.notice = format!("任务状态：{}", status.label_zh()).into();
                }
                EngineEvent::TaskCompleted { task, log } => {
                    let task_id = task.id;
                    if let Some(existing) = self.tasks.iter_mut().find(|value| value.id == task_id)
                    {
                        *existing = task.clone();
                    } else {
                        self.tasks.push(task.clone());
                    }
                    if self.selected == Some(task_id) {
                        self.logs.insert(0, log);
                        self.logs_task_id = Some(task_id);
                    }
                    self.notice = match task.status {
                        TaskStatus::Succeeded => "任务执行成功".into(),
                        TaskStatus::Failed => "任务执行失败；详情已写入执行记录".into(),
                        TaskStatus::Uncertain => "点击已派发，但页面没有提供足够的成功证据".into(),
                        _ => format!("任务状态：{}", task.status.label_zh()).into(),
                    };
                }
                EngineEvent::ExecutionLogsLoaded { task_id, logs, .. }
                    if self.selected == Some(task_id) =>
                {
                    self.logs = logs;
                    self.logs_task_id = Some(task_id);
                }
                EngineEvent::CommandFailed {
                    request_id,
                    task_id,
                    operation,
                    message,
                } => {
                    if request_id == Some(self.editor.inspection_id) {
                        self.editor.busy = false;
                        self.editor.message = message.clone();
                    }
                    let scope = task_id
                        .map(|value| format!("任务 {value}"))
                        .unwrap_or_else(|| "操作".into());
                    self.notice = format!("{scope}失败（{operation}）：{message}").into();
                }
                _ => {}
            }
        }
    }

    fn reload_tasks(&mut self) {
        match self.repository.list() {
            Ok(tasks) => {
                self.tasks = tasks;
                if self
                    .selected
                    .is_some_and(|id| !self.tasks.iter().any(|task| task.id == id))
                {
                    self.selected = self.tasks.first().map(|task| task.id);
                }
            }
            Err(error) => self.notice = format!("读取任务失败：{error}").into(),
        }
    }

    fn request_logs(&mut self, task_id: Uuid) {
        let request_id = Uuid::new_v4();
        if let Err(error) = self.engine.send(EngineCommand::LoadExecutionLogs {
            request_id,
            task_id,
        }) {
            self.notice = format!("读取执行记录失败：{error:#}").into();
        }
    }

    fn select_task(&mut self, task_id: Uuid) {
        self.selected = Some(task_id);
        self.editor_open = false;
        self.logs.clear();
        self.logs_task_id = None;
        self.request_logs(task_id);
    }

    fn open_new_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.reset(window, cx);
        self.editor_open = true;
        self.notice = "创建点击任务：先检测并确认目标按钮".into();
    }

    fn open_edit_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self
            .selected
            .and_then(|id| self.tasks.iter().find(|task| task.id == id))
            .cloned()
        else {
            self.notice = "请先选择任务".into();
            return;
        };
        self.editor.load(task, window, cx);
        self.editor_open = true;
        self.notice = "编辑任务会重新安排该任务；旧执行记录仍会保留".into();
    }

    fn detect_targets(&mut self, cx: &gpui::App) {
        let url = match self.editor.url_value(cx) {
            Ok(url) => url,
            Err(message) => {
                self.editor.message = message.clone();
                self.notice = message.into();
                return;
            }
        };
        self.editor.busy = true;
        self.editor.candidates.clear();
        self.editor.selected_candidate_id = None;
        self.editor.selected_target = None;
        self.editor.inspected_url = None;
        self.editor.validated_manual_text = None;
        self.editor.message = "正在可见浏览器中打开页面并扫描按钮…".into();
        if let Err(error) = self.engine.send(EngineCommand::DetectTargets {
            request_id: self.editor.inspection_id,
            url,
        }) {
            self.editor.busy = false;
            self.notice = format!("无法启动按钮检测：{error:#}").into();
        }
    }

    fn validate_manual_target(&mut self, cx: &gpui::App) {
        let text = self.editor.manual_text_value(cx);
        if text.is_empty() {
            self.editor.message = "请填写需要验证的按钮文案".into();
            self.notice = self.editor.message.clone().into();
            return;
        }
        self.editor.busy = true;
        self.editor.selected_candidate_id = None;
        self.editor.selected_target = None;
        self.editor.validated_manual_text = None;
        self.editor.message = format!("正在验证“{text}”是否对应真实可点击按钮…");
        if let Err(error) = self.engine.send(EngineCommand::ValidateManualTarget {
            request_id: self.editor.inspection_id,
            text,
        }) {
            self.editor.busy = false;
            self.notice = format!("无法验证按钮：{error:#}").into();
        }
    }

    fn highlight_target(&mut self, target: TargetFingerprint) {
        if let Err(error) = self.engine.send(EngineCommand::HighlightTarget {
            request_id: self.editor.inspection_id,
            target,
        }) {
            self.notice = format!("无法高亮按钮：{error:#}").into();
        }
    }

    fn save_editor(&mut self, cx: &gpui::App) {
        let task = match self.editor.build_task(cx) {
            Ok(task) => task,
            Err(message) => {
                self.editor.message = message.clone();
                self.notice = message.into();
                return;
            }
        };
        self.editor.busy = true;
        self.editor.message = "正在保存任务并启动调度…".into();
        if let Err(error) = self.engine.send(EngineCommand::SaveTask {
            request_id: self.editor.inspection_id,
            task,
        }) {
            self.editor.busy = false;
            self.notice = format!("保存任务失败：{error:#}").into();
        }
    }

    fn delete_selected(&mut self) {
        let Some(task_id) = self.selected else {
            self.notice = "请先选择任务".into();
            return;
        };
        let request_id = Uuid::new_v4();
        if let Err(error) = self.engine.send(EngineCommand::DeleteTask {
            request_id,
            task_id,
        }) {
            self.notice = format!("删除任务失败：{error:#}").into();
        } else {
            self.notice = "正在停止调度并删除任务…".into();
        }
    }

    fn render_task_card(task: ClickTask, selected: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = task.id;
        let (status_color, status_bg) = status_colors(task.status);
        let scheduled = format_in_timezone(task.scheduled_at_utc, &task.timezone)
            .unwrap_or_else(|_| task.scheduled_at_utc.to_rfc3339());

        div()
            .id(("task", element_key(&id)))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected { rgb(PRIMARY) } else { rgb(BORDER) })
            .bg(if selected {
                rgb(PRIMARY_SOFT)
            } else {
                rgb(PANEL)
            })
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(PRIMARY)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_task(id);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_full()
                            .bg(rgb(status_bg))
                            .text_color(rgb(status_color))
                            .text_xs()
                            .child(format!(
                                "{} · {}",
                                task.status.overview().label_zh(),
                                task.status.label_zh()
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(task.url.host_str().unwrap_or("未知站点").to_owned()),
                    ),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .whitespace_normal()
                    .text_color(rgb(TEXT))
                    .child(task.title),
            )
            .child(
                div()
                    .whitespace_normal()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(scheduled),
            )
            .child(
                div()
                    .whitespace_normal()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(format!("目标：{}", task.target.display_text())),
            )
    }

    fn copy_button(
        &self,
        id_key: &'static str,
        identity: u64,
        value: String,
        label_text: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id((id_key, identity))
            .flex_none()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL))
            .text_xs()
            .text_color(rgb(PRIMARY))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(PRIMARY_SOFT)))
            .child("复制")
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                this.notice = format!("{label_text}已复制到剪贴板").into();
                cx.notify();
            }))
    }

    fn copyable_info_block(
        &self,
        label_text: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> Div {
        let identity = element_key(&(label_text, value.as_str()));
        let copy_button = self.copy_button("copy-info", identity, value.clone(), label_text, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(240.0))
            .gap_2()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(label(label_text))
                    .child(copy_button),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .whitespace_normal()
                    .cursor_text()
                    .text_color(rgb(TEXT))
                    .child(value),
            )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> Div {
        let target_mode = self.editor.target_mode;
        let click_mode = self.editor.click_mode;
        let candidates = self
            .editor
            .candidates
            .iter()
            .take(12)
            .cloned()
            .map(|candidate| self.render_candidate(candidate, cx))
            .collect::<Vec<_>>();

        let candidate_list = div()
            .id("candidate-list-scroll")
            .flex()
            .flex_col()
            .gap_3()
            .max_h(px(520.0))
            .children(candidates)
            .when(self.editor.candidates.is_empty(), |panel| {
                panel.child(
                    div()
                        .p_5()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(BORDER))
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .whitespace_normal()
                        .child("检测后会显示文案、上下文、可点击状态和置信度。"),
                )
            })
            .overflow_y_scrollbar();

        let mut panel = div()
            .id("editor-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .p_6()
            .gap_5()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(280.0))
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .child(if self.editor.editing_task.is_some() {
                                        "编辑点击任务"
                                    } else {
                                        "新建点击任务"
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .whitespace_normal()
                                    .child("所有时间按明确 IANA 时区转换为 UTC 毫秒时间戳"),
                            ),
                    )
                    .child(
                        secondary_button("close-editor", "关闭").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.editor_open = false;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(360.0))
                            .gap_4()
                            .child(input_field("任务名称", &self.editor.title))
                            .child(input_field("页面 URL", &self.editor.url))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .min_w(px(260.0))
                                            .gap_2()
                                            .child(label("执行时间"))
                                            .child(Input::new(&self.editor.local_datetime)),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .w(px(190.0))
                                            .min_w(px(190.0))
                                            .gap_2()
                                            .child(label("IANA 时区"))
                                            .child(Input::new(&self.editor.timezone)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(label("目标识别方式"))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(
                                                choice_button(
                                                    "target-auto",
                                                    "自动推理",
                                                    target_mode == TargetMode::Auto,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.editor.target_mode = TargetMode::Auto;
                                                    this.editor.selected_candidate_id = None;
                                                    this.editor.selected_target = None;
                                                    this.editor.validated_manual_text = None;
                                                    this.editor.message =
                                                        "点击“检测页面按钮”进行自动推理".into();
                                                    cx.notify();
                                                })),
                                            )
                                            .child(
                                                choice_button(
                                                    "target-manual",
                                                    "手动文案",
                                                    target_mode == TargetMode::Manual,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.editor.target_mode = TargetMode::Manual;
                                                    this.editor.selected_candidate_id = None;
                                                    this.editor.selected_target = None;
                                                    this.editor.validated_manual_text = None;
                                                    this.editor.message =
                                                        "先检测页面，再验证手动文案".into();
                                                    cx.notify();
                                                })),
                                            ),
                                    ),
                            )
                            .when(target_mode == TargetMode::Manual, |panel| {
                                panel.child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_3()
                                        .items_end()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w(px(260.0))
                                                .gap_2()
                                                .child(label("按钮文案（精确匹配）"))
                                                .child(Input::new(&self.editor.manual_text)),
                                        )
                                        .child(
                                            primary_button("validate-manual", "验证文案").on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.validate_manual_target(cx);
                                                    cx.notify();
                                                }),
                                            ),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        primary_button(
                                            "detect-targets",
                                            if self.editor.busy {
                                                "处理中…"
                                            } else {
                                                "检测页面按钮"
                                            },
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                if !this.editor.busy {
                                                    this.detect_targets(cx);
                                                    cx.notify();
                                                }
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(240.0))
                                            .whitespace_normal()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(self.editor.message.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(label("点击策略"))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .child(
                                                choice_button(
                                                    "click-strict",
                                                    "严格准点",
                                                    click_mode == EditorClickMode::Strict,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.editor.click_mode =
                                                        EditorClickMode::Strict;
                                                    cx.notify();
                                                })),
                                            )
                                            .child(
                                                choice_button(
                                                    "click-wait",
                                                    "准点开始等待",
                                                    click_mode == EditorClickMode::Wait,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.editor.click_mode = EditorClickMode::Wait;
                                                    cx.notify();
                                                })),
                                            ),
                                    ),
                            )
                            .when(click_mode == EditorClickMode::Wait, |panel| {
                                panel.child(input_field(
                                    "按钮启用宽限期（毫秒，最多 60000）",
                                    &self.editor.grace_period_ms,
                                ))
                            })
                            .child(input_field(
                                "可选成功文案（URL 变化之外的成功信号）",
                                &self.editor.success_text,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(320.0))
                            .max_w(px(430.0))
                            .gap_3()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(BG))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .items_center()
                                    .gap_2()
                                    .child(div().text_lg().child("按钮候选"))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(format!("{} 项", self.editor.candidates.len())),
                                    ),
                            )
                            .child(candidate_list),
                    ),
            );

        panel = panel.child(
            div()
                .flex()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap_3()
                .pt_3()
                .border_t_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(260.0))
                        .whitespace_normal()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("表单与候选区均支持滚动；关键结果可在详情页一键复制。"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap_3()
                        .child(
                            secondary_button("cancel-save", "取消").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.editor_open = false;
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            primary_button(
                                "save-task",
                                if self.editor.busy {
                                    "处理中…"
                                } else {
                                    "保存并安排执行"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.editor.busy {
                                    this.save_editor(cx);
                                    cx.notify();
                                }
                            })),
                        ),
                ),
        );

        div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(rgb(PANEL))
            .child(panel.overflow_y_scrollbar())
    }

    fn render_candidate(
        &self,
        candidate: TargetCandidate,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected =
            self.editor.selected_candidate_id.as_deref() == Some(candidate.candidate_id.as_str());
        let candidate_for_select = candidate.clone();
        let target_for_highlight = candidate.to_fingerprint();
        let state_text = if candidate.is_clickable_now() {
            "当前可点击"
        } else if !candidate.enabled {
            "当前禁用"
        } else if candidate.covered {
            "当前被遮挡"
        } else {
            "当前不可点击"
        };
        let reasons = candidate
            .score_reasons
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join("；");

        div()
            .id(("candidate", element_key(&candidate.candidate_id)))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected { rgb(PRIMARY) } else { rgb(BORDER) })
            .bg(if selected {
                rgb(PRIMARY_SOFT)
            } else {
                rgb(PANEL)
            })
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(PRIMARY)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.editor.choose_candidate(&candidate_for_select);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.0))
                            .whitespace_normal()
                            .child(candidate.best_name().to_owned()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(PRIMARY))
                            .child(format!("{}%", candidate.confidence)),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if candidate.is_clickable_now() {
                        rgb(SUCCESS)
                    } else {
                        rgb(WARNING)
                    })
                    .child(state_text),
            )
            .when_some(candidate.context_text.clone(), |panel, context| {
                panel.child(
                    div()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("上下文：{context}")),
                )
            })
            .when(!reasons.is_empty(), |panel| {
                panel.child(
                    div()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(reasons),
                )
            })
            .child(
                div()
                    .id(("highlight", element_key(&candidate.candidate_id)))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .text_color(rgb(TEXT))
                    .text_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(BG)))
                    .child("在浏览器中高亮")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.highlight_target(target_for_highlight.clone());
                        cx.notify();
                    })),
            )
    }

    fn render_log(&self, log: ExecutionLog, cx: &mut Context<Self>) -> Div {
        let (color, background, outcome_label) = match log.outcome {
            ExecutionOutcome::Succeeded => (SUCCESS, SUCCESS_SOFT, "成功"),
            ExecutionOutcome::Failed => (DANGER, DANGER_SOFT, "失败"),
            ExecutionOutcome::Uncertain => (WARNING, WARNING_SOFT, "已点击但未确认"),
        };
        let timing = match (log.dispatched_at, log.dispatch_delay_ms) {
            (Some(value), Some(delay)) => format!(
                "派发：{} · 偏差 {:+} ms",
                value.format("%Y-%m-%d %H:%M:%S%.3f UTC"),
                delay
            ),
            _ => "未派发点击".into(),
        };
        let final_url = log.final_url.as_ref().map(ToString::to_string);
        let mut copy_text = format!("执行结果：{outcome_label}\n{timing}\n说明：{}", log.message);
        if let Some(url) = &final_url {
            copy_text.push_str("\n结果页面：");
            copy_text.push_str(url);
        }
        let copy_button =
            self.copy_button("copy-log", element_key(&log.id), copy_text, "执行记录", cx);

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(background))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(rgb(color)).child(outcome_label))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(timing),
                            )
                            .child(copy_button),
                    ),
            )
            .child(
                div()
                    .whitespace_normal()
                    .cursor_text()
                    .text_sm()
                    .child(log.message),
            )
            .when_some(final_url, |panel, url| {
                panel.child(
                    div()
                        .whitespace_normal()
                        .cursor_text()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(format!("结果页面：{url}")),
                )
            })
    }

    fn render_details(&self, cx: &mut Context<Self>) -> Div {
        let selected = self
            .selected
            .and_then(|id| self.tasks.iter().find(|task| task.id == id))
            .cloned();
        let Some(task) = selected else {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .bg(rgb(PANEL))
                .text_color(rgb(MUTED))
                .child(
                    div()
                        .text_2xl()
                        .text_color(rgb(TEXT))
                        .child("还没有点击任务"),
                )
                .child(
                    div()
                        .whitespace_normal()
                        .child("新建任务后，后台调度器会自动恢复和执行所有待执行任务。"),
                );
        };

        let scheduled = format_in_timezone(task.scheduled_at_utc, &task.timezone)
            .unwrap_or_else(|_| task.scheduled_at_utc.to_rfc3339());
        let (status_color, status_bg) = status_colors(task.status);
        let result_text = task
            .result
            .as_ref()
            .map(|result| result.message.clone())
            .unwrap_or_else(|| "尚未执行".into());
        let final_url = task
            .result
            .as_ref()
            .and_then(|result| result.final_url.as_ref())
            .map(ToString::to_string)
            .unwrap_or_else(|| "—".into());
        let execution_locked = task.status == TaskStatus::Executing;
        let logs = if self.logs_task_id == Some(task.id) {
            self.logs
                .iter()
                .cloned()
                .map(|log| self.render_log(log, cx))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let log_count = logs.len();
        let task_id = task.id.to_string();
        let task_id_copy = self.copy_button(
            "copy-task-id",
            element_key(&task.id),
            task_id.clone(),
            "任务 ID",
            cx,
        );

        let content = div()
            .id("details-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .p_6()
            .gap_5()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(280.0))
                            .gap_1()
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_2xl()
                                    .child(task.title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .whitespace_normal()
                                            .cursor_text()
                                            .text_sm()
                                            .text_color(rgb(MUTED))
                                            .child(task_id),
                                    )
                                    .child(task_id_copy),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(rgb(status_bg))
                            .text_color(rgb(status_color))
                            .child(format!(
                                "{} · {}",
                                task.status.overview().label_zh(),
                                task.status.label_zh()
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(self.copyable_info_block("执行时间", scheduled, cx))
                    .child(self.copyable_info_block(
                        "目标按钮",
                        task.target.display_text().to_owned(),
                        cx,
                    )),
            )
            .child(self.copyable_info_block("页面 URL", task.url.to_string(), cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(self.copyable_info_block("执行结果", result_text, cx))
                    .child(self.copyable_info_block("结果页面", final_url, cx)),
            )
            .when(!execution_locked, |panel| {
                panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .child(primary_button("edit-task", "编辑 / 重新安排").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_edit_editor(window, cx);
                                cx.notify();
                            }),
                        ))
                        .child(
                            danger_button("delete-task", "删除任务").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.delete_selected();
                                    cx.notify();
                                },
                            )),
                        ),
                )
            })
            .when(execution_locked, |panel| {
                panel.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(WARNING_SOFT))
                        .whitespace_normal()
                        .text_color(rgb(WARNING))
                        .child("点击已经进入派发阶段。为避免重复提交，此时不能编辑或删除任务。"),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(div().text_lg().child("执行记录"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(format!("{log_count} 条")),
                            ),
                    )
                    .children(logs)
                    .when(log_count == 0, |panel| {
                        panel.child(
                            div()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .text_color(rgb(MUTED))
                                .child("暂无执行记录"),
                        )
                    }),
            );

        div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(rgb(PANEL))
            .child(content.overflow_y_scrollbar())
    }
}

impl Render for PunctualDashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self
            .tasks
            .clone()
            .into_iter()
            .map(|task| {
                let selected = self.selected == Some(task.id);
                Self::render_task_card(task, selected, cx)
            })
            .collect::<Vec<_>>();
        let connection_color = if self.browser_connected {
            SUCCESS
        } else {
            MUTED
        };
        let right_panel = if self.editor_open {
            self.render_editor(cx)
        } else {
            self.render_details(cx)
        };

        let task_list = div()
            .id("task-list-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .gap_3()
            .px_4()
            .pb_4()
            .children(cards)
            .when(self.tasks.is_empty(), |panel| {
                panel.child(
                    div()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(PANEL))
                        .whitespace_normal()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("暂无任务。创建后会显示待执行或已执行状态。"),
                )
            })
            .overflow_y_scrollbar();

        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .h(px(66.0))
                    .min_w(px(0.0))
                    .gap_4()
                    .px_5()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_3()
                            .child(div().text_xl().child("Punctual · 准点"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("Rust + GPUI + 多浏览器自动化"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .items_center()
                            .justify_end()
                            .gap_3()
                            .child(
                                div()
                                    .flex_none()
                                    .text_sm()
                                    .text_color(rgb(connection_color))
                                    .child({
                                        let name = self
                                            .browser_name
                                            .as_ref()
                                            .map(|name| name.as_ref())
                                            .unwrap_or("浏览器");
                                        if self.browser_connected {
                                            format!("● {name} 已连接")
                                        } else {
                                            format!("○ {name} 按需启动")
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .max_w(px(520.0))
                                    .truncate()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child(self.notice.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(360.0))
                            .min_w(px(360.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .border_r_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(BG))
                            .child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .justify_between()
                                    .items_center()
                                    .gap_3()
                                    .p_4()
                                    .child(div().text_lg().child("点击任务"))
                                    .child(primary_button("new-task", "＋ 新建任务").on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.open_new_editor(window, cx);
                                            cx.notify();
                                        }),
                                    )),
                            )
                            .child(task_list),
                    )
                    .child(right_panel),
            )
    }
}

fn element_key(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn input_field(label_text: &'static str, state: &Entity<InputState>) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .gap_2()
        .child(label(label_text))
        .child(Input::new(state))
}

fn label(value: &'static str) -> Div {
    div().text_sm().text_color(rgb(MUTED)).child(value)
}

fn primary_button(id: &'static str, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(PRIMARY))
        .text_color(rgb(0xffffff))
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.opacity(0.88))
        .child(text)
}

fn secondary_button(id: &'static str, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(PANEL))
        .text_color(rgb(TEXT))
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(BG)))
        .child(text)
}

fn danger_button(id: &'static str, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_4()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(0xfecdca))
        .bg(rgb(DANGER_SOFT))
        .text_color(rgb(DANGER))
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.opacity(0.88))
        .child(text)
}

fn choice_button(id: &'static str, text: &'static str, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_4()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(if selected { rgb(PRIMARY) } else { rgb(BORDER) })
        .bg(if selected {
            rgb(PRIMARY_SOFT)
        } else {
            rgb(PANEL)
        })
        .text_color(if selected { rgb(PRIMARY) } else { rgb(TEXT) })
        .text_sm()
        .cursor_pointer()
        .child(text)
}

fn status_colors(status: TaskStatus) -> (u32, u32) {
    match status {
        TaskStatus::Succeeded => (SUCCESS, SUCCESS_SOFT),
        TaskStatus::Failed | TaskStatus::Missed => (DANGER, DANGER_SOFT),
        TaskStatus::Uncertain => (WARNING, WARNING_SOFT),
        _ => (PRIMARY, PRIMARY_SOFT),
    }
}
