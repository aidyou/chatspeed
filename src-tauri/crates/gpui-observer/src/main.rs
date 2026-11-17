//! Native read-only rendering of the formal workflow message projection.
//!
//! Visual rules mirror `src/components/workflow/WorkflowMessageList.vue` and
//! `src/styles/workflow/messages.scss`: a CLI-style tool transcript, grouped
//! tool activity, collapsible snapshots and delegated-task cards.

use chatspeed_gpui_observer::{
    icons,
    metrics::Metrics,
    model::Model,
    presentation::{
        self, ActivityItem, ActivityView, AlertKind, AlertView, AssistantView, ChoiceGroupView,
        CostTotals, CostView, FinishTaskView, Row, SnapshotView, SubAgentStatus, SubAgentView,
        ToolStatus, ToolView, UserView,
    },
    types::*,
    worker::Worker,
};
use clap::Parser;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        h_flex,
        text::{TextView, TextViewState},
        v_flex, ActiveTheme,
    },
    *,
};
use rust_i18n::t;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

rust_i18n::i18n!("locales", fallback = "en");

/// ChatSpeed's default primary preset. The user-selected preset replaces this
/// once the native window reads the application theme.
const ACCENT: u32 = 0x41b555;
const SUCCESS: u32 = 0x41b555;
const DANGER: u32 = 0xf56c6c;
const WARNING: u32 = 0xffc107;
const INFO: u32 = 0x46a2ed;
const VIOLET: u32 = 0x8b5cf6;
const MONO_FONT: &str = "Liberation Mono";

/// Vue's collapsed user bubble: four 1.6-line-height rows at 14px, plus room
/// for the corner toggle.
const USER_MESSAGE_LINES: usize = 4;
const USER_MESSAGE_COLLAPSED_HEIGHT: f32 = 114.;

fn accent() -> Rgba {
    rgb(ACCENT)
}

fn success() -> Rgba {
    rgb(SUCCESS)
}

fn danger() -> Rgba {
    rgb(DANGER)
}

fn warning() -> Rgba {
    rgb(WARNING)
}

fn info() -> Rgba {
    rgb(INFO)
}

fn violet() -> Rgba {
    rgb(VIOLET)
}

/// Iconfont glyph inside a square box so glyph metrics never shift baselines.
fn font_icon(name: &str, size: f32, color: impl Into<Hsla>) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(size))
        .h(px(size))
        .font_family(icons::FAMILY)
        .text_size(px(size))
        // The square line box keeps the glyph's own ascent/descent from pushing
        // the icon below the text it sits next to.
        .line_height(px(size))
        .text_color(color)
        .child(icons::glyph(name))
}

fn pill(label: String, color: Rgba, background: Option<Rgba>) -> impl IntoElement {
    div()
        .flex_none()
        .px_2()
        .py(px(1.))
        .rounded(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .when_some(background, |view, background| view.bg(background))
        .child(label)
}

/// User and raw tool text keep their authored line breaks.
fn plain_text(text: &str, color: impl Into<Hsla>) -> impl IntoElement {
    let color = color.into();
    let mut body = v_flex().w_full().min_w_0();
    for line in text.split('\n') {
        body = body.child(if line.trim().is_empty() {
            div().h(px(8.))
        } else {
            div().text_color(color).child(line.to_string())
        });
    }
    body
}

/// Groups digits the way `Intl.NumberFormat` does for token counts.
fn grouped_number(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn format_cost(value: Option<f64>) -> String {
    match value {
        Some(cost) => format!("${cost:.6}"),
        None => t!("cost_unpriced").to_string(),
    }
}

fn format_rate(value: Option<f64>) -> String {
    match value {
        Some(rate) => format!("${rate:.4} / M"),
        None => t!("cost_unpriced_rate").to_string(),
    }
}

fn format_percent(value: f64) -> String {
    format!("{value:.2}%")
}

fn format_cost_duration(duration_ms: Option<u64>) -> String {
    match duration_ms {
        Some(duration) => {
            let seconds = duration / 1000;
            format!("{}m {}s", seconds / 60, seconds % 60)
        }
        None => "--".to_owned(),
    }
}

/// One numbered `ask_user` option with its read-only selection marker.
fn choice_option(label: String, selected: bool, cx: &mut App) -> AnyElement {
    let (icon, color) = if selected {
        ("check-circle", Hsla::from(accent()))
    } else {
        ("uncheck", cx.theme().muted_foreground)
    };
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .py(px(1.))
        .child(font_icon(icon, 14., color))
        .child(
            div()
                .min_w_0()
                .text_size(px(12.))
                .text_color(if selected {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(label),
        )
        .into_any_element()
}

struct Observer {
    worker: Worker,
    options: Options,
    model: Model,
    sessions: Vec<Session>,
    selected: Option<String>,
    connected: bool,
    follow: bool,
    scroll_pending: bool,
    scroll: ScrollHandle,
    prepend_anchor: Option<(Point<Pixels>, Point<Pixels>)>,
    deferred_update: Option<Update>,
    rows: Vec<Row>,
    markdown: BTreeMap<String, (String, Entity<TextViewState>)>,
    expanded: BTreeSet<String>,
    compression_started: Option<Instant>,
    metrics: Metrics,
    switched: Option<Instant>,
    last_metrics: Instant,
    task: Task<()>,
}

impl Observer {
    fn new(worker: Worker, options: Options, cx: &mut Context<Self>) -> Self {
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| loop {
            executor.timer(Duration::from_millis(50)).await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        });
        Self {
            worker,
            options,
            model: Model::default(),
            sessions: Vec::new(),
            selected: None,
            connected: false,
            follow: true,
            scroll_pending: false,
            scroll: ScrollHandle::new(),
            prepend_anchor: None,
            deferred_update: None,
            rows: Vec::new(),
            markdown: BTreeMap::new(),
            expanded: BTreeSet::new(),
            compression_started: None,
            metrics: Metrics {
                started: Some(Instant::now()),
                ..Metrics::default()
            },
            switched: None,
            last_metrics: Instant::now(),
            task,
        }
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        let generation = self.model.generation.wrapping_add(1);
        if self
            .worker
            .commands
            .try_send(Command::Select {
                generation,
                id: id.clone(),
            })
            .is_err()
        {
            return;
        }
        self.model.select();
        self.selected = Some(id);
        self.markdown.clear();
        self.expanded.clear();
        self.rows.clear();
        self.follow = true;
        self.switched = Some(Instant::now());
        self.scroll = ScrollHandle::new();
        self.prepend_anchor = None;
        self.deferred_update = None;
        self.compression_started = None;
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        self.metrics.queue_depth_peak =
            self.metrics.queue_depth_peak.max(self.worker.updates.len());
        let mut dirty = false;
        for _ in 0..128 {
            if self.prepend_anchor.is_some() {
                break;
            }
            let Some(update) = self
                .deferred_update
                .take()
                .or_else(|| self.worker.updates.try_recv().ok())
            else {
                break;
            };
            // Give prepends an isolated layout frame; appended height must not affect restoration.
            if dirty && matches!(&update.kind, UpdateKind::Older(_)) {
                self.deferred_update = Some(update);
                break;
            }
            self.metrics.queue(update.queued);
            self.metrics.queue_depth_peak = self.metrics.queue_depth_peak.max(update.queue_depth);
            if update.generation != 0 && !self.model.accepts(update.generation) {
                self.metrics.stale_responses += 1;
                continue;
            }
            match update.kind {
                UpdateKind::Sessions(sessions) => self.sessions = sessions,
                UpdateKind::Connected(connected) => self.connected = connected,
                UpdateKind::Reconnect => self.metrics.reconnects += 1,
                UpdateKind::DecodeUs(us) => {
                    self.metrics.snapshot_decode_us += us;
                    self.metrics.snapshot_samples += 1;
                }
                UpdateKind::Reset => {
                    self.metrics.resets += 1;
                    self.model.reset_live();
                }
                UpdateKind::Snapshot(page) => {
                    self.model.page(page, false);
                    if let Some(start) = self.switched.take() {
                        self.metrics.switch_samples += 1;
                        self.metrics.switch_ui_applied_us = start.elapsed().as_micros() as u64;
                        self.metrics.switch_ui_applied_us_total +=
                            self.metrics.switch_ui_applied_us;
                        self.metrics.switch_ui_applied_us_max = self
                            .metrics
                            .switch_ui_applied_us_max
                            .max(self.metrics.switch_ui_applied_us);
                    }
                }
                UpdateKind::Older(page) => {
                    self.prepend_anchor = Some((self.scroll.offset(), self.scroll.max_offset()));
                    self.model.page(page, true);
                    self.follow = false;
                }
                UpdateKind::Event(event) => {
                    self.metrics.events += 1;
                    self.metrics.event_bytes +=
                        serde_json::to_vec(&event).map_or(0, |v| v.len() as u64);
                    self.model.event(event);
                }
            }
            dirty = true;
        }
        if self.model.compressing && self.compression_started.is_none() {
            self.compression_started = Some(Instant::now());
        }
        if !self.model.compressing {
            self.compression_started = None;
        }
        if dirty {
            if self.follow {
                self.scroll_pending = true;
            }
            self.rows = presentation::project(self.transcript(), self.roots());
            self.refresh_markdown(cx);
            self.metrics.ui_batches += 1;
            cx.notify();
        }
        if self.last_metrics.elapsed() >= Duration::from_secs(5) {
            self.metrics.cached_messages = self.model.messages.len();
            self.metrics.cached_markdown = self.markdown.len();
            let metrics = self.metrics.clone();
            if self
                .worker
                .commands
                .try_send(Command::Metrics(metrics, self.options.metrics.clone()))
                .is_ok()
            {
                self.metrics.samples += 1;
            }
            self.last_metrics = Instant::now();
        }
    }

    fn roots(&self) -> &[String] {
        &[]
    }

    /// Durable rows plus the transient live message and live tool lifecycle state.
    fn transcript(&self) -> Vec<Message> {
        let mut messages: Vec<Message> = self.model.visible().cloned().collect();
        if !self.model.live.message.is_empty() || self.model.live.reasoning.is_some() {
            let mut live = self.model.live.clone();
            live.id = -1;
            live.role = "assistant".into();
            messages.push(live);
        }
        for id in &self.model.tool_order {
            let Some(tool) = self.model.tools.get(id) else {
                continue;
            };
            if messages
                .iter()
                .any(|message| message.metadata["tool_call_id"].as_str() == Some(id.as_str()))
            {
                continue;
            }
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            id.hash(&mut hash);
            messages.push(Message {
                id: -((hash.finish() & 0x3fff_ffff_ffff_ffff) as i64) - 2,
                role: "tool".into(),
                metadata: tool.clone(),
                ..Message::default()
            });
        }
        messages
    }

    /// Collect every visible Markdown row once per projection change.
    fn refresh_markdown(&mut self, cx: &mut Context<Self>) {
        let start = Instant::now();
        let mut rows: Vec<(String, String)> = Vec::new();
        for row in &self.rows {
            match row {
                Row::Assistant(view) => {
                    if !view.markdown.is_empty() {
                        rows.push((view.key.clone(), view.markdown.clone()));
                    }
                    if self.expanded.contains(&view.thought_key) {
                        if let Some(reasoning) = &view.reasoning {
                            rows.push((view.thought_key.clone(), reasoning.clone()));
                        }
                    }
                }
                Row::Activity(group) => {
                    if !self.expanded.contains(&group.key) {
                        continue;
                    }
                    for item in &group.items {
                        if let Some(thought) = &item.thought {
                            if self.expanded.contains(&item.thought_key) {
                                rows.push((item.thought_key.clone(), thought.clone()));
                            }
                        }
                        if let Some(tool) = &item.tool {
                            if self.expanded.contains(&tool.detail_key) && !tool.detail.is_empty() {
                                rows.push((tool.detail_key.clone(), tool.detail.clone()));
                            }
                        }
                    }
                }
                Row::Tool(tool) => {
                    if self.expanded.contains(&tool.detail_key) && !tool.detail.is_empty() {
                        rows.push((tool.detail_key.clone(), tool.detail.clone()));
                    }
                }
                Row::Snapshot(snapshot) => {
                    if self.expanded.contains(&snapshot.key) && !snapshot.body.is_empty() {
                        rows.push((snapshot.body_key.clone(), snapshot.body.clone()));
                    }
                }
                Row::SubAgent(card) => {
                    if !self.expanded.contains(&card.key) {
                        continue;
                    }
                    if self.expanded.contains(&card.task_key) && !card.task.is_empty() {
                        rows.push((card.task_key.clone(), card.task.clone()));
                    }
                    if self.expanded.contains(&card.result_key) && !card.result.is_empty() {
                        rows.push((card.result_key.clone(), card.result.clone()));
                    }
                }
                Row::User(_) | Row::FinishTask(_) | Row::Alert(_) | Row::Divider => {}
            }
        }
        self.markdown
            .retain(|key, _| rows.iter().any(|(id, _)| id == key));
        for (key, text) in rows {
            if let Some((previous, entity)) = self.markdown.get_mut(&key) {
                if previous != &text {
                    entity.update(cx, |state, cx| {
                        if let Some(delta) = text.strip_prefix(previous.as_str()) {
                            state.push_str(delta, cx);
                        } else {
                            state.set_text(&text, cx);
                        }
                    });
                    *previous = text;
                }
            } else {
                let entity = cx.new(|cx| TextViewState::markdown(&text, cx));
                self.markdown.insert(key, (text, entity));
            }
        }
        self.metrics.markdown_samples += 1;
        self.metrics.markdown_update_us += start.elapsed().as_micros() as u64;
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        self.refresh_markdown(cx);
        cx.notify();
    }

    fn markdown_view(&self, key: &str) -> AnyElement {
        match self.markdown.get(key) {
            Some((_, state)) => div()
                .w_full()
                .min_w_0()
                .child(TextView::new(state).selectable(true).code_block_actions(
                    |code_block, _, cx| {
                        // GPUI renders these in the code block's top-right corner, where
                        // the language label would otherwise sit.
                        let code = code_block.code().to_string();
                        let lang = code_block
                            .lang()
                            .map(|lang| lang.to_string())
                            .unwrap_or_default();
                        let muted = cx.theme().muted_foreground;
                        h_flex()
                            .id("code-actions")
                            .items_center()
                            .gap_2()
                            .px_1()
                            .text_size(px(10.))
                            .text_color(muted)
                            .when(!lang.is_empty(), |row| row.child(lang))
                            .child(
                                h_flex()
                                    .id("copy-code")
                                    .items_center()
                                    .rounded(px(4.))
                                    .p(px(2.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(cx.theme().muted))
                                    .child(font_icon("copy", 12., muted))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code.clone(),
                                        ));
                                    }),
                            )
                            .into_any_element()
                    },
                ))
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn user_row(&self, view: &UserView, cx: &mut Context<Self>) -> AnyElement {
        // Vue collapses past four lines and keeps the toggle in the corner.
        let toggle_key = format!("{}:user", view.key);
        let target = toggle_key.clone();
        let expanded = self.expanded.contains(&toggle_key);
        let expandable = presentation::user_message_lines(&view.text) > USER_MESSAGE_LINES;
        let toggle = h_flex()
            .id(SharedString::from(toggle_key))
            .absolute()
            .right_1()
            .bottom_1()
            .w(px(20.))
            .h(px(20.))
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .bg(cx.theme().muted)
            .cursor_pointer()
            .child(font_icon(
                if expanded {
                    "double-arrow-up"
                } else {
                    "double-arrow-down"
                },
                12.,
                cx.theme().muted_foreground,
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx)));
        h_flex()
            .w_full()
            .min_w_0()
            .justify_end()
            .child(
                div()
                    .relative()
                    .max_w(px(680.))
                    .min_w_0()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .px_4()
                    .py_3()
                    .text_size(px(14.))
                    .when(expandable && !expanded, |bubble| {
                        // Reserve the corner so the toggle never covers the text.
                        bubble
                            .max_h(px(USER_MESSAGE_COLLAPSED_HEIGHT))
                            .overflow_hidden()
                            .pr(px(28.))
                            .pb(px(28.))
                    })
                    .when(expandable, |bubble| bubble.child(toggle))
                    .child(plain_text(&view.text, cx.theme().foreground)),
            )
            .into_any_element()
    }

    fn thought_row(
        &self,
        key: String,
        title: String,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.expanded.contains(&key);
        let target = key.clone();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .id(SharedString::from(key.clone()))
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .child(font_icon("reasoning", 14., muted))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(13.))
                            .text_color(muted)
                            .italic()
                            .child(title),
                    )
                    .child(font_icon(
                        if open { "caret-down" } else { "caret-right" },
                        12.,
                        muted,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx))),
            )
            .when(open, |view| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .rounded(px(4.))
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().muted)
                        .px_3()
                        .py_2()
                        .text_size(px(13.))
                        .italic()
                        .text_color(cx.theme().muted_foreground)
                        .child(plain_text(text, cx.theme().muted_foreground)),
                )
            })
            .into_any_element()
    }

    fn assistant_row(&self, view: &AssistantView, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().w_full().min_w_0().gap_2();
        if let Some(reasoning) = &view.reasoning {
            body = body.child(self.thought_row(
                view.thought_key.clone(),
                view.thought_title.clone(),
                reasoning,
                cx,
            ));
        }
        if !view.markdown.is_empty() {
            body = body.child(self.markdown_view(&view.key));
        }
        body.into_any_element()
    }

    fn tool_icon_color(&self, tool: &ToolView) -> Rgba {
        match tool.kind {
            Some(presentation::ToolGroupKind::Read)
            | Some(presentation::ToolGroupKind::Search)
            | Some(presentation::ToolGroupKind::Mutation) => accent(),
            Some(presentation::ToolGroupKind::Command) => info(),
            Some(presentation::ToolGroupKind::Task) => violet(),
            _ => accent(),
        }
    }

    fn tool_row(&self, tool: &ToolView, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&tool.detail_key);
        let target_key = tool.detail_key.clone();
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let border = cx.theme().border;
        let summary_color: Hsla = if tool.status.is_error() {
            danger().into()
        } else {
            muted
        };
        let status_color: Hsla = match tool.status {
            ToolStatus::Running | ToolStatus::Pending => accent().into(),
            ToolStatus::Failed | ToolStatus::Rejected => danger().into(),
            ToolStatus::Completed => border,
        };
        let expandable = !tool.detail.is_empty()
            || !tool.choices.is_empty()
            || tool.diff.is_some()
            || tool.command.is_some()
            || tool.arguments.is_some();
        let mut title = h_flex()
            .id(SharedString::from(tool.detail_key.clone()))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .py_1()
            .pl_2()
            .border_l_2()
            .border_color(status_color)
            .cursor_pointer()
            .child(font_icon(tool.icon, 15., self.tool_icon_color(tool)))
            .child(
                div()
                    .flex_none()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(foreground)
                    .child(truncate_text(&tool.action, 160)),
            );
        if !tool.target.is_empty() {
            title = title.child(
                div()
                    .flex_none()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(truncate_text(&tool.target, 120)),
            );
        }
        title = title.child(div().flex_1());
        for badge in &tool.badges {
            title = title.child(
                div()
                    .flex_none()
                    .px_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(badge.clone()),
            );
        }
        if tool.approved {
            title = title.child(font_icon("check", 12., success()));
        }
        if expandable {
            title = title
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(target_key.clone(), cx)));
        }
        let mut body = v_flex().w_full().min_w_0().cursor_pointer().child(title);
        if !open {
            let mut summary = h_flex().w_full().min_w_0().items_start().gap_1().pl_6();
            summary = summary
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child("⎿"),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(12.))
                        .text_color(summary_color)
                        .child(tool.summary.clone()),
                );
            // Vue keeps the hint right after the summary instead of pinning it
            // to the far edge of the row.
            if expandable {
                summary = summary.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(muted)
                        .opacity(0.6)
                        .child(t!("click_to_expand").to_string()),
                );
            }
            if !tool.summary.is_empty() {
                body = body.child(summary);
            }
        } else {
            body = body.child(self.tool_detail(tool, cx));
        }
        body.into_any_element()
    }

    fn tool_detail(&self, tool: &ToolView, cx: &mut Context<Self>) -> AnyElement {
        let mut detail = v_flex().w_full().min_w_0().gap_2().pl_6().pt_2().pb_2();
        if !tool.choices.is_empty() {
            let mut choices = v_flex().w_full().min_w_0().gap_3();
            for group in &tool.choices {
                choices = choices.child(self.choice_group(group, cx));
            }
            detail = detail.child(choices);
        }
        if let Some(lines) = &tool.diff {
            let mut diff = v_flex()
                .w_full()
                .rounded(px(4.))
                .text_size(px(12.))
                .font_family(MONO_FONT);
            let (mut old_line, mut new_line) = (0usize, 0usize);
            for (added, line) in lines {
                if *added {
                    new_line += 1;
                } else {
                    old_line += 1;
                }
                let color = if *added { success() } else { danger() };
                diff =
                    diff.child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .px_2()
                            .bg(color.opacity(0.12))
                            .text_color(color)
                            .child(div().flex_none().w(px(36.)).text_size(px(10.)).child(
                                if *added {
                                    new_line.to_string()
                                } else {
                                    old_line.to_string()
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(format!("{} {line}", if *added { "+" } else { "−" })),
                            ),
                    );
            }
            detail = detail.child(diff);
        }
        if let Some(command) = &tool.command {
            detail = detail.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_2()
                    .text_size(px(13.))
                    .font_family(MONO_FONT)
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent())
                            .child("$"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(plain_text(command, cx.theme().muted_foreground)),
                    ),
            );
        }
        if let Some(arguments) = &tool.arguments {
            detail = detail
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("arguments_label").to_string()),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .rounded(px(4.))
                        .bg(cx.theme().muted)
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .font_family(MONO_FONT)
                        .text_color(cx.theme().muted_foreground)
                        .child(plain_text(arguments, cx.theme().muted_foreground)),
                );
        }
        if !tool.detail.is_empty() {
            detail = detail.child(self.markdown_view(&tool.detail_key));
        }
        detail.into_any_element()
    }

    fn activity_row(&self, group: &ActivityView, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&group.key);
        let target = group.key.clone();
        let muted = cx.theme().muted_foreground;
        let mut header = h_flex()
            .id(SharedString::from(group.key.clone()))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .py_1()
            .pl_2()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .child(font_icon(group.icon, 15., muted));
        if group.thought_count > 0 {
            header = header.child(pill(
                format!("{} x{}", t!("thought"), group.thought_count),
                accent(),
                Some(accent().opacity(0.12)),
            ));
        }
        if group.error_count > 0 {
            // Vue tints group errors into the background instead of shouting in red.
            header = header.child(
                div()
                    .flex_none()
                    .px_2()
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(cx.theme().border.opacity(0.35))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(format!("{} x{}", t!("error"), group.error_count)),
            );
        }
        header = header
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .text_color(if group.ongoing {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(group.summary.clone()),
            )
            .child(font_icon(
                if open { "caret-down" } else { "caret-right" },
                14.,
                muted,
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx)));
        let mut body = v_flex().w_full().min_w_0().child(header);
        if open {
            let mut details = v_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .pl_4()
                .pt_1()
                .border_l_1()
                .border_color(cx.theme().border);
            for item in &group.items {
                details = details.child(self.activity_item(item, cx));
            }
            body = body.child(details);
        }
        body.into_any_element()
    }

    fn activity_item(&self, item: &ActivityItem, cx: &mut Context<Self>) -> AnyElement {
        match (&item.thought, &item.tool) {
            (Some(thought), _) => self.thought_row(
                item.thought_key.clone(),
                t!("thought_completed").to_string(),
                thought,
                cx,
            ),
            (None, Some(tool)) => self.tool_row(tool, cx),
            (None, None) => div().into_any_element(),
        }
    }

    fn status_color(&self, status: SubAgentStatus) -> Rgba {
        match status {
            SubAgentStatus::Running => accent(),
            SubAgentStatus::Completed => success(),
            SubAgentStatus::Failed => danger(),
            SubAgentStatus::Cancelled => rgb(0x848586),
        }
    }

    fn sub_agent_row(&self, card: &SubAgentView, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&card.key);
        let target = card.key.clone();
        let status_color = self.status_color(card.status);
        let muted = cx.theme().muted_foreground;
        let header = h_flex()
            .id(SharedString::from(card.key.clone()))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .py_1()
            .pl_2()
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .child(font_icon("skill-relation-chart", 15., muted))
            .child(pill(
                card.status_label.clone(),
                status_color,
                Some(status_color.opacity(0.12)),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .text_color(cx.theme().foreground)
                    .child(card.agent.clone()),
            )
            .child(font_icon(
                if open { "caret-down" } else { "caret-right" },
                14.,
                muted,
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx)));
        let mut body = v_flex().w_full().min_w_0().child(header);
        if open {
            let mut rows = h_flex().w_full().min_w_0().gap_3();
            for (label, value) in [
                (t!("sub_agent_agent").to_string(), card.agent.clone()),
                (t!("sub_agent_mode").to_string(), card.mode.clone()),
                (t!("sub_agent_tools").to_string(), card.tools.clone()),
                (t!("sub_agent_context").to_string(), card.context.clone()),
            ] {
                rows = rows.child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(label.to_uppercase()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_size(px(13.))
                                .text_color(cx.theme().foreground)
                                .child(value),
                        ),
                );
            }
            body = body.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .p_3()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(font_icon("task", 15., warning()))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .child(t!("sub_agent_delegated_task").to_string()),
                            )
                            .child(div().flex_1())
                            .when(!card.usage.is_empty(), |view| {
                                view.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(muted)
                                        .child(card.usage.clone()),
                                )
                            }),
                    )
                    .child(rows)
                    .child(self.sub_agent_section(
                        t!("sub_agent_task_section").to_string(),
                        card.task_key.clone(),
                        &card.task,
                        &card.task_preview,
                        cx,
                    ))
                    .when(card.has_result, |view| {
                        view.child(self.sub_agent_section(
                            t!("sub_agent_result_section").to_string(),
                            card.result_key.clone(),
                            &card.result,
                            &card.result_preview,
                            cx,
                        ))
                    })
                    .when_some(card.cost.as_ref(), |view, cost| {
                        view.child(self.cost_section(card.cost_key.clone(), cost, cx))
                    }),
            );
        }
        body.into_any_element()
    }

    fn choice_group(&self, group: &ChoiceGroupView, cx: &mut Context<Self>) -> AnyElement {
        let mut options = v_flex().w_full().min_w_0().gap_1();
        for (index, option) in group.options.iter().enumerate() {
            options = options.child(choice_option(
                format!("{}. {option}", index + 1),
                group.selected == Some(index),
                cx,
            ));
        }
        options = options.child(choice_option(
            format!("{}. {}", group.options.len() + 1, t!("ask_user_other")),
            group.other_selected,
            cx,
        ));
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(group.title.clone()),
            )
            .child(options)
            .when_some(group.answer.as_ref(), |view, answer| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .rounded(px(6.))
                        .bg(cx.theme().muted)
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(cx.theme().foreground)
                        .child(answer.clone()),
                )
            })
            .into_any_element()
    }

    fn cost_section(&self, key: String, cost: &CostView, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&key);
        let target = key.clone();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .id(SharedString::from(key.clone()))
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .child(t!("cost_label").to_string().to_uppercase()),
                    )
                    .child(font_icon(
                        if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        },
                        12.,
                        muted,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx))),
            )
            .when(open, |view| view.child(self.cost_card(cost, cx)))
            .into_any_element()
    }

    fn cost_card(&self, cost: &CostView, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let mut card = v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .px_3()
            .py_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(format_cost_duration(cost.duration_ms)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(format!(
                                "{} {}",
                                grouped_number(cost.with_sub_agents.total_tokens),
                                t!("cost_tokens")
                            )),
                    ),
            )
            .child(self.cost_row(t!("cost_self").to_string(), &cost.self_usage, true, cx));
        if cost.has_sub_agents {
            card = card.child(self.cost_row(
                t!("cost_with_sub_agents").to_string(),
                &cost.with_sub_agents,
                false,
                cx,
            ));
        }
        for model in &cost.models {
            let mut tokens = h_flex().w_full().gap_3();
            for (label, value) in [
                (t!("cost_input").to_string(), model.input_tokens),
                (t!("cost_cache").to_string(), model.cache_tokens),
                (t!("cost_output").to_string(), model.output_tokens),
            ] {
                tokens = tokens.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("{label} {}", grouped_number(value))),
                );
            }
            if let Some(rate) = model.cache_hit_rate {
                tokens = tokens.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!(
                            "{} {}",
                            t!("cost_cache_hit_rate"),
                            format_percent(rate)
                        )),
                );
            }
            let mut rates = h_flex().w_full().gap_3();
            for (label, rate) in [
                (t!("cost_input").to_string(), model.input_per_million),
                (t!("cost_cache").to_string(), model.cache_per_million),
                (t!("cost_output").to_string(), model.output_per_million),
            ] {
                rates = rates.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("{label} {}", format_rate(rate))),
                );
            }
            if let Some(multiplier) = model.multiplier {
                rates = rates.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("×{multiplier}")),
                );
            }
            card = card.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .rounded(px(6.))
                    .bg(cx.theme().muted)
                    .px_2()
                    .py_1()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().foreground)
                                    .child(model.backend_model.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .child(format_cost(model.estimated_cost)),
                            ),
                    )
                    .child(tokens)
                    .child(rates),
            );
        }
        if cost.is_partial {
            card = card.child(
                div()
                    .text_size(px(10.))
                    .text_color(warning())
                    .child(t!("cost_partial").to_string()),
            );
        }
        card.into_any_element()
    }

    fn cost_row(
        &self,
        label: String,
        totals: &CostTotals,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = if primary {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .text_color(color)
                    .child(label),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(format_cost(totals.estimated_cost)),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format_rate(totals.effective_cost_per_million)),
            )
            .into_any_element()
    }

    fn sub_agent_section(
        &self,
        label: String,
        key: String,
        body: &str,
        preview: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.expanded.contains(&key);
        let target = key.clone();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .rounded(px(6.))
            .bg(cx.theme().background)
            .p_2()
            .child(
                h_flex()
                    .id(SharedString::from(key.clone()))
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(label.to_uppercase()),
                    )
                    .when(!open && !preview.is_empty(), |view| {
                        view.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(preview.to_string()),
                        )
                    })
                    .when(open || preview.is_empty(), |view| {
                        view.child(div().flex_1())
                    })
                    .child(font_icon(
                        if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        },
                        12.,
                        muted,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx))),
            )
            .when(open, |view| {
                view.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .pt_1()
                        .child(self.markdown_view(&key)),
                )
            })
            .when(!open && !body.is_empty() && preview.is_empty(), |view| {
                view.child(div().text_size(px(12.)).text_color(muted).child(" "))
            })
            .into_any_element()
    }

    fn snapshot_row(&self, snapshot: &SnapshotView, cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&snapshot.key);
        let target = snapshot.key.clone();
        let muted = cx.theme().muted_foreground;
        let mut body = v_flex()
            .w_full()
            .min_w_0()
            .mb_2()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .overflow_hidden()
            .child(
                h_flex()
                    .id(SharedString::from(snapshot.key.clone()))
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().background))
                    .child(font_icon("compress", 14., accent()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(snapshot.title.clone()),
                    )
                    .when(!snapshot.preview.is_empty(), |view| {
                        view.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(snapshot.preview.clone()),
                        )
                    })
                    .when(snapshot.preview.is_empty(), |view| {
                        view.child(div().flex_1())
                    })
                    .child(font_icon(
                        if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        },
                        14.,
                        muted,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx))),
            );
        if open {
            body = body.child(
                div()
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(self.markdown_view(&snapshot.body_key)),
            );
        }
        body.into_any_element()
    }

    fn finish_task_row(&self, view: &FinishTaskView, cx: &mut Context<Self>) -> AnyElement {
        let (icon, color) = if view.failed {
            ("check-x", danger())
        } else {
            ("check-circle", success())
        };
        let open = self.expanded.contains(&view.cost_key);
        let mut label = h_flex()
            .id(SharedString::from(view.key.clone()))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .pl_2()
            .child(font_icon(icon, 15., color))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(view.label.clone()),
            );
        if view.cost.is_some() {
            let target = view.cost_key.clone();
            label = label
                .child(font_icon(
                    if open {
                        "double-arrow-up"
                    } else {
                        "double-arrow-down"
                    },
                    12.,
                    cx.theme().muted_foreground,
                ))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx)));
        }
        let mut body = v_flex().w_full().min_w_0().gap_2().child(label);
        if open {
            if let Some(cost) = &view.cost {
                body = body.child(self.cost_card(cost, cx));
            }
        }
        body.into_any_element()
    }

    fn alert_row(&self, view: &AlertView, cx: &mut Context<Self>) -> AnyElement {
        let is_warning = view.kind == AlertKind::Warning;
        let (icon, color) = if is_warning {
            ("warning", warning())
        } else {
            ("error", danger())
        };
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(color.opacity(0.45))
            .bg(color.opacity(0.08))
            .px_4()
            .py_3()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(font_icon(icon, 18., color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(view.title.clone()),
                    ),
            )
            .when(!view.body.is_empty(), |alert| {
                alert.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(cx.theme().foreground)
                        .child(plain_text(&view.body, cx.theme().foreground)),
                )
            })
            .into_any_element()
    }

    fn divider_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .items_center()
            .gap_3()
            .py_3()
            .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("clear_context_divider").to_string()),
            )
            .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
            .into_any_element()
    }

    fn row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        match row {
            Row::User(view) => self.user_row(view, cx),
            Row::Assistant(view) => self.assistant_row(view, cx),
            Row::Activity(group) => self.activity_row(group, cx),
            Row::Tool(tool) => self.tool_row(tool, cx),
            Row::SubAgent(card) => self.sub_agent_row(card, cx),
            Row::FinishTask(view) => self.finish_task_row(view, cx),
            Row::Snapshot(snapshot) => self.snapshot_row(snapshot, cx),
            Row::Alert(alert) => self.alert_row(alert, cx),
            Row::Divider => self.divider_row(cx),
        }
    }

    fn status_color_for(&self, status: &str) -> Rgba {
        match status.to_lowercase().as_str() {
            "executing" | "running" | "pending" | "waiting" => accent(),
            "completed" => success(),
            "failed" | "error" => danger(),
            _ => rgb(0x848586),
        }
    }

    fn session_row(&self, index: usize, session: &Session, cx: &mut Context<Self>) -> AnyElement {
        let active = self.selected.as_deref() == Some(session.id.as_str());
        let id = session.id.clone();
        let status_color = self.status_color_for(&session.status);
        let title = if session.title.trim().is_empty() {
            t!("untitled").to_string()
        } else {
            session.title.clone()
        };
        h_flex()
            .id(("session", index))
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .rounded(px(6.))
            .cursor_pointer()
            .when(active, |view| view.bg(accent().opacity(0.14)))
            .when(!active, |view| {
                view.hover(|style| style.bg(cx.theme().muted))
            })
            .child(
                div()
                    .flex_none()
                    .w(px(6.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(status_color),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(13.))
                            .when(active, |view| {
                                view.text_color(accent()).font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(!active, |view| view.text_color(cx.theme().foreground))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(session.status.clone()),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
            .into_any_element()
    }
}

fn truncate_text(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push_str("...");
    }
    result
}

impl Render for Observer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((offset, previous_max)) = self.prepend_anchor.take() {
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if !this.follow {
                        let max = this.scroll.max_offset();
                        this.scroll
                            .set_offset(point(offset.x, offset.y - (max.y - previous_max.y)));
                        cx.notify();
                    }
                });
            });
        }
        if self.follow && self.scroll_pending {
            self.scroll_pending = false;
            let entity = cx.entity().downgrade();
            // Resolve against new layout bounds, and cancel if the user started reading.
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |this, _| {
                    if this.follow {
                        this.scroll.scroll_to_bottom();
                    }
                });
            });
        }
        // Retain the polling task for the lifetime of this window only.
        let _ = &self.task;

        let border = cx.theme().border;
        let surface = cx.theme().muted;
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;

        let mut sidebar = v_flex()
            .id("sessions")
            .w(px(260.))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .gap_1()
            .p_3()
            .border_r_1()
            .border_color(border)
            .bg(background)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .pb_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .child(t!("sessions").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(self.sessions.len().to_string()),
                    ),
            );
        for (index, session) in self.sessions.iter().enumerate() {
            sidebar = sidebar.child(self.session_row(index, session, cx));
        }
        if self.sessions.is_empty() {
            sidebar = sidebar.child(
                div()
                    .pt_2()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(t!("no_sessions").to_string()),
            );
        }

        let mut content = v_flex()
            .id("history")
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_3()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                // Layout and scroll_to_bottom notifications are not reading intent.
                // Only an upward user wheel gesture suspends tail following.
                if this.selected.is_some()
                    && this
                        .model
                        .pause_for_scroll(event.delta.pixel_delta(px(16.)).y.into())
                {
                    this.follow = false;
                    cx.notify();
                }
            }));
        if self.selected.is_none() {
            content = content.child(
                div()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(t!("select").to_string()),
            );
        }
        if self.model.hidden > 0 {
            let before = self.model.before.clone();
            content = content.child(
                h_flex()
                    .id("earlier")
                    .w_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .mb_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_dashed()
                    .border_color(border)
                    .bg(background)
                    .cursor_pointer()
                    .hover(|style| style.bg(surface).border_color(accent().opacity(0.45)))
                    .child(font_icon("double-arrow-down", 12., muted))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(t!("earlier").to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let (Some(id), Some(previous)) = (&this.selected, before.clone()) {
                            let _ = this.worker.commands.try_send(Command::Older {
                                generation: this.model.generation,
                                id: id.clone(),
                                before: previous,
                            });
                            this.follow = false;
                            cx.notify();
                        }
                    })),
            );
        }
        for row in &self.rows {
            // Vue separates transcript entries with the medium spacing token.
            content = content.child(
                div()
                    .w_full()
                    .min_w_0()
                    .mb(px(20.))
                    .child(self.row(row, cx)),
            );
        }
        if self.model.live_overflow {
            content = content.child(
                div()
                    .px_4()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(t!("live_overflow").to_string()),
            );
        }
        if self.model.compressing {
            let seconds = self
                .compression_started
                .map(|started| started.elapsed().as_secs())
                .unwrap_or(0);
            let hint = self
                .model
                .compression_message
                .trim_end_matches(['.', '…'])
                .trim()
                .to_owned();
            content = content.child(
                h_flex().w_full().justify_center().py_3().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .rounded_full()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .child(font_icon("loading", 14., accent()))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(format!("{hint} {seconds}s...")),
                        ),
                ),
            );
        }

        let mut header = h_flex()
            .w_full()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(border);
        header = header
            .child(
                div()
                    .flex_none()
                    .w(px(6.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(if self.connected { success() } else { danger() }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(format!(
                        "{} · {}",
                        if self.connected {
                            t!("connected")
                        } else {
                            t!("connecting")
                        },
                        self.model.state
                    )),
            );
        if !self.follow {
            header = header.child(
                Button::new("follow")
                    .ghost()
                    .compact()
                    .label(t!("follow").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.resume_follow();
                        this.follow = true;
                        this.scroll_pending = true;
                        this.refresh_markdown(cx);
                        cx.notify();
                    })),
            );
        }

        h_flex()
            .size_full()
            .text_size(px(14.))
            .bg(background)
            .text_color(foreground)
            .child(sidebar)
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(header)
                    .child(content),
            )
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    let worker = Worker::start(options.clone())?;
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            if let Err(error) = cx
                .text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(icons::FONT)])
            {
                eprintln!("{}: {error}", t!("font_error"));
                cx.quit();
                return;
            }
            let window_options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(t!("title").to_string().into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            if gpui_kit::open_window(window_options, cx, move |_, cx| {
                cx.new(|cx| Observer::new(worker, options, cx))
            })
            .is_err()
            {
                eprintln!("{}", t!("window_error"));
                cx.quit();
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
