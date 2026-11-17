//! Native read-only rendering of the formal workflow message projection.
use chatspeed_gpui_observer::{
    icons,
    metrics::Metrics,
    model::Model,
    presentation::{self, message_text},
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

fn font_icon(name: &str) -> impl IntoElement {
    div()
        .flex_none()
        .w_4()
        .font_family(icons::FAMILY)
        .text_size(px(16.))
        .child(icons::glyph(name))
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
    markdown: BTreeMap<String, (String, Entity<TextViewState>)>,
    expanded: BTreeSet<String>,
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
            markdown: BTreeMap::new(),
            expanded: BTreeSet::new(),
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
        self.follow = true;
        self.switched = Some(Instant::now());
        self.scroll = ScrollHandle::new();
        self.prepend_anchor = None;
        self.deferred_update = None;
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
        if dirty {
            if self.follow {
                self.scroll_pending = true;
            }
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

    fn display_rows(&self) -> Vec<DisplayRow> {
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
        presentation::project_messages(messages)
    }

    fn refresh_markdown(&mut self, cx: &mut Context<Self>) {
        let start = Instant::now();
        let mut rows = Vec::new();
        for projected in self.display_rows() {
            let messages = match projected {
                DisplayRow::Message(message) => vec![message],
                DisplayRow::Activity { key, items } => {
                    if !self.expanded.contains(&key) {
                        continue;
                    }
                    items
                }
            };
            for mut message in messages {
                let key = presentation::message_key(&message);
                if let Some(reasoning) = message.reasoning.take() {
                    // Group expansion reveals thoughts directly, without a second disclosure.
                    rows.push((
                        format!("thought-{key}"),
                        presentation::clean_text(&reasoning),
                    ));
                }
                let text = if presentation::tool_name(&message.metadata) == "complete_workflow" {
                    presentation::completion_summary(&message)
                } else if presentation::is_tool_message(&message) {
                    if !self.expanded.contains(&format!("detail-{key}")) {
                        continue;
                    }
                    message_text(&message)
                } else {
                    message_text(&message)
                };
                if !text.is_empty() {
                    rows.push((key, text));
                }
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

    fn activity_group(&self, key: String, items: &[Message], cx: &mut Context<Self>) -> AnyElement {
        let open = self.expanded.contains(&key);
        let target = key.clone();
        let summary = presentation::activity_summary(items);
        let thought_count = items
            .iter()
            .filter(|item| presentation::think_only(item))
            .count();
        let mut body = v_flex().w_full().min_w_0().items_start().gap_2().child(
            h_flex()
                .id(SharedString::from(key))
                .w_full()
                .min_w_0()
                .justify_start()
                .items_start()
                .gap_2()
                .py_1()
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().muted))
                .child(font_icon(if open { "caret-down" } else { "caret-right" }))
                .child(font_icon(icons::activity_icon(items)))
                .child(div().flex_1().min_w_0().child(format!(
                    "{}{}",
                    summary,
                    if thought_count > 0 {
                        format!(" · {} x{thought_count}", t!("reasoning"))
                    } else {
                        String::new()
                    }
                )))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&target) {
                        this.expanded.insert(target.clone());
                    }
                    this.refresh_markdown(cx);
                    cx.notify();
                })),
        );
        if open {
            let mut details = v_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .pl_12()
                .border_l_1()
                .border_color(cx.theme().border);
            for item in items {
                if item.reasoning.is_some() {
                    details = details.child(self.disclosure(
                        format!("thought-{}", presentation::message_key(item)),
                        t!("reasoning").to_string(),
                        cx,
                    ));
                }
                if !presentation::think_only(item) {
                    details = details.child(self.tool_item(item, cx));
                }
            }
            body = body.child(details);
        }
        body.into_any_element()
    }

    fn tool_item(&self, message: &Message, cx: &mut Context<Self>) -> AnyElement {
        let key = presentation::message_key(&message);
        let target = format!("detail-{key}");
        let open = self.expanded.contains(&target);
        let label = presentation::tool_label(&message.metadata);
        let mut body = v_flex().w_full().min_w_0().items_start().gap_2().child(
            h_flex()
                .id(SharedString::from(target.clone()))
                .w_full()
                .min_w_0()
                .justify_start()
                .items_start()
                .gap_2()
                .py_1()
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().muted))
                .child(font_icon(if open { "caret-down" } else { "caret-right" }))
                .child(font_icon(icons::tool_icon(presentation::tool_name(
                    &message.metadata,
                ))))
                .child(div().flex_1().min_w_0().child(label.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&target) {
                        this.expanded.insert(target.clone());
                    }
                    this.refresh_markdown(cx);
                    cx.notify();
                })),
        );
        if !open {
            let status = presentation::tool_status(&message.metadata);
            let status_label = match status {
                "running" => t!("running").to_string(),
                "failed" => t!("failed").to_string(),
                "completed" => t!("completed").to_string(),
                _ => String::new(),
            };
            if !status_label.is_empty() {
                body = body.child(
                    div()
                        .pl_12()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status_label),
                );
            }
            let summary = presentation::tool_result_summary(message);
            if !summary.is_empty() {
                body = body.child(
                    div()
                        .pl_12()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                );
            }
        }
        if open {
            body = body.child(self.message_card(&key, "tool", label, &message.metadata, cx));
        }
        body.into_any_element()
    }

    fn disclosure(&self, key: String, label: String, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.expanded.contains(&key);
        let target = key.clone();
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .id(SharedString::from(key.clone()))
                    .w_full()
                    .min_w_0()
                    .justify_start()
                    .items_start()
                    .gap_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .child(font_icon(if expanded {
                        "caret-down"
                    } else {
                        "caret-right"
                    }))
                    .child(font_icon("reasoning"))
                    .child(div().flex_1().min_w_0().child(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&target) {
                            this.expanded.insert(target.clone());
                        }
                        this.refresh_markdown(cx);
                        cx.notify();
                    })),
            )
            .when(expanded, |view| {
                if let Some((_, state)) = self.markdown.get(&key) {
                    view.child(
                        div()
                            .pl_12()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(TextView::new(state).selectable(true)),
                    )
                } else {
                    view
                }
            })
            .into_any_element()
    }

    fn message_card(
        &self,
        key: &str,
        role: &str,
        title: String,
        metadata: &serde_json::Value,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        let is_user = role == "user";
        let is_tool = role == "tool";
        let icon = if is_user {
            "talk"
        } else if is_tool {
            icons::tool_icon(presentation::tool_name(metadata))
        } else {
            "assistant"
        };
        let accent = if is_tool {
            theme.primary
        } else {
            theme.muted_foreground
        };
        let mut body = v_flex().w_full().min_w_0().gap_2();
        let metadata = Some(metadata);
        let title = metadata
            .and_then(|value| {
                value["tool_name"]
                    .as_str()
                    .or_else(|| value["tool_call"]["function"]["name"].as_str())
            })
            .map(str::to_owned)
            .unwrap_or(title);
        if let Some(lines) = metadata.and_then(chatspeed_gpui_observer::presentation::edit_lines) {
            let mut diff = v_flex().w_full().font_family("Liberation Mono").text_sm();
            let mut old_line = 0usize;
            let mut new_line = 0usize;
            for (added, line) in lines {
                if added {
                    new_line += 1;
                } else {
                    old_line += 1;
                }
                let color = if added { rgb(0x41b555) } else { rgb(0xf56c6c) };
                diff = diff.child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .px_3()
                        .py_1()
                        .bg(color.opacity(0.12))
                        .text_color(color)
                        .child(div().w(px(36.)).text_xs().child(if added {
                            new_line.to_string()
                        } else {
                            old_line.to_string()
                        }))
                        .child(
                            div()
                                .flex_1()
                                .child(format!("{} {line}", if added { "+" } else { "−" })),
                        ),
                );
            }
            body = body.child(diff);
        } else if let Some((_, state)) = self.markdown.get(key) {
            body = body.child(TextView::new(state).selectable(true));
        }
        if is_tool {
            return div()
                .w_full()
                .min_w_0()
                .pl_12()
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .p_3()
                        .child(body),
                )
                .into_any_element();
        }
        h_flex()
            .w_full()
            .items_start()
            .gap_3()
            .py_3()
            .child(div().pt_1().text_color(accent).child(font_icon(icon)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(h_flex().gap_2().text_xs().text_color(accent).child(title))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .rounded(px(if is_user { 16. } else { 12. }))
                            .when(is_user, |view| {
                                view.bg(theme.muted)
                                    .p_4()
                                    .border_1()
                                    .border_color(theme.border)
                            })
                            .when(is_tool, |view| view.bg(theme.muted).p_2())
                            .child(body),
                    ),
            )
            .into_any_element()
    }
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
        let mut sessions = v_flex()
            .id("sessions")
            .w(px(260.))
            .h_full()
            .overflow_y_scroll()
            .gap_2()
            .p_3()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(t!("sessions").to_string());
        for (index, session) in self.sessions.iter().enumerate() {
            let id = session.id.clone();
            sessions = sessions.child(
                Button::new(("session", index))
                    .ghost()
                    .label(format!("{} · {}", session.title, session.status))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx))),
            );
        }
        let mut content = v_flex()
            .id("history")
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_4()
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
            content = content.child(t!("select").to_string());
        }
        if self.model.hidden > 0 {
            content = content.child(
                Button::new("earlier")
                    .link()
                    .label(t!("earlier").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let (Some(id), Some(before)) = (&this.selected, &this.model.before) {
                            let _ = this.worker.commands.try_send(Command::Older {
                                generation: this.model.generation,
                                id: id.clone(),
                                before: before.clone(),
                            });
                            this.follow = false;
                            cx.notify();
                        }
                    })),
            );
        }
        for row in self.display_rows() {
            let mut row_view = v_flex().w_full().min_w_0().gap_2();
            match row {
                DisplayRow::Activity { key, items } => {
                    row_view = row_view.child(self.activity_group(key, &items, cx));
                }
                DisplayRow::Message(message) => {
                    let key = presentation::message_key(&message);
                    if presentation::tool_name(&message.metadata) == "complete_workflow" {
                        if let Some((_, state)) = self.markdown.get(&key) {
                            row_view = row_view
                                .child(div().w_full().child(TextView::new(state).selectable(true)));
                        }
                        let status = presentation::tool_status(&message.metadata);
                        let failed = status == "failed";
                        let finish_label = if failed {
                            t!("failed").to_string()
                        } else if status == "running" {
                            t!("running").to_string()
                        } else {
                            t!("finish_task").to_string()
                        };
                        row_view = row_view.child(
                            div()
                                .text_sm()
                                .text_color(if failed {
                                    rgb(0xf56c6c).into()
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(finish_label),
                        );
                    } else if presentation::is_tool_message(&message) {
                        row_view = row_view.child(self.tool_item(&message, cx));
                    } else {
                        if message.reasoning.is_some() {
                            row_view = row_view.child(self.disclosure(
                                format!("thought-{key}"),
                                t!("reasoning").to_string(),
                                cx,
                            ));
                        }
                        if self.markdown.contains_key(&key) {
                            let label = if message.role == "user" {
                                t!("user")
                            } else {
                                t!("assistant")
                            };
                            row_view = row_view.child(self.message_card(
                                &key,
                                &message.role,
                                label.to_string(),
                                &message.metadata,
                                cx,
                            ));
                        }
                    }
                }
            }
            content = content.child(row_view);
        }
        h_flex()
            .size_full()
            .text_size(px(14.))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sessions)
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {}",
                                        if self.connected {
                                            t!("connected")
                                        } else {
                                            t!("connecting")
                                        },
                                        self.model.state
                                    )),
                            )
                            .when(!self.follow, |view| {
                                view.child(
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
                                )
                            }),
                    )
                    .when(self.model.live_overflow, |view| {
                        view.child(
                            div()
                                .px_4()
                                .text_xs()
                                .child(t!("live_overflow").to_string()),
                        )
                    })
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
