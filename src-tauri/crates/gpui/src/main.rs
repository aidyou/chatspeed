//! Native workflow window: titlebar, compact sidebar, composer and transcript.
//!
//! Visual rules mirror `src/views/Workflow.vue`, `src/components/window/Titlebar.vue`,
//! `src/components/workflow/WorkflowMessageList.vue` and
//! `src/styles/workflow/messages.scss`: a CLI-style tool transcript, grouped
//! tool activity, collapsible snapshots and delegated-task cards.

use crate::ui::{
    toast::{Toast, ToastKind},
    Icon,
};
use chatspeed_gpui::{
    chimes::{self, Chime, DecisionKind, PendingDecision, Reminder},
    icons,
    metrics::Metrics,
    model::Model,
    prefs::{self, normalize_locale, primary_preset, DEFAULT_PRIMARY},
    presentation::{
        self, ActivityItem, ActivityView, AssistantView, ChoiceGroupView, CostView, FinishTaskView,
        Row, SnapshotView, SubAgentView, ToolView, UserView,
    },
    types::*,
    worker::Worker,
};
use clap::Parser;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        h_flex,
        input::{InputEvent, InputState, Textarea, TextareaState},
        text::{TextView, TextViewState},
        theme::{Theme, ThemeMode},
        v_flex, ActiveTheme,
    },
    *,
};
use rust_i18n::t;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

rust_i18n::i18n!("locales", fallback = "en");

mod sound_player;
mod ui;

/// ChatSpeed's default primary preset. `primary_color` in the runtime config
/// replaces it through [`set_primary`].
static PRIMARY: AtomicU32 = AtomicU32::new(DEFAULT_PRIMARY);
const SUCCESS: u32 = 0x41b555;
const DANGER: u32 = 0xf56c6c;
const WARNING: u32 = 0xffc107;
const INFO: u32 = 0x46a2ed;
const VIOLET: u32 = 0x8b5cf6;
const MONO_FONT: &str = "Liberation Mono";

/// Geometry of the workflow window in `tauri.conf.json`: the native window opens
/// at the same size and is resized to the remembered size once settings arrive.
const WORKFLOW_WINDOW_SIZE: Size<Pixels> = size(px(1024.), px(650.));
/// Corner radius of the client-decorated window, matching
/// `--cs-border-radius-lg` (16px) in `src/style/element/css-vars.css`.
const WINDOW_RADIUS: f32 = 16.;
const WORKFLOW_WINDOW_MIN_SIZE: Size<Pixels> = size(px(800.), px(400.));

/// Markdown bold is a per-run font weight, and GPUI has no synthetic bold: it
/// only looks different when the family ships a bold face. Keep the themed
/// family when it does, otherwise move to one that does so summaries and
/// headings stay readable.
fn ensure_bold_capable_font(cx: &mut App) {
    let family = Theme::global(cx).font_family.clone();
    if has_distinct_bold_face(cx, &family) {
        return;
    }
    for candidate in [
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Source Han Sans SC",
        "Noto Sans CJK SC",
        "WenQuanYi Micro Hei",
    ] {
        if has_distinct_bold_face(cx, candidate) {
            Theme::update(cx, |theme| theme.font_family = candidate.into());
            eprintln!("{}: {family} -> {candidate}", t!("font_bold_fallback"));
            return;
        }
    }
}

fn has_distinct_bold_face(cx: &App, family: &str) -> bool {
    let regular = cx.text_system().resolve_font(&font(family));
    let bold = cx.text_system().resolve_font(&font(family).bold());
    regular != bold
}

fn set_primary(name: &str) {
    PRIMARY.store(primary_preset(name), Ordering::Relaxed);
}

fn accent() -> Rgba {
    rgb(PRIMARY.load(Ordering::Relaxed))
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

/// Stable identity for a code block, used for the copy confirmation.
fn code_key(code: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    hasher.finish()
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
        .child(Icon::new(icon).size(14.).color(color))
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

/// One numbered `ask_user` option the user can pick while the runtime waits.
fn choice_option_live(
    label: String,
    selected: bool,
    id: SharedString,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let (icon, color) = if selected {
        ("check-circle", Hsla::from(accent()))
    } else {
        ("uncheck", cx.theme().muted_foreground)
    };
    h_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px(px(4.))
        .py(px(2.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().muted))
        .child(Icon::new(icon).size(14.).color(color))
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
        .on_click(on_click)
        .into_any_element()
}

/// One file the user attached to the next message.
#[derive(Clone)]
struct Attachment {
    path: PathBuf,
    name: String,
}

/// One answer this window submitted, kept locally until the runtime echoes it.
#[derive(Clone)]
struct LocalAskUserAnswer {
    selected: Option<usize>,
    other: bool,
    supplement: String,
}

/// Configuration dialogs opened from the composer's quick actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuickPanel {
    Models,
    Skills,
    Mcp,
    Tools,
    Personality,
}

/// Pending skill changes; the panel edits this before the command lands.
#[derive(Clone)]
struct SkillsDraft {
    enabled: bool,
    selected: BTreeSet<String>,
}

impl SkillsDraft {
    fn from_config(config: &Value) -> Self {
        let enabled = config["skillEnabled"]
            .as_bool()
            .or_else(|| config["skill_enabled"].as_bool())
            .unwrap_or(false);
        let selected = config["selectedSkills"]
            .as_array()
            .or_else(|| config["selected_skills"].as_array())
            .map(|skills| {
                skills
                    .iter()
                    .filter_map(|skill| skill.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self { enabled, selected }
    }
}

/// Pending tool/MCP selections; they are written to the agent config as sets.
#[derive(Clone, Default)]
struct ToolDraft {
    available: BTreeSet<String>,
    auto_approve: BTreeSet<String>,
    mcp_available: BTreeSet<String>,
    mcp_auto_approve: BTreeSet<String>,
    mcp_auto_expand: BTreeSet<String>,
}

impl ToolDraft {
    fn from_config(config: &Value) -> Self {
        let mcp = &config["mcpTools"];
        Self {
            available: string_set(&config["availableTools"]),
            auto_approve: string_set(&config["autoApprove"]),
            mcp_available: string_set(&mcp["available"]),
            // `McpToolConfig` keeps its fields snake_case inside camelCase agents.
            mcp_auto_approve: string_set(
                &mcp.get("auto_approve")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .or_else(|| mcp.get("autoApprove").cloned())
                    .unwrap_or(Value::Null),
            ),
            mcp_auto_expand: string_set(
                &mcp.get("auto_expand")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .or_else(|| mcp.get("autoExpand").cloned())
                    .unwrap_or(Value::Null),
            ),
        }
    }
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn is_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
            )
        })
        .unwrap_or(false)
}

/// The context note the runtime receives with the message.
fn attachment_context(attachments: &[Attachment]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    let mut note = String::from("The user attached these image files:\n");
    for attachment in attachments {
        note.push_str(&format!(
            "- {}: {}\n",
            attachment.name,
            attachment.path.to_string_lossy()
        ));
    }
    Some(note)
}

/// Attachment metadata, in the shape the transcript renders.
fn attachment_metadata(attachments: &[Attachment]) -> Option<Value> {
    if attachments.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "attachments": attachments
            .iter()
            .map(|attachment| serde_json::json!({
                "type": "image",
                "name": attachment.name,
                "path": attachment.path.to_string_lossy(),
            }))
            .collect::<Vec<_>>(),
    }))
}

/// Scroll position, tail following and the update deferred during a history prepend.
struct TimelineScroll {
    follow: bool,
    scroll_pending: bool,
    scroll: ScrollHandle,
    scrollbar_input: ui::history_scrollbar::ScrollbarInput,
    prepend_anchor: Option<(Point<Pixels>, Point<Pixels>)>,
    deferred_update: Option<Update>,
}

struct Observer {
    worker: Worker,
    options: Options,
    terminal: ui::terminal::Panel,
    model: Model,
    sessions: Vec<Session>,
    automations: Option<Result<Vec<AutomationSummary>, String>>,
    automation_history: Option<(String, Option<Result<Vec<AutomationRun>, String>>)>,
    pending_decisions: BTreeMap<String, Vec<PendingDecision>>,
    selected: Option<String>,
    connected: bool,
    timeline_scroll: TimelineScroll,
    rows: Vec<Row>,
    markdown: BTreeMap<String, (String, Entity<TextViewState>)>,
    expanded: BTreeSet<String>,
    compression_started: Option<Instant>,
    metrics: Metrics,
    switched: Option<Instant>,
    last_metrics: Instant,
    /// Last repaint that refreshed the live status line's elapsed seconds.
    last_tick: Instant,
    /// `theme` runtime setting: `system`, `light`, or `dark`.
    theme_setting: String,
    /// Last appearance the theme was resolved against, to catch system switches.
    appearance_dark: bool,
    applied_primary: u32,
    palette_applied: bool,
    /// Size to apply on the next frame, from the remembered workflow geometry.
    geometry_ready: bool,
    geometry_restore_pending: bool,
    pending_window_size: Option<Size<Pixels>>,
    #[cfg(target_os = "windows")]
    pending_window_position: Option<(i32, i32, String)>,
    /// Remembered size already applied, so the periodic config poll cannot
    /// override a size the user picked in this window.
    applied_window_size: Option<Size<Pixels>>,
    #[cfg(target_os = "windows")]
    window_position_read: bool,
    copy_flash: Arc<Mutex<Option<CopyFlash>>>,
    /// Sidebar collapse state, matching the Vue compact sidebar.
    sidebar_collapsed: bool,
    sidebar_changed: bool,
    pending_settings: BTreeMap<(String, &'static str), (bool, Option<Instant>)>,
    models_saved_at: Option<Instant>,
    models_pending: Option<Value>,
    clear_pending: bool,
    send_with_shift: bool,
    /// Tab shown in the expanded sidebar, and the rail entry that selects it.
    sidebar_tab: ui::sidebar::SidebarTab,
    rail_tab: ui::sidebar::RailTab,
    /// Session search box and the primary-root filter beside it.
    session_search: Entity<InputState>,
    session_root_filter: Option<String>,
    /// Draggable width of the expanded panel, matching Vue's 200px floor.
    sidebar_width: f32,
    /// Pointer that started dragging the sidebar edge, while it is held.
    sidebar_drag: Option<SidebarDrag>,
    show_menu_button: bool,
    sound_enabled: BTreeSet<Chime>,
    sound_pending: BTreeMap<Chime, bool>,
    sound_saved_at: Option<Instant>,
    sound_player: Option<sound_player::SoundPlayer>,
    /// Always-on-top request from the titlebar.
    pinned: bool,
    /// Expanded folders of the authorized-path tree, with the children the
    /// native window listed for them and any failure worth showing.
    expanded_dirs: BTreeSet<String>,
    dir_entries: BTreeMap<String, Vec<DirEntry>>,
    dir_errors: BTreeMap<String, String>,
    dir_pending: BTreeSet<String>,
    file_editor: ui::file_editor::FileEditor,
    /// The first authorized root opens once per selected workflow, matching
    /// `FileTree.vue`'s mount behaviour.
    files_auto_expanded: bool,
    /// A titlebar press that became a window move on the first pointer motion.
    title_drag: bool,
    /// Multi-line composer state; sends read it and clear it.
    composer: Entity<TextareaState>,
    drafts: BTreeMap<Option<String>, (String, Vec<Attachment>)>,
    restore_composer: Option<String>,
    show_today_cost: bool,
    today_cost: f64,
    rewind_pending: bool,
    automation_pending: bool,
    /// Whether the composer owns focus, mirrored from its input events.
    composer_focused: bool,
    /// Vue's fullscreen toggle: the box grows instead of the transcript.
    composer_expanded: bool,
    /// A new session is being created and started; further sends wait.
    start_pending: bool,
    start_draft: Option<(String, Vec<Attachment>)>,
    /// Transient action feedback shown below the titlebar.
    toast: Option<Toast>,
    /// Child transcripts loaded for expanded sub-agent cards, by task id.
    child_rows: BTreeMap<String, Vec<Row>>,
    active_child: Option<SubAgentView>,
    /// Selected option per `ask_user` group, keyed by `tool_call_id:group`.
    ask_user_choice: BTreeMap<String, usize>,
    /// Free text typed for an `ask_user` "Other" row, by the same key.
    approval_drafts: BTreeMap<String, Entity<TextareaState>>,
    ask_user_text: BTreeMap<String, Entity<TextareaState>>,
    /// Child transcripts requested but not loaded yet.
    child_epoch: u64,
    child_model: Model,
    child_connected: Option<bool>,
    child_pending: BTreeSet<String>,
    /// Agents, model providers and skills the composer selectors list.
    agents: Vec<serde_json::Value>,
    models: Vec<serde_json::Value>,
    skills: Vec<serde_json::Value>,
    /// Sandbox schemes the execution-mode popover can pick from.
    sandbox_schemes: Vec<serde_json::Value>,
    /// Runtime tools the tool/MCP configuration panels list.
    tools: Vec<serde_json::Value>,
    /// Proxy groups and aliases, used by the model selector for `id = 0` slots.
    proxy_targets: Value,
    /// Vue's planning-mode switch, applied through the workflow phase.
    planning_mode: bool,
    /// Optimistic sandbox choice; the workflow config confirms it afterwards.
    config_pending: BTreeSet<(String, &'static str)>,
    config_acks: BTreeMap<&'static str, (Instant, Value)>,
    /// Answers this window submitted before the runtime echoed them back.
    ask_user_answered: BTreeMap<String, Vec<LocalAskUserAnswer>>,
    /// Which configuration dialog the quick actions opened, if any.
    quick_panel: Option<QuickPanel>,
    /// Slot being edited in the model configuration dialog.
    model_config_slot: String,
    skills_draft: Option<SkillsDraft>,
    tool_draft: Option<ToolDraft>,
    /// Image files attached to the next message, shown above the box.
    attachments: Vec<Attachment>,
    /// The `@` query the file panel answered, so a stale list is never shown.
    suggestions: ui::composer::suggestions::State,
    _completion_keys: Subscription,
    file_results: Vec<serde_json::Value>,
    task: Task<()>,
}

/// A code block copied within the last few seconds, so its action can say so.
#[derive(Clone, Copy)]
struct CopyFlash {
    key: u64,
    at: Instant,
}

/// Sidebar edge drag: where the pointer went down and the width it started at.
#[derive(Clone, Copy)]
struct SidebarDrag {
    start_x: Pixels,
    start_width: f32,
}

/// Vue clamps the expanded panel between 200px and half the window.
const SIDEBAR_MIN_WIDTH: f32 = 200.;

const COPY_FLASH: Duration = Duration::from_secs(3);

impl Observer {
    fn new(worker: Worker, options: Options, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("composer_placeholder").to_string())
                .auto_grow(1, 10)
        });
        // Enter sends, like the Vue input; Shift+Enter keeps the newline the
        // textarea just inserted.
        cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, _window, cx| match event {
                InputEvent::Change => {
                    ui::composer::suggestions::refresh(this, cx);
                    cx.notify();
                }
                // The box paints its focus ring from this flag; querying the
                // window during render missed the change on the first frame.
                InputEvent::Focus => {
                    this.composer_focused = true;
                    cx.notify();
                }
                InputEvent::Blur => {
                    this.composer_focused = false;
                    cx.notify();
                }
                _ => {}
            },
        )
        .detach();
        cx.observe_in(&composer, window, |this, _, _, cx| {
            if ui::composer::suggestions::refresh(this, cx) {
                cx.notify();
            }
        })
        .detach();
        let owner = cx.entity().downgrade();
        let completion_keys = cx.intercept_keystrokes(move |event, window, cx| {
            let key = &event.keystroke;
            let modifiers = key.modifiers;
            if modifiers.control
                || modifiers.platform
                || modifiers.alt
                || modifiers.function
                || (modifiers.shift && key.key != "enter")
                || !matches!(key.key.as_str(), "enter" | "tab" | "up" | "down" | "escape")
            {
                return;
            }
            let _ = owner.update(cx, |this, cx| {
                let input = this.composer.clone();
                if !input.read(cx).focus_handle(cx).is_focused(window) {
                    return;
                }
                let composing = input.update(cx, |state, cx| {
                    state.marked_text_range(window, cx).is_some()
                });
                if composing {
                    return;
                }
                let consumed = ui::composer::suggestions::handle_key(this, &key.key, window, cx);
                let send =
                    !consumed && key.key == "enter" && modifiers.shift == this.send_with_shift;
                if send {
                    this.send_message(window, cx);
                }
                if consumed || send {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        });
        let session_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("search_sessions").to_string()));
        // The list filters while the query changes, so every edit repaints it.
        cx.subscribe_in(
            &session_search,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )
        .detach();
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| loop {
            executor.timer(Duration::from_millis(50)).await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        });
        cx.observe_window_bounds(window, |this, window, _| {
            if !this.geometry_ready || window.is_maximized() || window.is_fullscreen() {
                return;
            }
            let bounds = window.bounds();
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            if !(width.is_finite() && height.is_finite() && width > 0. && height > 0.) {
                return;
            }
            this.worker.save_geometry(ui::window_pin::geometry_values(window, width, height));
        }).detach();
        let observer = Self {
            worker,
            options,
            terminal: ui::terminal::Panel::new(cx),
            model: Model::default(),
            sessions: Vec::new(),
            automations: None,
            automation_history: None,
            pending_decisions: BTreeMap::new(),
            selected: None,
            connected: false,
            timeline_scroll: TimelineScroll {
                follow: true,
                scroll_pending: false,
                scroll: ScrollHandle::new(),
                scrollbar_input: Default::default(),
                prepend_anchor: None,
                deferred_update: None,
            },
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
            last_tick: Instant::now(),
            theme_setting: "system".into(),
            appearance_dark: false,
            applied_primary: DEFAULT_PRIMARY,
            palette_applied: false,
            geometry_ready: false,
            geometry_restore_pending: false,
            pending_window_size: None,
            #[cfg(target_os = "windows")]
            pending_window_position: None,
            applied_window_size: None,
            #[cfg(target_os = "windows")]
            window_position_read: false,
            copy_flash: Arc::new(Mutex::new(None)),
            sidebar_collapsed: false,
            sidebar_changed: false,
            pending_settings: BTreeMap::new(),
            models_saved_at: None,
            models_pending: None,
            clear_pending: false,
            send_with_shift: false,
            sidebar_tab: ui::sidebar::SidebarTab::Tasks,
            rail_tab: ui::sidebar::RailTab::Tasks,
            session_search,
            session_root_filter: None,
            sidebar_width: ui::SIDEBAR_WIDTH,
            sidebar_drag: None,
            show_menu_button: true,
            sound_enabled: BTreeSet::new(),
            sound_pending: BTreeMap::new(),
            sound_saved_at: None,
            sound_player: sound_player::SoundPlayer::start().ok(),
            pinned: false,
            expanded_dirs: BTreeSet::new(),
            dir_entries: BTreeMap::new(),
            dir_errors: BTreeMap::new(),
            dir_pending: BTreeSet::new(),
            file_editor: Default::default(),
            files_auto_expanded: false,
            title_drag: false,
            composer,
            drafts: BTreeMap::new(),
            restore_composer: None,
            show_today_cost: false,
            today_cost: 0.,
            rewind_pending: false,
            automation_pending: false,
            composer_focused: false,
            composer_expanded: false,
            start_pending: false,
            start_draft: None,
            toast: None,
            child_rows: BTreeMap::new(),
            active_child: None,
            ask_user_choice: BTreeMap::new(),
            approval_drafts: BTreeMap::new(),
            ask_user_text: BTreeMap::new(),
            child_epoch: 0,
            child_model: Model::default(),
            child_connected: None,
            child_pending: BTreeSet::new(),
            agents: Vec::new(),
            models: Vec::new(),
            skills: Vec::new(),
            sandbox_schemes: Vec::new(),
            tools: Vec::new(),
            proxy_targets: Value::Null,
            planning_mode: false,
            config_pending: BTreeSet::new(),
            config_acks: BTreeMap::new(),
            ask_user_answered: BTreeMap::new(),
            quick_panel: None,
            model_config_slot: "act".into(),
            skills_draft: None,
            tool_draft: None,
            attachments: Vec::new(),
            suggestions: Default::default(),
            _completion_keys: completion_keys,
            file_results: Vec::new(),
            task,
        };
        observer.request_catalogue();
        observer
    }

    /// Loads the agent, model, skill and sandbox lists the footer selectors use.
    fn request_catalogue(&self) {
        let _ = self.worker.commands.try_send(Command::Catalogue);
    }

    fn switch_draft(&mut self, next: Option<String>, cx: &App) {
        let text = self
            .restore_composer
            .take()
            .unwrap_or_else(|| self.composer.read(cx).value().to_string());
        let attachments = std::mem::take(&mut self.attachments);
        if !text.is_empty() || !attachments.is_empty() {
            self.drafts
                .insert(self.selected.clone(), (text, attachments));
        }
        let (text, attachments) = self.drafts.remove(&next).unwrap_or_default();
        self.selected = next;
        self.attachments = attachments;
        self.restore_composer = Some(text);
    }

    fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.restore_composer.take() {
            self.composer
                .update(cx, |state, cx| state.set_value(text, window, cx));
            self.suggestions.file_request = None;
            self.file_results.clear();
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
        if self.selected.as_deref() != Some(id.as_str()) {
            self.switch_draft(Some(id), cx);
        }
        self.model.select();
        self.models_saved_at = None;
        self.models_pending = None;
        self.clear_pending = false;
        self.worker.set_child(None);
        self.active_child = None;
        self.child_pending.clear();
        self.child_rows.clear();
        self.approval_drafts.clear();
        self.rewind_pending = false;
        self.markdown.clear();
        self.expanded.clear();
        self.rows.clear();
        self.ask_user_choice.clear();
        self.ask_user_text.clear();
        self.ask_user_answered.clear();
        self.quick_panel = None;
        self.config_acks.clear();
        self.planning_mode = false;
        self.skills_draft = None;
        self.tool_draft = None;
        self.timeline_scroll.follow = true;
        self.switched = Some(Instant::now());
        self.timeline_scroll.scroll = ScrollHandle::new();
        self.timeline_scroll.scrollbar_input = Default::default();
        self.timeline_scroll.prepend_anchor = None;
        self.timeline_scroll.deferred_update = None;
        self.compression_started = None;
        self.clear_dir_state();
        cx.notify();
    }

    fn reveal_latest(&mut self, cx: &mut Context<Self>) {
        self.timeline_scroll.follow = true;
        self.model.resume_follow();
        self.timeline_scroll.scroll_pending = true;
        cx.notify();
    }

    /// Directory listings belong to the selected workflow's roots.
    fn clear_dir_state(&mut self) {
        self.expanded_dirs.clear();
        self.dir_entries.clear();
        self.dir_errors.clear();
        self.dir_pending.clear();
        self.files_auto_expanded = false;
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.set_sidebar_collapsed(!self.sidebar_collapsed, cx);
    }

    fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.sidebar_collapsed == collapsed {
            return;
        }
        self.sidebar_collapsed = collapsed;
        self.sidebar_changed = true;
        if !self.worker.save_sidebar(collapsed) {
            self.show_toast(
                ToastKind::Warning,
                t!("sidebar_save_failed").into_owned(),
                cx,
            );
        }
        // Vue drops the root filter when the panel collapses.
        if self.sidebar_collapsed {
            self.session_root_filter = None;
        }
        cx.notify();
    }

    /// Rail selection also picks the list tab Vue shows next to it.
    fn select_rail_tab(&mut self, tab: ui::sidebar::RailTab, cx: &mut Context<Self>) {
        self.rail_tab = tab;
        if tab == ui::sidebar::RailTab::Automation {
            self.refresh_automations();
        }
        self.sidebar_tab = match tab {
            ui::sidebar::RailTab::Tasks => ui::sidebar::SidebarTab::Tasks,
            ui::sidebar::RailTab::Automation => ui::sidebar::SidebarTab::Automation,
        };
        cx.notify();
    }

    fn refresh_automations(&mut self) {
        if self.worker.commands.try_send(Command::ListAutomations).is_ok() {
            self.automations = None;
        }
    }

    fn select_sidebar_tab(&mut self, tab: ui::sidebar::SidebarTab, cx: &mut Context<Self>) {
        self.sidebar_tab = tab;
        cx.notify();
    }

    fn set_session_root_filter(&mut self, root: Option<String>, cx: &mut Context<Self>) {
        self.session_root_filter = root;
        cx.notify();
    }

    fn begin_sidebar_drag(&mut self, start_x: Pixels, cx: &mut Context<Self>) {
        self.sidebar_drag = Some(SidebarDrag {
            start_x,
            start_width: self.sidebar_width,
        });
        cx.notify();
    }

    /// The tab strip hides the list tab the rail did not select, so the panel
    /// never shows a tab that is not on screen.
    fn visible_sidebar_tab(&self) -> ui::sidebar::SidebarTab {
        match (self.sidebar_tab, self.rail_tab) {
            (ui::sidebar::SidebarTab::Automation, ui::sidebar::RailTab::Tasks) => {
                ui::sidebar::SidebarTab::Tasks
            }
            (ui::sidebar::SidebarTab::Tasks, ui::sidebar::RailTab::Automation) => {
                ui::sidebar::SidebarTab::Automation
            }
            (tab, _) => tab,
        }
    }

    /// Vue's titlebar path opens the authorized-paths tab, expanding first.
    fn open_files_tab(&mut self, cx: &mut Context<Self>) {
        self.set_sidebar_collapsed(false, cx);
        self.sidebar_tab = ui::sidebar::SidebarTab::Files;
        cx.notify();
    }

    /// Expands or collapses one authorized folder, loading its children the
    /// first time it opens.
    fn toggle_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if self.expanded_dirs.remove(&path) {
            cx.notify();
            return;
        }
        self.expanded_dirs.insert(path.clone());
        if !self.dir_entries.contains_key(&path) {
            self.load_dir(path, cx);
        }
        cx.notify();
    }

    fn load_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.dir_pending.insert(path.clone()) {
            return;
        }
        self.dir_errors.remove(&path);
        if self
            .worker
            .commands
            .try_send(Command::ListDir { path: path.clone() })
            .is_err()
        {
            self.dir_pending.remove(&path);
        }
        cx.notify();
    }

    /// Vue's FileTree opens the first authorized root when it mounts.
    fn auto_expand_first_root(&mut self, cx: &mut Context<Self>) {
        if self.files_auto_expanded {
            return;
        }
        let Some(root) = self.model.roots.first().cloned() else {
            return;
        };
        self.files_auto_expanded = true;
        self.expanded_dirs.insert(root.clone());
        self.load_dir(root, cx);
    }

    /// Pending decisions from all watched workflows, with a selected-session
    /// question fallback until the notification stream has hydrated.
    fn pending_approvals(&self) -> Vec<(PendingDecision, String)> {
        let mut pending = Vec::new();
        for (owner, entries) in &self.pending_decisions {
            let title = self.session_heading(owner);
            pending.extend(entries.iter().cloned().map(|entry| (entry, title.clone())));
        }
        if let Some(selected) = &self.selected {
            if chimes::waiting_for_user(&self.model.state, &self.model.wait_reason)
                && !pending.iter().any(|(entry, _)| {
                    entry.target == *selected && entry.kind == DecisionKind::Question
                })
            {
                pending.push((
                    PendingDecision {
                        target: selected.clone(),
                        kind: DecisionKind::Question,
                    },
                    self.session_heading(selected),
                ));
            }
        }
        pending.sort_by_key(|(entry, _)| self.selected.as_deref() != Some(entry.target.as_str()));
        pending
    }

    fn session_heading(&self, id: &str) -> String {
        self.sessions
            .iter()
            .find(|session| session.id == id)
            .map(Session::heading)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| t!("untitled").to_string())
    }

    fn open_pending_decision(&mut self, target: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(target.as_str()) {
            self.reveal_latest(cx);
        } else {
            self.select(target, cx);
        }
    }

    fn desktop_menu_action(
        &mut self,
        action: chatspeed_runtime_client::desktop_menu::Action,
        cx: &mut Context<Self>,
    ) {
        if self
            .worker
            .commands
            .try_send(Command::DesktopMenu(action))
            .is_err()
        {
            self.show_toast(
                ToastKind::Warning,
                t!("desktop_menu_unavailable").to_string(),
                cx,
            );
        }
    }

    fn sound_is_enabled(&self, chime: Chime) -> bool {
        self.sound_pending
            .get(&chime)
            .map_or_else(|| self.sound_enabled.contains(&chime), |muted| !muted)
    }

    fn set_sound_enabled(&mut self, chime: Chime, enabled: bool) {
        if enabled {
            self.sound_enabled.insert(chime);
        } else {
            self.sound_enabled.remove(&chime);
        }
    }

    fn toggle_sound(&mut self, chime: Chime, cx: &mut Context<Self>) {
        if self.sound_pending.contains_key(&chime) {
            return;
        }
        let muted = self.sound_is_enabled(chime);
        if self
            .worker
            .commands
            .try_send(Command::SetSound { chime, muted })
            .is_ok()
        {
            self.sound_pending.insert(chime, muted);
        } else {
            self.show_toast(ToastKind::Error, t!("sound_save_failed").to_string(), cx);
        }
        cx.notify();
    }

    fn can_rewind(&self) -> bool {
        self.selected.is_some()
            && self.model.can_rewind_tail
            && !self.workflow_running()
            && self.model.wait_reason.is_empty()
            && !self.rewind_pending
    }

    fn edit_session(&mut self, session: String, title: Option<String>, cx: &mut Context<Self>) -> bool {
        if !self.sessions.iter().any(|entry| entry.id == session) {
            return false;
        }
        if self.worker.commands.try_send(Command::EditSession { session, title }).is_ok() {
            return true;
        }
        self.show_toast(ToastKind::Error, t!("session_edit_failed").to_string(), cx);
        false
    }

    fn confirm_rewind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_rewind() {
            return;
        }
        let generation = self.model.generation;
        let answer = window.prompt(
            PromptLevel::Warning,
            &t!("delete_last_message"),
            Some(&t!("rewind_confirm")),
            &[
                PromptButton::cancel(t!("action_cancel").to_string()),
                PromptButton::new(t!("delete_last_message").to_string()),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await.ok() != Some(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if !this.model.accepts(generation) || !this.can_rewind() {
                    return;
                }
                let Some(session) = this.selected.clone() else {
                    return;
                };
                if this
                    .worker
                    .commands
                    .try_send(Command::Rewind {
                        generation,
                        session,
                    })
                    .is_ok()
                {
                    this.rewind_pending = true;
                    cx.notify();
                } else {
                    this.show_toast(ToastKind::Error, t!("rewind_unavailable").to_string(), cx);
                }
            });
        })
        .detach();
    }

    fn toggle_pin(&mut self, window: &Window, cx: &mut Context<Self>) {
        match ui::window_pin::set(window, !self.pinned) {
            Ok(()) => {
                self.pinned = !self.pinned;
                cx.notify();
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
    }

    fn show_toast(&mut self, kind: ToastKind, message: String, cx: &mut Context<Self>) {
        self.toast = Some(Toast::new(kind, message, Instant::now(), kind.duration()));
        cx.notify();
    }

    fn not_implemented(&mut self, feature: impl Into<String>, cx: &mut Context<Self>) {
        self.show_toast(
            ToastKind::Info,
            format!("{} · {}", feature.into(), t!("not_supported_native")),
            cx,
        );
    }

    /// Composer footer: connection state plus the workflow state the runtime reports.
    /// The selected workflow's effective agent config.
    ///
    /// The list route serializes it as JSON text and the snapshot route as an
    /// object, so both shapes are unwrapped here.
    fn agent_config(&self) -> serde_json::Value {
        let workflow = &self.model.workflow;
        let raw = workflow
            .get("agent_config")
            .or_else(|| workflow.get("agentConfig"));
        match raw {
            Some(serde_json::Value::String(text)) => {
                serde_json::from_str(text).unwrap_or_else(|_| serde_json::json!({}))
            }
            Some(value) => value.clone(),
            None => serde_json::json!({}),
        }
    }

    fn current_agent_id(&self) -> Option<String> {
        let workflow = &self.model.workflow;
        workflow["agent_id"]
            .as_str()
            .or_else(|| workflow["agentId"].as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    }

    fn current_agent(&self) -> Option<&serde_json::Value> {
        let id = self.current_agent_id()?;
        self.agents
            .iter()
            .find(|agent| agent["id"].as_str() == Some(id.as_str()))
    }

    /// The model slot the composer edits: Vue uses `plan` in planning mode.
    fn active_model_config(&self) -> serde_json::Value {
        let models = self.model_configs();
        let slots: [&str; 2] = if self.planning_mode {
            ["plan", "act"]
        } else {
            ["act", "plan"]
        };
        for key in slots {
            if models[key]["model"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
            {
                return models[key].clone();
            }
        }
        serde_json::json!({})
    }

    fn save_models(&mut self, configs: Value, cx: &mut Context<Self>) {
        if self.models_pending.is_some() || self.options.demo {
            return;
        }
        let Some(session) = self.selected.clone() else {
            return;
        };
        match self.worker.commands.try_send(Command::SetModelConfig {
            generation: self.model.generation,
            session,
            configs: configs.clone(),
        }) {
            Ok(()) => self.models_pending = Some(configs),
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
        cx.notify();
    }

    fn model_configs(&self) -> serde_json::Value {
        self.models_pending
            .clone()
            .unwrap_or_else(|| self.agent_config()["models"].clone())
    }

    /// Provider id of the active model: `0` means a ChatSpeed proxy alias.
    fn active_provider(&self) -> Option<&serde_json::Value> {
        let id = self.active_model_config()["id"].as_i64()?;
        self.models
            .iter()
            .find(|provider| provider["id"].as_i64() == Some(id))
    }

    /// The model name Vue shows on the footer pill.
    fn model_label(&self) -> String {
        let config = self.active_model_config();
        let Some(model) = config["model"].as_str().filter(|name| !name.is_empty()) else {
            return t!("model_placeholder").to_string();
        };
        let prefix = if self.planning_mode { "Plan/" } else { "" };
        if config["id"].as_i64().unwrap_or_default() == 0 {
            // Proxy slots carry `group@alias`; Vue shows `alias(group)`.
            return match model.split_once('@') {
                Some((group, alias)) => format!("{prefix}{alias}({group})"),
                None => format!("{prefix}{model}"),
            };
        }
        let name = self
            .active_provider()
            .and_then(|provider| {
                provider["models"]
                    .as_array()?
                    .iter()
                    .find(|entry| entry["id"].as_str() == Some(model))
                    .and_then(|entry| entry["name"].as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| model.to_owned());
        format!("{prefix}{name}")
    }

    /// The workflow phase drives Vue's planning-mode switch.
    fn sync_planning_mode(&mut self) {
        let phase = self.agent_config()["phase"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        self.planning_mode = self
            .selected
            .as_ref()
            .and_then(|id| self.pending_settings.get(&(id.clone(), "phase")))
            .map_or(phase == "planning", |(enabled, _)| *enabled);
    }

    fn submit_config(&mut self, session: &str, key: &'static str, command: Command, cx: &mut Context<Self>) -> bool {
        let entry = (session.to_owned(), key);
        if self.options.demo || self.selected.as_deref() != Some(session) || self.config_pending.contains(&entry) {
            return false;
        }
        if self.worker.commands.try_send(command).is_err() {
            return false;
        }
        self.config_pending.insert(entry);
        cx.notify();
        true
    }

    fn apply_config_ack(&mut self, key: &str, patch: &Value) {
        if key == "agent" {
            self.model.workflow["agent_id"] = patch["agent_id"].clone();
            self.model.workflow["agent_config"] = patch["agent_config"].clone();
        } else if let Some(fields) = patch.as_object() {
            let mut config = self.agent_config();
            for (field, value) in fields {
                config[field] = value.clone();
            }
            self.model.workflow["agent_config"] = config;
        }
    }

    fn sandbox_mode(&self) -> String {
        let config = self.agent_config();
        config["sandboxExecutionMode"]
            .as_str()
            .or_else(|| config["sandbox_execution_mode"].as_str())
            .unwrap_or("host_only")
            .to_owned()
    }

    fn sandbox_scheme_id(&self) -> Option<String> {
        let config = self.agent_config();
        config.get("sandboxSchemeId")
            .or_else(|| config.get("sandbox_scheme_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    }

    fn can_toggle_planning(&self) -> bool {
        if self.selected.is_none() {
            return true;
        }
        if self.model.workflow.is_null() || self.model.has_live_session || self.workflow_running() {
            return false;
        }
        self.can_switch_agent() || self.can_clear_context()
    }

    fn can_clear_context(&self) -> bool {
        self.selected.is_some() && self.model.can_clear_context()
    }

    fn toggle_planning(&mut self, cx: &mut Context<Self>) {
        if !self.can_toggle_planning() {
            return;
        }
        let enabled = !self.planning_mode;
        if let Some(session) = self.selected.clone() {
            let key = (session.clone(), "phase");
            if self.options.demo
                || self
                    .pending_settings
                    .get(&key)
                    .is_some_and(|(_, ack)| ack.is_none())
            {
                return;
            }
            if self
                .worker
                .commands
                .try_send(Command::SetPlanning { session, enabled })
                .is_err()
            {
                return;
            }
            self.pending_settings.insert(key, (enabled, None));
        }
        self.planning_mode = enabled;
        cx.notify();
    }

    /// Vue's `canSwitchCurrentWorkflowAgent`: a live session blocks switching,
    /// and so does a workflow that already ran, unless the last turn cleared the
    /// context frame.
    fn can_switch_agent(&self) -> bool {
        if self.selected.is_none() {
            return true;
        }
        if self.workflow_running() {
            return false;
        }
        let workflow = &self.model.workflow;
        let has_query = workflow["user_query"]
            .as_str()
            .or_else(|| workflow["userQuery"].as_str())
            .is_some_and(|query| !query.trim().is_empty());
        let has_messages = !self.model.messages.is_empty();
        let cleared = self
            .model
            .messages
            .values()
            .next_back()
            .and_then(|message| message.message_subtype.as_deref())
            == Some("manual_clear_context");
        (!has_query && !has_messages) || cleared
    }

    /// A boolean agent-config switch, accepting both serializations.
    fn agent_bool(&self, camel: &str, snake: &str, default: bool) -> bool {
        let config = self.agent_config();
        config[camel]
            .as_bool()
            .or_else(|| config[snake].as_bool())
            .unwrap_or(default)
    }

    fn auto_compress_enabled(&self) -> bool {
        self.runtime_preference("autoCompress", "auto_compress")
    }

    fn runtime_preference(&self, key: &'static str, snake: &str) -> bool {
        self.selected
            .as_ref()
            .and_then(|id| self.pending_settings.get(&(id.clone(), key)))
            .map(|(enabled, _)| *enabled)
            .unwrap_or_else(|| self.agent_bool(key, snake, false))
    }

    fn toggle_runtime_preference(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone() else {
            return;
        };
        let entry = (session.clone(), key);
        if self.options.demo
            || self
                .pending_settings
                .get(&entry)
                .is_some_and(|(_, ack)| ack.is_none())
        {
            return;
        }
        let (enabled, command) = match key {
            "autoCompress" => {
                let enabled = !self.auto_compress_enabled();
                (enabled, Command::SetAutoCompress { session, enabled })
            }
            "finalAudit" => {
                let enabled = !self.final_audit_enabled();
                (enabled, Command::SetFinalAudit { session, enabled })
            }
            _ => return,
        };
        if self.worker.commands.try_send(command).is_ok() {
            self.pending_settings.insert(entry, (enabled, None));
            cx.notify();
        }
    }

    fn final_audit_enabled(&self) -> bool {
        self.runtime_preference("finalAudit", "final_audit")
    }

    fn approval_level(&self) -> String {
        let config = self.agent_config();
        config["approvalLevel"]
            .as_str()
            .or_else(|| config["approval_level"].as_str())
            .unwrap_or("default")
            .to_owned()
    }

    /// Starts a new context frame for later turns, as Vue's clear-context does.
    fn clear_context_frame(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone() else {
            self.show_toast(
                ToastKind::Info,
                t!("clear_context_no_session").to_string(),
                cx,
            );
            cx.notify();
            return;
        };
        if self.clear_pending || self.options.demo || !self.can_clear_context() { return; }
        if self
            .worker
            .commands
            .try_send(Command::ClearContext { generation: self.model.generation, session })
            .is_err()
        {
            self.show_toast(ToastKind::Error, t!("clear_context_failed").to_string(), cx);
        } else {
            self.clear_pending = true;
        }
        cx.notify();
    }

    /// Runtime states that accept a stop and show the running affordance.
    fn workflow_running(&self) -> bool {
        matches!(
            self.model.state.as_str(),
            "thinking" | "executing" | "auditing" | "running" | "stopping"
        )
    }

    /// Clears the window back to an unsent session; the first message creates it.
    fn start_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_new_session(cx);
        self.restore_draft(window, cx);
        self.composer.update(cx, |state, cx| {
            let focus = state.focus_handle(cx);
            focus.focus(window, cx);
        });
        cx.notify();
    }

    fn reset_new_session(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_some() {
            self.switch_draft(None, cx);
        }
        self.rewind_pending = false;
        self.model.select();
        self.models_saved_at = None;
        self.models_pending = None;
        self.clear_pending = false;
        self.worker.set_child(None);
        self.active_child = None;
        self.child_pending.clear();
        self.child_rows.clear();
        self.approval_drafts.clear();
        self.rows.clear();
        self.markdown.clear();
        self.expanded.clear();
        self.child_rows.clear();
        self.ask_user_choice.clear();
        self.ask_user_text.clear();
        self.ask_user_answered.clear();
        self.config_acks.clear();
        self.planning_mode = false;
        self.timeline_scroll.follow = true;
        self.timeline_scroll.scroll_pending = true;
        self.clear_dir_state();
        cx.notify();
    }

    /// Sends the composer text, creating the session first when none is selected.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_draft(window, cx);
        let text = self.composer.read(cx).value().trim().to_string();
        if self.rewind_pending
            || (text.is_empty() && self.attachments.is_empty())
            || self.start_pending
        {
            return;
        }
        let attachments = self.attachments.clone();
        let attached_context = attachment_context(&attachments);
        let metadata = attachment_metadata(&attachments);
        match self.selected.clone() {
            Some(id) => {
                let queue = (self.workflow_running()
                    || matches!(self.model.wait_reason.as_str(), "approval" | "sub_agent"))
                    && !matches!(
                        self.model.wait_reason.as_str(),
                        "user_input" | "confirmation"
                    );
                let mut signal = serde_json::json!({
                    "type": "user_message",
                    "content": text,
                    "attached_context": attached_context,
                    "metadata": metadata,
                });
                let result = if queue {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
                    let queue_id = format!("gpui-{}-{nanos}-{sequence}", std::process::id());
                    signal["queued_user_message_id"] = serde_json::json!(queue_id);
                    let mut correlation = metadata.unwrap_or_else(|| serde_json::json!({}));
                    correlation["client_message_id"] = serde_json::json!(queue_id);
                    signal["metadata"] = correlation.clone();
                    self.worker
                        .commands
                        .try_send(Command::SubmitQueuedMessage {
                            generation: self.model.generation,
                            session: id,
                            queue_id: queue_id.clone(),
                            signal,
                        })
                        .map(|()| {
                            self.model
                                .queued_messages
                                .push(chatspeed_gpui::model::QueuedMessage {
                                    id: queue_id,
                                    content: text,
                                    metadata: correlation,
                                    provisional: true,
                                });
                        })
                } else if !self.options.demo
                    && matches!(self.model.state.as_str(), "completed" | "failed" | "cancelled" | "error")
                {
                    static NEXT_TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let sequence = NEXT_TURN.fetch_add(1, Ordering::Relaxed);
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
                    let client_id = format!("gpui-turn-{}-{nanos}-{sequence}", std::process::id());
                    let mut correlation = metadata.unwrap_or_else(|| serde_json::json!({}));
                    correlation["client_message_id"] = serde_json::json!(client_id);
                    signal["metadata"] = correlation.clone();
                    correlation["gpui_turn_status"] = serde_json::json!("pending");
                    let label = if text.is_empty() {
                        attachments.iter().map(|item| item.name.as_str()).collect::<Vec<_>>().join(", ")
                    } else {
                        text
                    };
                    self.worker.commands.try_send(Command::SubmitTurn {
                        generation: self.model.generation,
                        session: id,
                        client_id,
                        signal,
                    }).map(|()| {
                        self.model.pending_turns.push(Message {
                            id: 0,
                            role: "user".into(),
                            message: label,
                            metadata: correlation,
                            ..Message::default()
                        });
                        self.rows = presentation::project(self.transcript(), self.roots());
                        self.refresh_markdown(cx);
                    })
                } else {
                    self.worker
                        .commands
                        .try_send(Command::Signal { id, signal })
                };
                if result.is_err() {
                    self.show_toast(
                        ToastKind::Error,
                        t!("queued_message_send_failed").to_string(),
                        cx,
                    );
                    return;
                }
                self.timeline_scroll.follow = true;
                self.model.resume_follow();
                self.timeline_scroll.scroll_pending = true;
            }
            None => {
                self.start_pending = true;
                // A new workflow still needs a query for its list title; fall
                // back to the attachment names when the user typed nothing.
                let prompt = if text.is_empty() {
                    attachments
                        .iter()
                        .map(|attachment| attachment.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    text
                };
                if self
                    .worker
                    .commands
                    .try_send(Command::StartNew {
                        prompt,
                        attached_context,
                        metadata,
                    })
                    .is_err()
                {
                    self.start_pending = false;
                    self.show_toast(
                        ToastKind::Error,
                        t!("queued_message_send_failed").to_string(),
                        cx,
                    );
                    return;
                }
                self.start_draft = Some((self.composer.read(cx).value().to_string(), attachments));
            }
        }
        self.attachments.clear();
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Opens the platform image picker, like Vue's attachment dialog.
    fn pick_attachments(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(SharedString::from(t!("add_attachment").to_string())),
        });
        cx.spawn(async move |this, cx| {
            let picked = receiver
                .await
                .ok()
                .and_then(|result| result.ok())
                .flatten()
                .unwrap_or_default();
            if picked.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| this.add_attachments(picked, cx));
        })
        .detach();
    }

    fn add_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            if !is_image_path(&path) {
                self.show_toast(
                    ToastKind::Warning,
                    t!("attachment_unsupported").to_string(),
                    cx,
                );
                continue;
            }
            if self.attachments.iter().any(|item| item.path == path) {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            self.attachments.push(Attachment { path, name });
        }
        cx.notify();
    }

    fn remove_queued_message(&mut self, generation: u64, id: &str, cx: &mut Context<Self>) {
        if !self.model.accepts(generation)
            || !self.model.queued_messages.iter().any(|item| item.id == id)
        {
            return;
        }
        let Some(session) = self.selected.clone() else {
            return;
        };
        // Leave the row visible until the runtime confirms removal or applies it.
        if self.worker.commands.try_send(Command::Signal {
            id: session,
            signal: serde_json::json!({"type": "remove_queued_user_message", "queued_user_message_id": id}),
        }).is_err() {
            self.show_toast(ToastKind::Error, t!("queued_message_remove_failed").to_string(), cx);
        }
    }

    fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            cx.notify();
        }
    }

    fn stop_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone().filter(|_| self.model.can_stop()) else {
            return;
        };
        let generation = self.model.generation;
        let answer = window.prompt(
            PromptLevel::Warning,
            &t!("stop_confirm"),
            None,
            &[
                PromptButton::cancel(t!("action_cancel").into_owned()),
                PromptButton::new(t!("stop_workflow").into_owned()),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await.ok() != Some(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.selected.as_deref() == Some(id.as_str())
                    && this.model.accepts(generation)
                    && this.model.can_stop()
                {
                    let _ = this.worker.commands.try_send(Command::Stop { id });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn toggle_composer_expanded(&mut self, cx: &mut Context<Self>) {
        self.composer_expanded = !self.composer_expanded;
        cx.notify();
    }

    /// Opens one of the configuration dialogs from the `+` menu.
    fn open_quick_panel(&mut self, panel: QuickPanel, cx: &mut Context<Self>) {
        let config = self.agent_config();
        match panel {
            QuickPanel::Skills => self.skills_draft = Some(SkillsDraft::from_config(&config)),
            QuickPanel::Mcp | QuickPanel::Tools => {
                self.tool_draft = Some(ToolDraft::from_config(&config));
                if self.tools.is_empty() {
                    let _ = self.worker.commands.try_send(Command::Tools);
                }
            }
            QuickPanel::Models => {
                self.model_config_slot = if self.planning_mode { "plan" } else { "act" }.into();
            }
            QuickPanel::Personality => {}
        }
        self.quick_panel = Some(panel);
        cx.notify();
    }

    fn close_quick_panel(&mut self, cx: &mut Context<Self>) {
        self.quick_panel = None;
        self.skills_draft = None;
        self.tool_draft = None;
        cx.notify();
    }

    /// Vue's "Continue" is offered for paused, failed, cancelled and errored
    /// workflows; the runtime resumes from that signal.
    fn can_continue(&self) -> bool {
        self.selected.is_some() && !self.options.demo && self.model.can_continue()
    }

    fn continue_workflow(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone().filter(|_| self.can_continue()) {
            if self.worker.commands.try_send(Command::Signal {
                id,
                signal: serde_json::json!({"type": "continue"}),
            }).is_err() {
                return;
            }
            self.timeline_scroll.follow = true;
            self.model.resume_follow();
            self.timeline_scroll.scroll_pending = true;
            cx.notify();
        }
    }

    fn manual_compress(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            let _ = self.worker.commands.try_send(Command::Signal {
                id,
                signal: serde_json::json!({"type": "manual_compress"}),
            });
            cx.notify();
        }
    }

    /// Answers the approval the runtime is waiting on (`id` is the tool call).
    fn submit_approval(
        &mut self,
        tool_call_id: String,
        approved: bool,
        approve_all: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.selected.clone() else {
            return;
        };
        let reason = self
            .approval_drafts
            .get(&tool_call_id)
            .map(|input| input.read(cx).value().trim().to_owned());
        let mut signal = serde_json::json!({"type":"approval", "id":tool_call_id, "approved":approved, "approve_all":approve_all});
        if !approved {
            if let Some(reason) = reason.filter(|reason| !reason.is_empty()) {
                signal["rejection_message"] = Value::String(reason);
            }
        }
        let _ = self.worker.commands.try_send(Command::Signal {
            id: session,
            signal,
        });
        cx.notify();
    }

    /// True while the runtime waits on this `ask_user` call for an answer.
    fn ask_user_active(&self, tool: &ToolView) -> bool {
        !tool.tool_call_id.is_empty()
            && self.model.awaiting_user_tool_call_id.as_deref() == Some(tool.tool_call_id.as_str())
    }

    /// Answers submitted here and not yet echoed by the runtime.
    fn ask_user_answered(&self, tool: &ToolView) -> Option<&[LocalAskUserAnswer]> {
        self.ask_user_answered
            .get(&tool.tool_call_id)
            .map(Vec::as_slice)
    }

    /// The projection may already carry the echoed answer before the runtime
    /// leaves its waiting phase (for example after reopening the window).
    fn ask_user_projected_answer(tool: &ToolView) -> bool {
        tool.choices
            .iter()
            .any(|group| group.selected.is_some() || group.other_selected || group.answer.is_some())
    }

    fn set_ask_user_choice(&mut self, key: String, option: usize, cx: &mut Context<Self>) {
        self.ask_user_choice.insert(key, option);
        cx.notify();
    }

    /// Creates the free-text field Vue keeps under every group while it waits,
    /// so the "Other" row and its text area read as one answer.
    fn ensure_ask_user_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.model.awaiting_user_tool_call_id.clone() else {
            return;
        };
        if self.ask_user_answered.contains_key(&id) {
            return;
        }
        let keys: Vec<String> = self
            .rows
            .iter()
            .find_map(|row| match row {
                Row::Tool(tool) if tool.tool_call_id == id && !tool.choices.is_empty() => Some(
                    (0..tool.choices.len())
                        .map(|index| format!("{id}:{index}"))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        for key in keys {
            if self.ask_user_text.contains_key(&key) {
                continue;
            }
            let state = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(1, 6)
                    .placeholder(t!("ask_user_other_placeholder").to_string())
            });
            let event_key = key.clone();
            cx.subscribe_in(&state, window, move |this, _, event: &InputEvent, _, cx| {
                // Vue focuses the text area as soon as "Other" is picked and
                // picks "Other" as soon as the field gains focus.
                if !matches!(event, InputEvent::Focus | InputEvent::Change) {
                    return;
                }
                let Some((_, index)) = event_key.rsplit_once(':') else {
                    return;
                };
                let Ok(index) = index.parse::<usize>() else {
                    return;
                };
                if !this.ask_user_choice.contains_key(&event_key) {
                    this.ask_user_choice.insert(event_key.clone(), index);
                    cx.notify();
                }
            })
            .detach();
            self.ask_user_text.insert(key, state);
        }
    }

    /// Vue focuses the free-text field as soon as the "Other" row is picked.
    fn ask_user_supplement(
        &mut self,
        key: String,
        options: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_user_choice.insert(key.clone(), options);
        if let Some(state) = self.ask_user_text.get(&key) {
            state.update(cx, |state, cx| {
                state.focus_handle(cx).focus(window, cx);
            });
        }
        cx.notify();
    }

    /// Submits the drafted answer as the hidden `user_message` the engine reads.
    fn submit_ask_user(&mut self, tool: ToolView, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone() else {
            return;
        };
        if !self.ask_user_active(&tool) || self.ask_user_answered.contains_key(&tool.tool_call_id) {
            return;
        }
        let mut answers = Vec::with_capacity(tool.choices.len());
        let mut choices = Vec::with_capacity(tool.choices.len());
        let mut supplements = Vec::with_capacity(tool.choices.len());
        for (index, group) in tool.choices.iter().enumerate() {
            let key = format!("{}:{}", tool.tool_call_id, index);
            let choice = self.ask_user_choice.get(&key).copied();
            let supplement = self
                .ask_user_text
                .get(&key)
                .map(|state| state.read(cx).value().to_string())
                .unwrap_or_default();
            let supplement = supplement.trim().to_owned();
            let other_index = group.options.len();
            // A filled text area without a radio choice is Vue's "Other" path.
            let (choice, other) = match choice {
                Some(choice) => (Some(choice), choice == other_index),
                None if !supplement.is_empty() => (Some(other_index), true),
                None => (None, false),
            };
            answers.push(LocalAskUserAnswer {
                selected: choice.filter(|choice| *choice < other_index),
                other,
                supplement: supplement.clone(),
            });
            choices.push(choice);
            supplements.push(supplement);
        }
        let drafts: Vec<presentation::AskUserDraft<'_>> = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| presentation::AskUserDraft {
                choice: *choice,
                supplement: supplements[index].as_str(),
            })
            .collect();
        let content = match presentation::ask_user_response(&tool.choices, &drafts) {
            Ok(content) => content,
            Err(error) => {
                self.show_toast(
                    ToastKind::Warning,
                    match error {
                        presentation::AskUserDraftError::OtherRequired => {
                            t!("ask_user_other_required")
                        }
                        presentation::AskUserDraftError::Required => t!("ask_user_required"),
                    }
                    .to_string(),
                    cx,
                );
                return;
            }
        };
        if self.worker.commands.try_send(Command::Signal {
            id: session,
            signal: serde_json::json!({
                "type": "user_message",
                "content": content,
                "metadata": {
                    "ui_visibility": "hide",
                    "ask_user_response": true,
                    "requested_tool_call_id": tool.tool_call_id,
                },
            }),
        }).is_err() {
            self.show_toast(ToastKind::Error, t!("ask_user_submit_failed").to_string(), cx);
            return;
        }
        // Switch the card to its answered form immediately; the runtime's
        // persisted answer takes over on the next projection.
        self.ask_user_answered
            .insert(tool.tool_call_id.clone(), answers);
        cx.notify();
    }

    fn open_child(&mut self, card: SubAgentView, cx: &mut Context<Self>) {
        let Some(id) = card.task_id.clone() else { return; };
        self.active_child = Some(card);
        self.child_epoch = self.child_epoch.wrapping_add(1);
        self.child_pending.clear();
        self.child_rows.clear();
        self.child_model = Model::default();
        self.child_connected = None;
        self.worker.set_child(Some((self.model.generation, self.child_epoch, id.clone())));
        self.child_pending.insert(id);
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.timeline_scroll.scrollbar_input.take() && self.selected.is_some() {
            let follow = self.near_bottom();
            if follow != self.timeline_scroll.follow {
                self.timeline_scroll.follow = follow;
                if follow {
                    self.model.resume_follow();
                } else {
                    self.model.freeze();
                }
                cx.notify();
            }
        }
        self.metrics.queue_depth_peak =
            self.metrics.queue_depth_peak.max(self.worker.updates.len());
        let mut dirty = false;
        for _ in 0..128 {
            if self.timeline_scroll.prepend_anchor.is_some() {
                break;
            }
            let Some(update) = self
                .timeline_scroll
                .deferred_update
                .take()
                .or_else(|| self.worker.updates.try_recv().ok())
            else {
                break;
            };
            // Give prepends an isolated layout frame; appended height must not affect restoration.
            if dirty && matches!(&update.kind, UpdateKind::Older(_)) {
                self.timeline_scroll.deferred_update = Some(update);
                break;
            }
            self.metrics.queue(update.queued);
            self.metrics.queue_depth_peak = self.metrics.queue_depth_peak.max(update.queue_depth);
            if update.generation != 0 && !self.model.accepts(update.generation) {
                self.metrics.stale_responses += 1;
                continue;
            }
            match update.kind {
                UpdateKind::ConfigSaved { session, key, result } => {
                    self.config_pending.remove(&(session.clone(), key));
                    if self.selected.as_deref() == Some(session.as_str()) {
                        match result {
                            Ok(patch) => {
                                if key == "agent" {
                                    self.config_acks.clear();
                                    self.models_saved_at = None;
                                }
                                self.apply_config_ack(key, &patch);
                                self.config_acks.insert(key, (update.queued, patch));
                                self.sync_planning_mode();
                            }
                            Err(error) => self.show_toast(ToastKind::Error, error, cx),
                        }
                        if key == "skills" && self.skills_draft.is_some() {
                            self.skills_draft = Some(SkillsDraft::from_config(&self.agent_config()));
                        }
                    }
                }
                UpdateKind::SettingsChanged {
                    session,
                    key,
                    result,
                } => {
                    let entry = (session, key);
                    if let Err(error) = result {
                        self.pending_settings.remove(&entry);
                        if key == "phase" && self.selected.as_ref() == Some(&entry.0) {
                            self.sync_planning_mode();
                        }
                        self.show_toast(ToastKind::Error, error, cx);
                    } else if let Some((_, ack)) = self.pending_settings.get_mut(&entry) {
                        *ack = Some(Instant::now());
                    }
                }
                UpdateKind::TodayCost(amount) => self.today_cost = amount,
                UpdateKind::Rewound(result) => {
                    self.rewind_pending = false;
                    match result {
                        Ok(true) => {
                            if let Some(id) = self.selected.clone() {
                                self.select(id, cx);
                            }
                            self.show_toast(ToastKind::Success, t!("rewind_done").to_string(), cx);
                        }
                        Ok(false) => self.show_toast(
                            ToastKind::Warning,
                            t!("rewind_unavailable").to_string(),
                            cx,
                        ),
                        Err(error) => self.show_toast(ToastKind::Error, error, cx),
                    }
                }
                UpdateKind::SessionEdited { session, title, result } => match result {
                    Err(error) => self.show_toast(ToastKind::Error, error, cx),
                    Ok(()) => {
                        if let Some(title) = title {
                            if let Some(entry) = self.sessions.iter_mut().find(|entry| entry.id == session) {
                                entry.title = title;
                            }
                        } else {
                            self.sessions.retain(|entry| entry.id != session);
                            self.pending_decisions.remove(&session);
                            if self.selected.as_ref() == Some(&session) {
                                self.reset_new_session(cx);
                                if let Some(id) = self.sessions.first().map(|entry| entry.id.clone()) {
                                    self.select(id, cx);
                                }
                            }
                            self.drafts.remove(&Some(session));
                        }
                    }
                },
                UpdateKind::Automations(result) => self.automations = Some(result),
                UpdateKind::AutomationRuns(id, result) => {
                    if let Some((current, runs)) = &mut self.automation_history {
                        if *current == id {
                            *runs = Some(result);
                        }
                    }
                }
                UpdateKind::AutomationCreated(result) => {
                    if result.is_ok() && self.rail_tab == ui::sidebar::RailTab::Automation {
                        self.refresh_automations();
                    }
                    self.automation_pending = false;
                    match result {
                        Ok(()) => self.show_toast(
                            ToastKind::Success,
                            t!("automation_created").to_string(),
                            cx,
                        ),
                        Err(error) => self.show_toast(ToastKind::Error, error, cx),
                    }
                }
                UpdateKind::Sessions(sessions) => self.sessions = sessions,
                UpdateKind::Connected(connected) => self.connected = connected,
                UpdateKind::Config(config, fetched_at) => {
                    self.apply_config(&config, fetched_at, cx);
                }
                UpdateKind::DesktopMenuUnavailable => {
                    self.show_toast(
                        ToastKind::Warning,
                        t!("desktop_menu_unavailable").to_string(),
                        cx,
                    );
                }
                UpdateKind::SoundSaved {
                    chime,
                    muted,
                    saved,
                } => {
                    self.sound_pending.remove(&chime);
                    if saved {
                        self.sound_saved_at = Some(Instant::now());
                        self.set_sound_enabled(chime, !muted);
                    } else {
                        self.show_toast(ToastKind::Error, t!("sound_save_failed").to_string(), cx);
                    }
                }
                UpdateKind::Decisions {
                    owner,
                    pending,
                    reminder,
                } => {
                    let sound = match reminder {
                        Some(Reminder::Sound(chime)) => Some(chime),
                        Some(Reminder::BackgroundQuestion)
                            if self.selected.as_deref() != Some(&owner) =>
                        {
                            Some(Chime::Approval)
                        }
                        _ => None,
                    };
                    if pending.is_empty() {
                        self.pending_decisions.remove(&owner);
                    } else {
                        self.pending_decisions.insert(owner, pending);
                    }
                    if let Some(chime) = sound.filter(|chime| self.sound_is_enabled(*chime)) {
                        if let Some(player) = &self.sound_player {
                            player.play(chime);
                        } else {
                            self.show_toast(
                                ToastKind::Warning,
                                t!("sound_playback_unavailable").to_string(),
                                cx,
                            );
                        }
                    }
                }
                UpdateKind::Opened(id) => {
                    self.start_pending = false;
                    self.start_draft = None;
                    if self.selected.is_none() {
                        self.select(id, cx);
                    }
                }
                UpdateKind::StartFailed(message) => {
                    self.start_pending = false;
                    if let Some((text, attachments)) = self.start_draft.take() {
                        if self.selected.is_none()
                            && self
                                .restore_composer
                                .as_deref()
                                .unwrap_or(&self.composer.read(cx).value())
                                .is_empty()
                            && self.attachments.is_empty()
                        {
                            self.restore_composer = Some(text);
                            self.attachments = attachments;
                        } else if self.selected.is_some() {
                            self.drafts.entry(None).or_insert((text, attachments));
                        }
                    }
                    self.show_toast(ToastKind::Error, message, cx);
                }
                UpdateKind::Child(epoch, id, result) => {
                    if epoch != self.child_epoch || self.active_child.as_ref().and_then(|card| card.task_id.as_ref()) != Some(&id) {
                        continue;
                    }
                    match *result {
                        UpdateKind::Snapshot(page) => {
                            self.child_model.page(page, false);
                            self.child_pending.remove(&id);
                        }
                        UpdateKind::Connected(connected) => {
                            self.child_connected = Some(connected);
                            cx.notify();
                            continue;
                        }
                        UpdateKind::Event(event) => self.child_model.event(event),
                        UpdateKind::Reset => self.child_model.reset_live(),
                        _ => continue,
                    }
                    let messages = Self::transcript_for(&self.child_model);
                    self.child_rows.insert(id, presentation::project(messages, &self.child_model.roots));
                    cx.notify();
                    continue;
                }
                UpdateKind::Error(message) => {
                    self.show_toast(ToastKind::Error, message, cx);
                }
                UpdateKind::Catalogue {
                    agents,
                    models,
                    skills,
                    schemes,
                } => {
                    self.agents = agents;
                    self.models = models;
                    self.skills = skills;
                    self.sandbox_schemes = schemes;
                    cx.notify();
                }
                UpdateKind::Tools(tools) => {
                    self.tools = tools;
                    cx.notify();
                }
                UpdateKind::ContextCleared(result) => {
                    self.clear_pending = false;
                    match result {
                        Ok(value) => {
                            if self.model.apply_context_clear(&value) && value["noop"] != true {
                                self.timeline_scroll.scroll_pending = self.timeline_scroll.follow;
                                self.show_toast(
                                    ToastKind::Success,
                                    t!("clear_context_frame_done").into_owned(),
                                    cx,
                                );
                            }
                        }
                        Err(error) => self.show_toast(ToastKind::Error, error, cx),
                    }
                }
                UpdateKind::ModelsSaved(result) => {
                    self.models_pending = None;
                    match result {
                        Ok(models) => {
                            let mut config = self.agent_config();
                            config["models"] = models;
                            self.model.workflow["agent_config"] = config;
                            self.models_saved_at = Some(update.queued);
                        }
                        Err(error) => self.show_toast(ToastKind::Error, error, cx),
                    }
                }
                UpdateKind::Files { request, files } => {
                    if self
                        .suggestions
                        .file_request
                        .as_ref()
                        .is_some_and(|(id, _, _)| *id == request)
                    {
                        self.file_results = files;
                    }
                    cx.notify();
                }
                UpdateKind::Dir {
                    path,
                    entries,
                    error,
                } => {
                    self.dir_pending.remove(&path);
                    if let Some(error) = error {
                        self.dir_errors.insert(path, error);
                    } else {
                        self.dir_errors.remove(&path);
                        self.dir_entries.insert(path, entries);
                    }
                    // Directory updates do not change the transcript, so skip
                    // the projection pass a dirty batch would trigger.
                    cx.notify();
                }
                UpdateKind::Reconnect => self.metrics.reconnects += 1,
                UpdateKind::DecodeUs(us) => {
                    self.metrics.snapshot_decode_us += us;
                    self.metrics.snapshot_samples += 1;
                }
                UpdateKind::Reset => {
                    self.metrics.resets += 1;
                    self.model.reset_live();
                }
                UpdateKind::TurnUnconfirmed { client_id, message } => {
                    if self.model.mark_turn_unconfirmed(&client_id) {
                        self.show_toast(ToastKind::Error, message, cx);
                    }
                }
                UpdateKind::QueueSubmissionFailed { queue_id, message } => {
                    if self.model.reject_queue_submission(&queue_id) {
                        self.show_toast(ToastKind::Error, message, cx);
                    }
                }
                UpdateKind::Snapshot(page) => {
                    let preserve_models = self.models_saved_at.is_some_and(|at| page.fetched_from.is_none_or(|start| start <= at));
                    let saved_models = preserve_models.then(|| self.agent_config()["models"].clone());
                    if let (Some(session), Some(fetched_from)) =
                        (self.selected.as_ref(), page.fetched_from)
                    {
                        self.pending_settings.retain(|(id, _), (_, ack)| {
                            id != session || !ack.is_some_and(|at| fetched_from > at)
                        });
                    }
                    let fetched_from = page.fetched_from;
                    self.model.page(page, false);
                    self.config_acks.retain(|_, (saved, _)| fetched_from.is_none_or(|start| start <= *saved));
                    let mut patches: Vec<_> = self.config_acks.iter().map(|(&key, (at, patch))| (*at, key, patch.clone())).collect();
                    patches.sort_by_key(|(at, _, _)| *at);
                    for (_, key, patch) in patches {
                        self.apply_config_ack(key, &patch);
                    }
                    if let Some(models) = saved_models {
                        let mut config = self.agent_config();
                        config["models"] = models;
                        self.model.workflow["agent_config"] = config;
                    } else {
                        self.models_saved_at = None;
                    }
                    self.sync_planning_mode();
                    self.auto_expand_first_root(cx);
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
                    self.timeline_scroll.prepend_anchor = Some((
                        self.timeline_scroll.scroll.offset(),
                        self.timeline_scroll.scroll.max_offset(),
                    ));
                    self.model.page(page, true);
                    self.timeline_scroll.follow = false;
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
        if self
            .sound_player
            .as_ref()
            .is_some_and(|player| player.playback_failed())
        {
            self.show_toast(
                ToastKind::Warning,
                t!("sound_playback_unavailable").to_string(),
                cx,
            );
            cx.notify();
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.expired(Instant::now()))
        {
            self.toast = None;
            cx.notify();
        }
        if self.copy_flash_expired() {
            cx.notify();
        }
        // The live status line counts seconds; a running task needs a repaint
        // even when no event arrives.
        if self.workflow_running() && self.last_tick.elapsed() >= Duration::from_secs(1) {
            self.last_tick = Instant::now();
            cx.notify();
        }
        if dirty {
            if self.timeline_scroll.follow {
                self.timeline_scroll.scroll_pending = true;
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
            // The catalogue is fetched at startup, which can precede the
            // runtime connection; keep asking until it answers.
            if self.agents.is_empty() {
                self.request_catalogue();
            }
        }
    }

    /// Mirrors the runtime preferences shared with the Vue titlebar.
    fn apply_config(&mut self, config: &Value, fetched_at: Instant, cx: &mut Context<Self>) {
        if !self.sidebar_changed {
            self.sidebar_collapsed = config["gpui_workflow_sidebar_collapsed"]
                .as_bool()
                .unwrap_or(false);
        }
        self.send_with_shift = config["send_message_key"].as_str() == Some("Shift+Enter");
        self.show_menu_button = config["show_menu_button"].as_bool().unwrap_or(true);
        self.show_today_cost = config["show_today_cost_stats"].as_bool().unwrap_or(false);
        if !self.show_today_cost {
            self.today_cost = 0.;
        }
        // A poll already in flight when a save completed may contain old values.
        if self
            .sound_saved_at
            .is_none_or(|saved_at| fetched_at > saved_at)
        {
            for chime in Chime::ALL {
                self.set_sound_enabled(chime, !chime.muted(config));
            }
        }
        let language = config["interface_language"]
            .as_str()
            .map(normalize_locale)
            .unwrap_or_else(chatspeed_gpui::prefs::system_locale);
        if *rust_i18n::locale() != *language {
            rust_i18n::set_locale(language);
            self.rows = presentation::project(self.transcript(), self.roots());
            self.refresh_markdown(cx);
        }
        set_primary(config["primary_color"].as_str().unwrap_or("green"));
        self.proxy_targets = config["chat_completion_proxy"].clone();
        if !self.geometry_ready && !self.geometry_restore_pending {
            self.geometry_restore_pending = true;
            self.adopt_window_size(config);
        }
        #[cfg(target_os = "windows")]
        if !std::mem::replace(&mut self.window_position_read, true) {
            self.pending_window_position =
                prefs::saved_window_position(config).map(|(x, y, name)| (x, y, name.to_owned()));
        }
        self.theme_setting = config["theme"].as_str().unwrap_or("system").to_owned();
        self.refresh_palette(cx);
        cx.notify();
    }

    /// Queues the remembered workflow geometry so the window stops at the size
    /// the Tauri window was left at. The configuration poll repeats every few
    /// seconds, so only the first value is adopted.
    fn adopt_window_size(&mut self, config: &Value) {
        if self.applied_window_size.is_some() {
            return;
        }
        let Some((width, height)) = prefs::saved_window_size(config) else {
            return;
        };
        let size = Size {
            width: px(width.max(WORKFLOW_WINDOW_MIN_SIZE.width.as_f32())),
            height: px(height.max(WORKFLOW_WINDOW_MIN_SIZE.height.as_f32())),
        };
        self.applied_window_size = Some(size);
        self.pending_window_size = Some(size);
    }

    /// Applies the primary preset and the Vue light/dark surface colors.
    fn refresh_palette(&mut self, cx: &mut Context<Self>) {
        let dark = match self.theme_setting.as_str() {
            "light" => false,
            "dark" => true,
            _ => ThemeMode::from(cx.window_appearance()).is_dark(),
        };
        let primary = PRIMARY.load(Ordering::Relaxed);
        if self.palette_applied && dark == self.appearance_dark && primary == self.applied_primary {
            return;
        }
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        let accent: Hsla = rgb(primary).into();
        let (background, muted, border, foreground, muted_foreground) = if dark {
            (
                rgb(0x202122),
                rgb(0x242426),
                rgb(0xffffff).opacity(0.12),
                rgb(0xffffff).opacity(0.87),
                rgb(0x848586),
            )
        } else {
            (
                rgb(0xf1f2f1),
                rgb(0xeaeaea),
                rgb(0x000000).opacity(0.10),
                rgb(0x000000).opacity(0.95),
                rgb(0x000000).opacity(0.55),
            )
        };
        Theme::update(cx, |theme| {
            // The kit rounds client-decorated windows from `window_radius`; the
            // default is square, which is what the frame looked like before.
            theme.window_radius = px(WINDOW_RADIUS);
            theme.colors.primary = accent;
            theme.colors.primary_hover = accent.opacity(0.9);
            theme.colors.primary_active = accent.opacity(0.8);
            theme.colors.ring = accent.opacity(0.45);
            theme.colors.background = background.into();
            theme.colors.popover = background.into();
            theme.colors.muted = muted.into();
            theme.colors.border = border.into();
            theme.colors.sidebar = muted.into();
            theme.colors.sidebar_border = border.into();
            theme.colors.foreground = foreground.into();
            theme.colors.muted_foreground = muted_foreground.into();
        });
        self.appearance_dark = dark;
        self.applied_primary = primary;
        self.palette_applied = true;
        cx.notify();
    }

    /// Vue resumes following once the reader is within 60px of the tail.
    ///
    /// `ScrollHandle` reports the distance scrolled as a negative offset, so
    /// the remaining distance is `max + offset`; subtracting would count the
    /// whole transcript and never report the tail.
    fn near_bottom(&self) -> bool {
        let max = self.timeline_scroll.scroll.max_offset();
        let offset = self.timeline_scroll.scroll.offset();
        (max.y + offset.y) <= px(60.)
    }

    fn copy_flash_expired(&self) -> bool {
        let Ok(mut flash) = self.copy_flash.lock() else {
            return false;
        };
        if flash.is_some_and(|copy| copy.at.elapsed() >= COPY_FLASH) {
            *flash = None;
            return true;
        }
        false
    }

    fn roots(&self) -> &[String] {
        &self.model.roots
    }

    /// Durable rows plus the transient live message and live tool lifecycle state.
    fn transcript(&self) -> Vec<Message> {
        Self::transcript_for(&self.model)
    }

    fn transcript_for(model: &Model) -> Vec<Message> {
        let mut messages: Vec<Message> = model.visible().cloned().collect();
        messages.extend(model.pending_turns.iter().cloned());
        if !model.live.message.is_empty() || model.live.reasoning.is_some() {
            let mut live = model.live.clone();
            live.id = -1;
            live.role = "assistant".into();
            messages.push(live);
        }
        for id in &model.tool_order {
            let Some(tool) = model.tools.get(id) else {
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
        // Expanding a card grows the transcript under the pointer, so Vue keeps
        // the newly revealed body visible; following the tail is the same move.
        if self.timeline_scroll.follow {
            self.timeline_scroll.scroll_pending = true;
        }
        self.refresh_markdown(cx);
        cx.notify();
    }

    fn markdown_view(&self, key: &str) -> AnyElement {
        let flash = self.copy_flash.clone();
        match self.markdown.get(key) {
            Some((_, state)) => div()
                .w_full()
                .min_w_0()
                .child(TextView::new(state).selectable(true).code_block_actions(
                    move |code_block, _, cx| {
                        // GPUI renders these in the code block's top-right corner, where
                        // the language label would otherwise sit.
                        let code = code_block.code().to_string();
                        let lang = code_block
                            .lang()
                            .map(|lang| lang.to_string())
                            .unwrap_or_default();
                        let block = code_key(&code);
                        let copied = flash
                            .lock()
                            .ok()
                            .and_then(|slot| *slot)
                            .is_some_and(|copy| {
                                copy.key == block && copy.at.elapsed() < COPY_FLASH
                            });
                        let muted = cx.theme().muted_foreground;
                        let on_copy = flash.clone();
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
                                    .child(Icon::new("copy").size(12.).color(muted))
                                    .on_click(move |_, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code.clone(),
                                        ));
                                        if let Ok(mut slot) = on_copy.lock() {
                                            *slot = Some(CopyFlash {
                                                key: block,
                                                at: Instant::now(),
                                            });
                                        }
                                        window.refresh();
                                    }),
                            )
                            .when(copied, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(success())
                                        .child(t!("copied").to_string()),
                                )
                            })
                            .into_any_element()
                    },
                ))
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn user_row(&self, view: &UserView, cx: &mut Context<Self>) -> AnyElement {
        let key = format!("{}:user", view.key);
        let target = key.clone();
        let expanded = self.expanded.contains(&key);
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx));
        ui::transcript::messages::user_bubble(view, key, expanded, on_toggle, cx)
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
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx));
        let live = key == "msg--1:thought"
            && self.workflow_running()
            && self.model.live.message.is_empty();
        let title = if live {
            t!("live_thinking").to_string()
        } else {
            title
        };
        ui::transcript::messages::thought(key, title, text, open, live, on_toggle, cx)
    }

    fn assistant_row(&self, view: &AssistantView, cx: &mut Context<Self>) -> AnyElement {
        let thought = view.reasoning.as_ref().map(|reasoning| {
            self.thought_row(
                view.thought_key.clone(),
                view.thought_title.clone(),
                reasoning,
                cx,
            )
        });
        let markdown = (!view.markdown.is_empty()).then(|| self.markdown_view(&view.key));
        ui::transcript::messages::assistant_message(thought, markdown)
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
        // A pending approval or an unanswered `ask_user` keeps its body open,
        // exactly like Vue's `isApprovalPending || isMessageExpanded`.
        let pending = !tool.tool_call_id.is_empty()
            && (tool.status == presentation::ToolStatus::Pending
                || self.model.pending_tool_ids.contains(&tool.tool_call_id));
        let answered =
            self.ask_user_answered(tool).is_some() || Self::ask_user_projected_answer(tool);
        let answering = self.ask_user_active(tool) && !answered;
        // Vue keeps `ask_user` cards open: their options are the body.
        let open = pending
            || answering
            || answered
            || !tool.choices.is_empty()
            || self.expanded.contains(&tool.detail_key);
        let target_key = tool.detail_key.clone();
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let border = cx.theme().border;
        let summary_color: Hsla = if tool.status.is_error() {
            danger().into()
        } else {
            muted
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
            .cursor_pointer()
            .child(
                Icon::new(tool.icon)
                    .size(15.)
                    .color(self.tool_icon_color(tool)),
            );
        // The label is the only flexible child, so the right-hand badges keep
        // their room and a long command truncates instead of pushing them out.
        let mut label = h_flex().flex_1().min_w_0().overflow_hidden().gap_2();
        label = label.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(foreground)
                // The action alone fills the row; otherwise it keeps its natural
                // width and the target absorbs the truncation.
                .when(tool.target.is_empty(), |action| action.flex_1())
                .child(tool.action.clone()),
        );
        if !tool.target.is_empty() {
            label = label.child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(tool.target.clone()),
            );
        }
        title = title.child(label);
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
            title = title.child(Icon::new("check").size(12.).color(success()));
        }
        if expandable {
            title = title
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(target_key.clone(), cx)));
        }
        let mut body = v_flex().w_full().min_w_0().cursor_pointer().child(title);
        if !open {
            let mut summary = h_flex()
                .id(SharedString::from(format!("{}:summary", tool.detail_key)))
                .w_full()
                .min_w_0()
                .items_start()
                .gap_1()
                .pl_6();
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
                if expandable {
                    let target = tool.detail_key.clone();
                    summary = summary.on_click(
                        cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx)),
                    );
                }
                body = body.child(summary);
            }
        } else if pending && tool.choices.is_empty() && tool.name != "ask_user" {
            // The approval card owns the diff/command/plan while pending, so the
            // regular detail would only repeat it. `ask_user` keeps its options.
            let id = tool.tool_call_id.clone();
            let owner = cx.entity().downgrade();
            let generation = self.model.generation;
            let dispatch = move |decision, _: &mut Window, cx: &mut App| {
                let _ = owner.update(cx, |this, cx| {
                    if !this.model.accepts(generation) || !this.approval_drafts.contains_key(&id) {
                        return;
                    }
                    use ui::approval::Decision;
                    match decision {
                        Decision::Reject => this.submit_approval(id.clone(), false, false, cx),
                        Decision::Approve => this.submit_approval(id.clone(), true, false, cx),
                        Decision::Whitelist => this.submit_approval(id.clone(), true, true, cx),
                        Decision::Batch => {
                            let tool_ids = this.approval_drafts.keys().cloned().collect();
                            if let Some(session) = this.selected.clone() {
                                let _ = this
                                    .worker
                                    .commands
                                    .try_send(Command::ApprovalBatch { session, tool_ids });
                            }
                        }
                    }
                });
            };
            body = body.child(div().w_full().min_w_0().pl_6().pb_2().child(
                ui::approval::approval_card(
                    tool,
                    self.approval_drafts.get(&tool.tool_call_id),
                    self.approval_drafts.len(),
                    dispatch,
                    cx,
                ),
            ));
        } else {
            body = body.child(self.tool_detail(tool, cx));
        }
        body.into_any_element()
    }

    fn tool_detail(&self, tool: &ToolView, cx: &mut Context<Self>) -> AnyElement {
        let mut detail = v_flex().w_full().min_w_0().gap_2().pl_6().pt_2().pb_2();
        let answered =
            self.ask_user_answered(tool).is_some() || Self::ask_user_projected_answer(tool);
        let answering = self.ask_user_active(tool) && !answered;
        if !tool.choices.is_empty() {
            let mut choices = v_flex().w_full().min_w_0().gap_3();
            for (index, group) in tool.choices.iter().enumerate() {
                choices = choices.child(self.choice_group(tool, group, index, answering, cx));
            }
            if answering {
                let submit = tool.clone();
                choices = choices.child(
                    h_flex().w_full().justify_end().child(
                        h_flex()
                            .id("ask-user-submit")
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(accent())
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(gpui_kit::white())
                            .child(Icon::new("send").size(12.).color(gpui_kit::white()))
                            .child(t!("submit_answer").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit_ask_user(submit.clone(), cx)
                            })),
                    ),
                );
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
        if tool.arguments.is_some() {
            // The JSON body is the following fence; only the small label lives here.
            detail = detail.child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("arguments_label").to_string()),
            );
        }
        if !tool.detail.is_empty() {
            detail = detail.child(self.markdown_view(&tool.detail_key));
        }
        if tool.status == presentation::ToolStatus::Running {
            let primary = cx.theme().primary;
            detail = detail.child(
                h_flex().items_center().gap_1().py_2().px(px(20.)).text_size(px(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new("loading").size(14.).color(primary).spin(
                        SharedString::from(format!("{}:running", tool.detail_key)),
                        ui::Spin::Eased,
                    ))
                    .child(t!("executing").into_owned()),
            );
        }
        detail.into_any_element()
    }

    fn activity_row(
        &self,
        group: &ActivityView,
        shimmer: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = group.key.clone();
        let details = self.expanded.contains(&target).then(|| {
            group
                .items
                .iter()
                .map(|item| self.activity_item(item, cx))
                .collect()
        });
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx));
        ui::transcript::activity_group(group, shimmer, details, on_toggle, cx)
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

    fn sub_agent_row(&self, card: &SubAgentView, cx: &mut Context<Self>) -> AnyElement {
        use ui::transcript::{sub_agent_card, SubAgentDetails};

        let key = card.key.clone();
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx));
        let details = self.expanded.contains(&card.key).then(|| {
            let task = self.sub_agent_section(
                t!("sub_agent_task_section").to_string(),
                card.task_key.clone(),
                &card.task,
                &card.task_preview,
                cx,
            );
            let result = card.has_result.then(|| {
                self.sub_agent_section(
                    t!("sub_agent_result_section").to_string(),
                    card.result_key.clone(),
                    &card.result,
                    &card.result_preview,
                    cx,
                )
            });
            let cost = card
                .cost
                .as_ref()
                .map(|cost| self.cost_section(card.cost_key.clone(), cost, cx));
            let open_transcript = card.task_id.as_ref().map(|_| {
                let card = card.clone();
                let hover = cx.theme().muted;
                let foreground = cx.theme().foreground;
                div().id("open-child-transcript").p_1().rounded(px(4.))
                    .cursor_pointer().hover(move |style| style.bg(hover))
                    .child(Icon::new("fullscreen-off").size(14.).color(foreground))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_child(card.clone(), cx);
                    })).into_any_element()
            });
            SubAgentDetails { task, result, cost, open_transcript }
        });
        sub_agent_card(card, details, on_toggle, cx)
    }

    fn child_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.active_child.as_ref()?;
        let id = card.task_id.as_ref()?;
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted;
        let content = if let Some(rows) = self.child_rows.get(id) {
            // The primary row renderer binds decision callbacks to the parent.
            // Until the child has its own action scope, render its preview without those callbacks.
            let entries = rows.iter().filter_map(|row| {
                let text = match row {
                    Row::Assistant(view) => {
                        return Some(v_flex().gap_1()
                            .when_some(view.reasoning.as_deref().filter(|text| !text.is_empty()), |row, text| {
                                row.child(plain_text(text, cx.theme().muted_foreground))
                            })
                            .when(!view.markdown.is_empty(), |row| row.child(plain_text(&view.markdown, foreground)))
                            .into_any_element());
                    }
                    Row::User(view) => &view.text,
                    Row::Tool(view) => &view.summary,
                    Row::Activity(view) => &view.summary,
                    _ => return None,
                };
                (!text.is_empty()).then(|| plain_text(text, foreground).into_any_element())
            });
            ui::transcript::child_transcript(entries, cx)
        } else if self.child_pending.contains(id) {
            ui::transcript::child_loading(cx)
        } else {
            div().into_any_element()
        };
        Some(v_flex().id(SharedString::from(format!("child-pane-{id}")))
            .absolute().inset_0().occlude().bg(cx.theme().background)
            .child(h_flex().h(px(40.)).flex_none().items_center().gap_2().px_3()
                .border_b_1().border_color(cx.theme().border)
                .child(Icon::new("task").size(15.).color(foreground))
                .child(div().min_w_0().text_ellipsis().overflow_hidden().font_weight(FontWeight::SEMIBOLD).child(card.agent.clone()))
                .child(div().flex_1())
                .child(div().id("close-child-pane").p_1().rounded(px(4.)).cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(Icon::new("fullscreen").size(14.).color(foreground))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.worker.set_child(None);
                        this.active_child = None;
                        this.refresh_markdown(cx);
                        cx.notify();
                    }))))
            .when(self.child_connected == Some(false), |pane| {
                pane.child(div().px_3().py_1().text_xs().text_color(cx.theme().muted_foreground)
                    .child(t!("child_reconnecting").to_string()))
            })
            .child(div().id("child-pane-scroll").flex_1().min_h_0().overflow_y_scroll().child(content))
            .into_any_element())
    }

    fn choice_group(
        &self,
        tool: &ToolView,
        group: &ChoiceGroupView,
        index: usize,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut options = v_flex().w_full().min_w_0().gap_1();
        let base = format!("{}:{}", tool.tool_call_id, index);
        let other_index = group.options.len();
        let local = self
            .ask_user_answered(tool)
            .and_then(|answers| answers.get(index));
        let read_only = !active || local.is_some();
        if read_only {
            // Vue's answered group keeps every option visible with the chosen
            // one marked, then prints the free-text answer below it.
            let selected = local
                .map(|answer| answer.selected)
                .unwrap_or(group.selected);
            let other = local
                .map(|answer| answer.other)
                .unwrap_or(group.other_selected);
            let answer = local
                .map(|answer| answer.supplement.as_str())
                .filter(|text| !text.trim().is_empty())
                .or(group.answer.as_deref());
            for (option_index, option) in group.options.iter().enumerate() {
                options = options.child(choice_option(
                    format!("{}. {option}", option_index + 1),
                    selected == Some(option_index),
                    cx,
                ));
            }
            options = options.child(choice_option(
                format!("{}. {}", other_index + 1, t!("ask_user_other")),
                other,
                cx,
            ));
            if let Some(answer) = answer {
                options = options.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .mt_2()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(cx.theme().muted)
                        .text_size(px(12.))
                        .child(plain_text(answer, cx.theme().foreground)),
                );
            }
        } else {
            for (option_index, option) in group.options.iter().enumerate() {
                let label = format!("{}. {option}", option_index + 1);
                let key = base.clone();
                options = options.child(choice_option_live(
                    label,
                    self.ask_user_choice.get(&key).copied() == Some(option_index),
                    SharedString::from(format!("{key}:{option_index}")),
                    cx.listener(move |this, _, _, cx| {
                        this.set_ask_user_choice(key.clone(), option_index, cx)
                    }),
                    cx,
                ));
            }
            let other_label = format!("{}. {}", other_index + 1, t!("ask_user_other"));
            let key = base.clone();
            options = options.child(choice_option_live(
                other_label,
                self.ask_user_choice.get(&base).copied() == Some(other_index),
                SharedString::from(format!("{base}:other")),
                cx.listener(move |this, _, window, cx| {
                    this.ask_user_supplement(key.clone(), other_index, window, cx)
                }),
                cx,
            ));
            // Vue always keeps the text area under the options while it waits;
            // focusing it selects "Other" for the user.
            if let Some(state) = self.ask_user_text.get(&base) {
                options = options.child(
                    div().w_full().pl(px(22.)).pt_1().child(
                        div()
                            .w_full()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .px_2()
                            .py_1()
                            .child(
                                Textarea::new(state)
                                    .appearance(false)
                                    .bordered(false)
                                    .w_full(),
                            ),
                    ),
                );
            }
        }
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
                    .child(
                        Icon::new(if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        })
                        .size(12.)
                        .color(muted),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx))),
            )
            .when(open, |view| view.child(ui::transcript::cost_card(cost, cx)))
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
                    .child(
                        Icon::new(if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        })
                        .size(12.)
                        .color(muted),
                    )
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
        let target = snapshot.key.clone();
        let markdown = self
            .expanded
            .contains(&target)
            .then(|| self.markdown_view(&snapshot.body_key));
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx));
        ui::transcript::snapshot_card(snapshot, markdown, on_toggle, cx)
    }

    fn finish_task_row(&self, view: &FinishTaskView, cx: &mut Context<Self>) -> AnyElement {
        let target = view.cost_key.clone();
        let open = self.expanded.contains(&target);
        let on_toggle = cx.listener(move |this, _, _, cx| this.toggle(target.clone(), cx));
        ui::transcript::finish_task_card(view, open, on_toggle, cx)
    }

    fn row(&self, row: &Row, active_group: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        ui::transcript::row(self, row, active_group, cx)
    }
}

impl Render for Observer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.restore_draft(window, cx);
        if let Some((offset, previous_max)) = self.timeline_scroll.prepend_anchor.take() {
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if !this.timeline_scroll.follow {
                        let max = this.timeline_scroll.scroll.max_offset();
                        this.timeline_scroll
                            .scroll
                            .set_offset(point(offset.x, offset.y - (max.y - previous_max.y)));
                        cx.notify();
                    }
                });
            });
        }
        if self.timeline_scroll.follow && self.timeline_scroll.scroll_pending {
            self.timeline_scroll.scroll_pending = false;
            let entity = cx.entity().downgrade();
            // Resolve against new layout bounds, and cancel if the user started reading.
            window.on_next_frame(move |_, cx| {
                let _ = entity.update(cx, |this, _| {
                    if this.timeline_scroll.follow {
                        this.timeline_scroll.scroll.scroll_to_bottom();
                    }
                });
            });
        }
        // Retain the polling task for the lifetime of this window only.
        let _ = &self.task;

        // Adopt the remembered size before the frame that draws it, clamped to
        // the display so a saved geometry cannot open a window off screen.
        if let Some(target) = self.pending_window_size.take() {
            let target = match cx.primary_display() {
                Some(display) => target.min(&display.visible_bounds().size),
                None => target,
            };
            if target != window.bounds().size {
                // Resizing now would restart layout under the frame being built.
                window.on_next_frame(move |window, _| window.resize(target));
            }
        }

        #[cfg(target_os = "windows")]
        if let Some((x, y, name)) = self.pending_window_position.take() {
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |window, _| {
                    if !window.is_maximized() && !window.is_fullscreen() {
                        ui::window_pin::restore_position(window, x, y, &name);
                    }
                });
            });
        }

        if std::mem::take(&mut self.geometry_restore_pending) {
            let observer = cx.entity().downgrade();
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, cx| {
                        let _ = observer.update(cx, |this, _| this.geometry_ready = true);
                    });
                });
            });
        }

        // Apply the palette before the first frame, then follow OS light/dark
        // switches while the setting is `system`.
        if !self.palette_applied
            || (self.theme_setting == "system"
                && ThemeMode::from(window.appearance()).is_dark() != self.appearance_dark)
        {
            self.refresh_palette(cx);
        }

        let border = cx.theme().border;
        let surface = cx.theme().muted;
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;

        // The answer fields are entities, so they are created outside the row
        // builder; Vue keeps them mounted under every waiting question.
        let approval_ids: BTreeSet<_> = self
            .model
            .pending_tool_ids
            .iter()
            .cloned()
            .chain(
                self.model
                    .messages
                    .values()
                    .filter(|row| {
                        row.metadata["approval_status"] == "pending"
                            || row.metadata["execution_status"] == "pending_approval"
                    })
                    .filter_map(|row| row.metadata["tool_call_id"].as_str().map(str::to_owned)),
            )
            .filter(|id| self.model.awaiting_user_tool_call_id.as_ref() != Some(id))
            .collect();
        self.approval_drafts
            .retain(|id, _| approval_ids.contains(id));
        for id in approval_ids {
            self.approval_drafts.entry(id).or_insert_with(|| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(t!("approval_rejection_reason").into_owned())
                        .auto_grow(2, 4)
                })
            });
        }
        self.ensure_ask_user_fields(window, cx);

        // The resize band owns the press; this frame follows the pointer while
        // it stays held, so the drag survives leaving the band.
        if let Some(drag) = self.sidebar_drag {
            let max_width = (f32::from(window.bounds().size.width) * 0.5).max(SIDEBAR_MIN_WIDTH);
            let entity = cx.entity().downgrade();
            window.on_mouse_event(
                move |event: &MouseMoveEvent, _phase, _window, cx: &mut App| {
                    if event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let _ = entity.update(cx, |this, cx| {
                        let width = (drag.start_width + f32::from(event.position.x - drag.start_x))
                            .clamp(SIDEBAR_MIN_WIDTH, max_width);
                        if (this.sidebar_width - width).abs() > 0.5 {
                            this.sidebar_width = width;
                            cx.notify();
                        }
                    });
                },
            );
            let entity = cx.entity().downgrade();
            window.on_mouse_event(move |_: &MouseUpEvent, _phase, _window, cx: &mut App| {
                let _ = entity.update(cx, |this, cx| {
                    if this.sidebar_drag.take().is_some() {
                        cx.notify();
                    }
                });
            });
        }

        let mut content = v_flex()
            .id("history")
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_3()
            .track_scroll(&self.timeline_scroll.scroll)
            .overflow_y_scroll()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                if this.selected.is_none() {
                    return;
                }
                // Landing near the tail resumes following, so reading history is a
                // pause rather than a mode the user has to leave manually.
                if this.near_bottom() {
                    if !this.timeline_scroll.follow {
                        this.timeline_scroll.follow = true;
                        this.model.resume_follow();
                        cx.notify();
                    }
                    return;
                }
                // Layout and scroll_to_bottom notifications are not reading intent.
                // Only an upward user wheel gesture suspends tail following.
                if this
                    .model
                    .pause_for_scroll(event.delta.pixel_delta(px(16.)).y.into())
                {
                    this.timeline_scroll.follow = false;
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
                    .child(Icon::new("double-arrow-up").size(12.).color(muted))
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
                            this.timeline_scroll.follow = false;
                            cx.notify();
                        }
                    })),
            );
        }
        let active_group = ui::transcript::latest_active_group(&self.rows, self.workflow_running());
        for row in &self.rows {
            // Vue separates transcript entries with the medium spacing token.
            content = content.child(div().w_full().min_w_0().mb(px(20.)).child(self.row(
                row,
                active_group,
                cx,
            )));
        }
        if !self.model.queued_messages.is_empty() {
            content = content.child(ui::composer::message_queue::render(self, cx));
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
                        .child(Icon::new("loading").size(14.).color(accent()))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(format!("{hint} {seconds}s...")),
                        ),
                ),
            );
        }

        // The kit's root plugin owns the hairline outline, the gutter shadow and
        // the compositor corner radii; this frame owns the rounded surface and
        // the resize bands inside that outline.
        let radii = ui::frame::inner_corner_radii(window, cx);
        // Vue's titlebar is a lighter strip with `box-shadow: 0 0 2px`; GPUI
        // paints later siblings over earlier ones, so the shadow is a gradient
        // strip drawn after the body instead of a shadow on the bar itself.
        let shadow = ui::surfaces(self.appearance_dark).titlebar_border;
        let frame = ui::frame::window_frame(
            v_flex()
                .size_full()
                .text_size(px(14.))
                .text_color(foreground)
                .overflow_hidden()
                .rounded_tl(radii.top_left)
                .rounded_tr(radii.top_right)
                .rounded_bl(radii.bottom_left)
                .rounded_br(radii.bottom_right)
                .bg(background)
                .child(ui::titlebar::titlebar(self, window, cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(ui::sidebar::sidebar(self, window, cx))
                        .when(!self.sidebar_collapsed, |row| {
                            row.child(ui::sidebar::resize_band(self.sidebar_drag.is_some(), cx))
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .h_full()
                                .min_w_0()
                                .min_h_0()
                                .child(if let Some(panel) = ui::file_editor::panel(self, cx) {
                                    panel
                                } else {
                                    v_flex()
                                        .flex_1()
                                        .min_h_0()
                                        .child(
                                            v_flex().relative().flex_1().min_h_0().min_w_0()
                                                .child(content)
                                                .child(self.timeline_scroll.scrollbar_input.overlay(&self.timeline_scroll.scroll))
                                                .children(self.child_overlay(cx)),
                                        )
                                        .child(ui::composer::composer(self, cx))
                                        .into_any_element()
                                })
                                .children(ui::terminal::render(self, cx)),
                        ),
                )
                .child(
                    div()
                        .id("titlebar-shadow")
                        .absolute()
                        .top(px(ui::TITLEBAR_HEIGHT - 1.))
                        .left(px(1.))
                        .right(px(1.))
                        .h(px(3.))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(shadow.opacity(0.55), 0.),
                            linear_color_stop(shadow.opacity(0.), 1.),
                        )),
                ),
            window,
            cx,
        );
        // Configuration dialogs float above the whole frame instead of growing
        // the composer, like Vue's appended-to-body popovers.
        let mut shell = div().size_full().relative().child(frame);
        if self.quick_panel.is_some() {
            shell = shell.child(ui::composer::panels::overlay(self, cx));
        }
        if let Some(toast) = &self.toast {
            shell = shell.child(toast.render(cx));
        }
        shell
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start in the system language; the runtime setting takes over on connect.
    rust_i18n::set_locale(chatspeed_gpui::prefs::system_locale());
    let options = Options::parse();
    let worker = Worker::start(options.clone())?;
    gpui_kit::application()
        .with_assets(ui::IconAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("ai.aidyou.chatspeed", "ChatSpeed");
            ensure_bold_capable_font(cx);
            if let Err(error) = cx
                .text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(icons::FONT)])
            {
                eprintln!("{}: {error}", t!("font_error"));
                cx.quit();
                return;
            }
            let window_options = WindowOptions {
                app_id: Some("ai.aidyou.chatspeed".into()),
                #[cfg(target_os = "linux")]
                icon: image::load_from_memory_with_format(
                    include_bytes!("../../../icons/128x128.png"),
                    image::ImageFormat::Png,
                )
                .ok()
                .map(|icon| Arc::new(icon.into_rgba8())),
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.), px(9.))),
                }),
                // The rounded frame and its shadow live inside the window bounds,
                // so everything outside that frame has to stay transparent.
                window_background: WindowBackgroundAppearance::Transparent,
                // The native titlebar draws itself, so the platform must not
                // paint a second one (Tauri runs with `decorations: false`).
                // macOS keeps its own decorations and only hands us the
                // traffic lights, so the flag stays unset there.
                window_decorations: if cfg!(target_os = "macos") {
                    None
                } else {
                    Some(WindowDecorations::Client)
                },
                app_owns_titlebar_drag: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    WORKFLOW_WINDOW_SIZE,
                    cx,
                ))),
                window_min_size: Some(WORKFLOW_WINDOW_MIN_SIZE),
                ..Default::default()
            };
            if gpui_kit::open_window(window_options, cx, move |window, cx| {
                cx.new(|cx| Observer::new(worker, options, window, cx))
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

#[cfg(test)]
mod tests {
    use super::{attachment_context, attachment_metadata, is_image_path, ToolDraft};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn tool_draft_accepts_both_mcp_serializations() {
        let config = json!({
            "availableTools": ["bash", "server__MCP__tool"],
            "autoApprove": ["bash"],
            "mcpTools": {
                "available": ["server__MCP__tool"],
                "auto_approve": ["server__MCP__tool"],
                "auto_expand": ["server__MCP__tool"],
            }
        });
        let draft = ToolDraft::from_config(&config);
        assert!(draft.available.contains("bash"));
        assert!(draft.auto_approve.contains("bash"));
        assert!(draft.mcp_available.contains("server__MCP__tool"));
        assert!(draft.mcp_auto_approve.contains("server__MCP__tool"));
        assert!(draft.mcp_auto_expand.contains("server__MCP__tool"));
    }

    #[test]
    fn attachments_accept_only_images_and_keep_their_paths() {
        assert!(is_image_path(&PathBuf::from("/tmp/shot.PNG")));
        assert!(!is_image_path(&PathBuf::from("/tmp/notes.txt")));

        let attachments = vec![super::Attachment {
            path: PathBuf::from("/tmp/shot.png"),
            name: "shot.png".into(),
        }];
        let context = attachment_context(&attachments).expect("context");
        assert!(context.contains("/tmp/shot.png"));
        let metadata = attachment_metadata(&attachments).expect("metadata");
        assert_eq!(metadata["attachments"][0]["name"], "shot.png");
        assert!(attachment_context(&[]).is_none());
    }
}
