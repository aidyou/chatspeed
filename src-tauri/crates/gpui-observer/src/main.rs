//! Native workflow window: titlebar, compact sidebar, composer and transcript.
//!
//! Visual rules mirror `src/views/Workflow.vue`, `src/components/window/Titlebar.vue`,
//! `src/components/workflow/WorkflowMessageList.vue` and
//! `src/styles/workflow/messages.scss`: a CLI-style tool transcript, grouped
//! tool activity, collapsible snapshots and delegated-task cards.

use crate::ui::Icon;
use chatspeed_gpui_observer::{
    chimes::Chime,
    icons,
    metrics::Metrics,
    model::Model,
    prefs::{self, normalize_locale, primary_preset, DEFAULT_PRIMARY},
    presentation::{
        self, format_duration, grouped_number, ActivityItem, ActivityView, AlertKind, AlertView,
        AssistantView, ChoiceGroupView, CostTotals, CostView, FinishTaskView, Row, SnapshotView,
        SubAgentStatus, SubAgentView, ToolView, UserView,
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
/// Vue's collapsed user bubble: four 1.6-line-height rows at 14px, plus room
/// for the corner toggle.
const USER_MESSAGE_LINES: usize = 4;
const USER_MESSAGE_COLLAPSED_HEIGHT: f32 = 114.;

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

/// A bordered summary value inside the cost card.
fn cost_chip(label: String, accent: Hsla, cx: &mut App) -> AnyElement {
    div()
        .flex_none()
        .min_h(px(26.))
        .px_2()
        .py(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(accent.opacity(0.22))
        .bg(cx.theme().background.opacity(0.92))
        .text_size(px(12.))
        .text_color(cx.theme().foreground)
        .child(label)
        .into_any_element()
}

/// Money values use the accent color, monospace, and a heavier weight.
fn cost_amount(value: Option<f64>, accent: Hsla) -> AnyElement {
    div()
        .flex_none()
        .font_family(MONO_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(accent)
        .child(format_cost(value))
        .into_any_element()
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
    /// Last repaint that refreshed the live status line's elapsed seconds.
    last_tick: Instant,
    /// `theme` runtime setting: `system`, `light`, or `dark`.
    theme_setting: String,
    /// Last appearance the theme was resolved against, to catch system switches.
    appearance_dark: bool,
    applied_primary: u32,
    palette_applied: bool,
    /// Size to apply on the next frame, from the remembered workflow geometry.
    pending_window_size: Option<Size<Pixels>>,
    /// Remembered size already applied, so the periodic config poll cannot
    /// override a size the user picked in this window.
    applied_window_size: Option<Size<Pixels>>,
    copy_flash: Arc<Mutex<Option<CopyFlash>>>,
    /// Sidebar collapse state, matching the Vue compact sidebar.
    sidebar_collapsed: bool,
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
    /// The first authorized root opens once per selected workflow, matching
    /// `FileTree.vue`'s mount behaviour.
    files_auto_expanded: bool,
    /// A titlebar press that became a window move on the first pointer motion.
    title_drag: bool,
    /// Multi-line composer state; sends read it and clear it.
    composer: Entity<TextareaState>,
    /// Whether the composer owns focus, mirrored from its input events.
    composer_focused: bool,
    /// Vue's fullscreen toggle: the box grows instead of the transcript.
    composer_expanded: bool,
    /// A new session is being created and started; further sends wait.
    start_pending: bool,
    /// Last action failure, shown above the composer until the next send.
    notice: Option<String>,
    /// Child transcripts loaded for expanded sub-agent cards, by task id.
    child_rows: BTreeMap<String, Vec<Row>>,
    /// Selected option per `ask_user` group, keyed by `tool_call_id:group`.
    ask_user_choice: BTreeMap<String, usize>,
    /// Free text typed for an `ask_user` "Other" row, by the same key.
    ask_user_text: BTreeMap<String, Entity<TextareaState>>,
    /// Child transcripts requested but not loaded yet.
    child_pending: BTreeSet<String>,
    /// Agents, model providers and skills the composer selectors list.
    agents: Vec<serde_json::Value>,
    models: Vec<serde_json::Value>,
    skills: Vec<serde_json::Value>,
    /// Sandbox schemes the execution-mode popover can pick from.
    sandbox_schemes: Vec<serde_json::Value>,
    /// The `@` query the file panel answered, so a stale list is never shown.
    file_query: String,
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
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => {
                    // Enter completes the open `@`/`/` panel before it sends.
                    if !ui::composer::suggestions::apply_first(this, window, cx) {
                        this.send_message(window, cx);
                    }
                }
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
        let observer = Self {
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
            last_tick: Instant::now(),
            theme_setting: "system".into(),
            appearance_dark: false,
            applied_primary: DEFAULT_PRIMARY,
            palette_applied: false,
            pending_window_size: None,
            applied_window_size: None,
            copy_flash: Arc::new(Mutex::new(None)),
            sidebar_collapsed: false,
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
            files_auto_expanded: false,
            title_drag: false,
            composer,
            composer_focused: false,
            composer_expanded: false,
            start_pending: false,
            notice: None,
            child_rows: BTreeMap::new(),
            ask_user_choice: BTreeMap::new(),
            ask_user_text: BTreeMap::new(),
            child_pending: BTreeSet::new(),
            agents: Vec::new(),
            models: Vec::new(),
            skills: Vec::new(),
            sandbox_schemes: Vec::new(),
            file_query: String::new(),
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
        self.ask_user_choice.clear();
        self.ask_user_text.clear();
        self.follow = true;
        self.switched = Some(Instant::now());
        self.scroll = ScrollHandle::new();
        self.prepend_anchor = None;
        self.deferred_update = None;
        self.compression_started = None;
        self.clear_dir_state();
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
        self.sidebar_collapsed = !self.sidebar_collapsed;
        // Vue drops the root filter when the panel collapses.
        if self.sidebar_collapsed {
            self.session_root_filter = None;
        }
        cx.notify();
    }

    /// Rail selection also picks the list tab Vue shows next to it.
    fn select_rail_tab(&mut self, tab: ui::sidebar::RailTab, cx: &mut Context<Self>) {
        self.rail_tab = tab;
        self.sidebar_tab = match tab {
            ui::sidebar::RailTab::Tasks => ui::sidebar::SidebarTab::Tasks,
            ui::sidebar::RailTab::Automation => ui::sidebar::SidebarTab::Automation,
        };
        cx.notify();
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
        self.sidebar_collapsed = false;
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

    /// Pending decisions of the selected workflow: (tool call, tool name).
    fn pending_approvals(&self) -> Vec<(String, String)> {
        self.model
            .pending_tool_ids
            .iter()
            .chain(
                self.model
                    .awaiting_user_tool_call_id
                    .as_ref()
                    .filter(|id| !self.model.pending_tool_ids.contains(*id)),
            )
            .filter_map(|id| {
                let tool = self.model.tools.get(id)?;
                Some((id.clone(), presentation::tool_name(tool).to_owned()))
            })
            .collect()
    }

    fn desktop_menu_action(
        &mut self,
        action: chatspeed_runtime_client::desktop_menu::Action,
        cx: &mut Context<Self>,
    ) {
        self.notice = self
            .worker
            .commands
            .try_send(Command::DesktopMenu(action))
            .err()
            .map(|_| t!("desktop_menu_unavailable").to_string());
        cx.notify();
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
            self.notice = Some(t!("sound_save_failed").to_string());
        }
        cx.notify();
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.pinned = !self.pinned;
        // GPUI exposes no always-on-top request, so the switch only reports the
        // state the titlebar is asked for.
        self.notice = self
            .pinned
            .then(|| format!("{} · {}", t!("pin"), t!("not_supported_native")));
        cx.notify();
    }

    fn not_implemented(&mut self, feature: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(format!(
            "{} · {}",
            feature.into(),
            t!("not_supported_native")
        ));
        cx.notify();
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

    /// The model slot the composer edits: Vue uses `act` outside planning mode.
    fn active_model_config(&self) -> serde_json::Value {
        let models = self.agent_config()["models"].clone();
        for key in ["act", "plan"] {
            if models[key]["model"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
            {
                return models[key].clone();
            }
        }
        serde_json::json!({})
    }

    fn model_configs(&self) -> serde_json::Value {
        self.agent_config()["models"].clone()
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
        let provider = self.active_provider();
        if config["id"].as_i64().unwrap_or_default() == 0 {
            return model.to_owned();
        }
        provider
            .and_then(|provider| {
                provider["models"]
                    .as_array()?
                    .iter()
                    .find(|entry| entry["id"].as_str() == Some(model))
                    .and_then(|entry| entry["name"].as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| model.to_owned())
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
        config["sandboxSchemeId"]
            .as_str()
            .or_else(|| config["sandbox_scheme_id"].as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
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
        self.agent_bool("autoCompress", "auto_compress", true)
    }

    fn final_audit_enabled(&self) -> bool {
        self.agent_bool("finalAudit", "final_audit", false)
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
            self.notice = Some(t!("clear_context_no_session").to_string());
            cx.notify();
            return;
        };
        if self
            .worker
            .commands
            .try_send(Command::ClearContext { session })
            .is_err()
        {
            self.notice = Some(t!("clear_context_failed").to_string());
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
        self.model.select();
        self.selected = None;
        self.rows.clear();
        self.markdown.clear();
        self.expanded.clear();
        self.child_rows.clear();
        self.ask_user_choice.clear();
        self.ask_user_text.clear();
        self.notice = None;
        self.follow = true;
        self.scroll_pending = true;
        self.clear_dir_state();
        self.composer.update(cx, |state, cx| {
            state.set_value("", window, cx);
            // Typing the first message is the next step, so take the focus there.
            let focus = state.focus_handle(cx);
            focus.focus(window, cx);
        });
        cx.notify();
    }

    /// Sends the composer text, creating the session first when none is selected.
    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() || self.start_pending {
            return;
        }
        self.notice = None;
        match self.selected.clone() {
            Some(id) => {
                let _ = self.worker.commands.try_send(Command::Signal {
                    id,
                    signal: serde_json::json!({"type": "user_message", "content": text}),
                });
                self.follow = true;
                self.model.resume_follow();
                self.scroll_pending = true;
            }
            None => {
                self.start_pending = true;
                let _ = self
                    .worker
                    .commands
                    .try_send(Command::StartNew { prompt: text });
            }
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    fn stop_workflow(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            let _ = self.worker.commands.try_send(Command::Stop { id });
            cx.notify();
        }
    }

    fn toggle_composer_expanded(&mut self, cx: &mut Context<Self>) {
        self.composer_expanded = !self.composer_expanded;
        cx.notify();
    }

    /// Vue's "Continue" is offered for paused, failed, cancelled and errored
    /// workflows; the runtime resumes from that signal.
    fn can_continue(&self) -> bool {
        self.selected.is_some()
            && matches!(
                self.model.state.as_str(),
                "paused" | "error" | "failed" | "cancelled"
            )
    }

    fn continue_workflow(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            let _ = self.worker.commands.try_send(Command::Signal {
                id,
                signal: serde_json::json!({"type": "continue"}),
            });
            self.follow = true;
            self.model.resume_follow();
            self.scroll_pending = true;
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
        let _ = self.worker.commands.try_send(Command::Signal {
            id: session,
            signal: serde_json::json!({
                "type": "approval",
                "id": tool_call_id,
                "approved": approved,
                "approve_all": approve_all,
            }),
        });
        cx.notify();
    }

    /// True while the runtime waits on this `ask_user` call for an answer.
    fn ask_user_active(&self, tool: &ToolView) -> bool {
        !tool.tool_call_id.is_empty()
            && self.model.awaiting_user_tool_call_id.as_deref() == Some(tool.tool_call_id.as_str())
    }

    fn set_ask_user_choice(&mut self, key: String, option: usize, cx: &mut Context<Self>) {
        self.ask_user_choice.insert(key, option);
        cx.notify();
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
        let state = match self.ask_user_text.get(&key) {
            Some(state) => state.clone(),
            None => {
                let state = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(1, 6)
                        .placeholder(t!("ask_user_other_placeholder").to_string())
                });
                self.ask_user_text.insert(key, state.clone());
                state
            }
        };
        state.update(cx, |state, cx| {
            state.focus_handle(cx).focus(window, cx);
        });
        cx.notify();
    }

    /// Submits the drafted answer as the hidden `user_message` the engine reads.
    fn submit_ask_user(&mut self, tool: ToolView, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone() else {
            return;
        };
        if !self.ask_user_active(&tool) {
            return;
        }
        let supplements: Vec<String> = tool
            .choices
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.ask_user_text
                    .get(&format!("{}:{}", tool.tool_call_id, index))
                    .map(|state| state.read(cx).value().to_string())
                    .unwrap_or_default()
            })
            .collect();
        let drafts: Vec<presentation::AskUserDraft<'_>> = tool
            .choices
            .iter()
            .enumerate()
            .map(|(index, _)| presentation::AskUserDraft {
                choice: self
                    .ask_user_choice
                    .get(&format!("{}:{}", tool.tool_call_id, index))
                    .copied(),
                supplement: supplements[index].as_str(),
            })
            .collect();
        let content = match presentation::ask_user_response(&tool.choices, &drafts) {
            Ok(content) => content,
            Err(error) => {
                self.notice = Some(
                    match error {
                        presentation::AskUserDraftError::OtherRequired => {
                            t!("ask_user_other_required")
                        }
                        presentation::AskUserDraftError::Required => t!("ask_user_required"),
                    }
                    .to_string(),
                );
                cx.notify();
                return;
            }
        };
        let _ = self.worker.commands.try_send(Command::Signal {
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
        });
        self.notice = None;
        cx.notify();
    }

    /// Expands a sub-agent card and requests its child transcript once.
    fn toggle_sub_agent(&mut self, card: SubAgentView, cx: &mut Context<Self>) {
        if self.expanded.remove(&card.key) {
            cx.notify();
            return;
        }
        self.expanded.insert(card.key.clone());
        if let Some(task_id) = card.task_id {
            if !self.child_rows.contains_key(&task_id) && self.child_pending.insert(task_id.clone())
            {
                let _ = self.worker.commands.try_send(Command::Child {
                    id: task_id.clone(),
                    session: task_id,
                });
            }
        }
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
                UpdateKind::Config(config, fetched_at) => {
                    self.apply_config(&config, fetched_at, cx);
                }
                UpdateKind::DesktopMenuUnavailable => {
                    self.notice = Some(t!("desktop_menu_unavailable").to_string());
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
                        self.notice = Some(t!("sound_save_failed").to_string());
                    }
                }
                UpdateKind::Chime(chime) => {
                    if self.sound_is_enabled(chime) {
                        if let Some(player) = &self.sound_player {
                            player.play(chime);
                        } else {
                            self.notice = Some(t!("sound_playback_unavailable").to_string());
                            cx.notify();
                        }
                    }
                    continue;
                }
                UpdateKind::Opened(id) => {
                    self.start_pending = false;
                    self.select(id, cx);
                }
                UpdateKind::Child(id, page) => {
                    self.child_pending.remove(&id);
                    self.child_rows
                        .insert(id, presentation::project(page.messages, self.roots()));
                }
                UpdateKind::Error(message) => {
                    self.start_pending = false;
                    self.notice = Some(message);
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
                UpdateKind::Toast(message) => {
                    self.notice = Some(message);
                    cx.notify();
                }
                UpdateKind::Files { query, files } => {
                    // Only the newest query may paint the `@` panel.
                    if query == self.file_query {
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
                UpdateKind::Snapshot(page) => {
                    self.model.page(page, false);
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
        if self
            .sound_player
            .as_ref()
            .is_some_and(|player| player.playback_failed())
        {
            self.notice = Some(t!("sound_playback_unavailable").to_string());
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
            // The catalogue is fetched at startup, which can precede the
            // runtime connection; keep asking until it answers.
            if self.agents.is_empty() {
                self.request_catalogue();
            }
        }
    }

    /// Mirrors the runtime preferences shared with the Vue titlebar.
    fn apply_config(&mut self, config: &Value, fetched_at: Instant, cx: &mut Context<Self>) {
        self.show_menu_button = config["show_menu_button"].as_bool().unwrap_or(true);
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
            .unwrap_or_else(chatspeed_gpui_observer::prefs::system_locale);
        if *rust_i18n::locale() != *language {
            rust_i18n::set_locale(language);
            self.rows = presentation::project(self.transcript(), self.roots());
            self.refresh_markdown(cx);
        }
        set_primary(config["primary_color"].as_str().unwrap_or("green"));
        self.adopt_window_size(config);
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
        let max = self.scroll.max_offset();
        let offset = self.scroll.offset();
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
        // Expanding a card grows the transcript under the pointer, so Vue keeps
        // the newly revealed body visible; following the tail is the same move.
        if self.follow {
            self.scroll_pending = true;
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
            .child(
                Icon::new(if expanded {
                    "double-arrow-up"
                } else {
                    "double-arrow-down"
                })
                .size(12.)
                .color(cx.theme().muted_foreground),
            )
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
                    .h(px(24.))
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .child(Icon::new("reasoning").size(14.).color(muted))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(13.))
                            .line_height(px(24.))
                            .text_color(muted)
                            .italic()
                            .child(title),
                    )
                    .child(
                        Icon::new(if open { "caret-down" } else { "caret-right" })
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
        // A pending approval or an unanswered `ask_user` keeps its body open,
        // exactly like Vue's `isApprovalPending || isMessageExpanded`.
        let pending = !tool.tool_call_id.is_empty()
            && (tool.status == presentation::ToolStatus::Pending
                || self.model.pending_tool_ids.contains(&tool.tool_call_id));
        let answering = self.ask_user_active(tool);
        let open = pending || answering || self.expanded.contains(&tool.detail_key);
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
        } else if pending {
            // The approval card owns the diff/command/plan while pending, so the
            // regular detail would only repeat it.
            body = body.child(
                div()
                    .w_full()
                    .min_w_0()
                    .pl_6()
                    .pb_2()
                    .child(ui::approval::approval_card(tool, cx)),
            );
        } else {
            body = body.child(self.tool_detail(tool, cx));
        }
        body.into_any_element()
    }

    fn tool_detail(&self, tool: &ToolView, cx: &mut Context<Self>) -> AnyElement {
        let mut detail = v_flex().w_full().min_w_0().gap_2().pl_6().pt_2().pb_2();
        let answering = self.ask_user_active(tool);
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
            .h(px(26.))
            .pl_2()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .child(Icon::new(group.icon).size(15.).color(muted));
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
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .line_height(px(26.))
                    .text_color(if group.ongoing {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(group.summary.clone()),
            )
            .child(
                Icon::new(if open { "caret-down" } else { "caret-right" })
                    .size(14.)
                    .color(muted),
            )
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
        let card = card.clone();
        let click_card = card.clone();
        let open = self.expanded.contains(&card.key);
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
            .child(Icon::new("skill-relation-chart").size(15.).color(muted))
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
            .child(
                Icon::new(if open { "caret-down" } else { "caret-right" })
                    .size(14.)
                    .color(muted),
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_sub_agent(click_card.clone(), cx)),
            );
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
                            .child(Icon::new("task").size(15.).color(warning()))
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
            // The expanded card shows the delegated transcript with the same
            // renderer as the main list, like Vue's child message pane.
            if let Some(rows) = card.task_id.as_ref().and_then(|id| self.child_rows.get(id)) {
                if !rows.is_empty() {
                    let mut transcript = v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .mt_2()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background);
                    for row in rows {
                        transcript =
                            transcript.child(div().w_full().min_w_0().child(self.row(row, cx)));
                    }
                    body = body.child(transcript);
                }
            } else if card.task_id.is_some() {
                body = body.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(Icon::new("loading").size(12.).color(muted))
                        .child(t!("sub_agent_loading").to_string()),
                );
            }
        }
        body.into_any_element()
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
        for (option_index, option) in group.options.iter().enumerate() {
            let label = format!("{}. {option}", option_index + 1);
            if active {
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
            } else {
                options = options.child(choice_option(
                    label,
                    group.selected == Some(option_index),
                    cx,
                ));
            }
        }
        let other_label = format!("{}. {}", other_index + 1, t!("ask_user_other"));
        if active {
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
            // Vue shows the free-text field under the picked "Other" row; the
            // field is created the first time that row is pressed.
            if self.ask_user_choice.get(&base).copied() == Some(other_index) {
                if let Some(state) = self.ask_user_text.get(&base) {
                    options = options.child(
                        div()
                            .w_full()
                            .pl(px(22.))
                            .pt_1()
                            .child(Textarea::new(state).appearance(false).h(px(34.))),
                    );
                }
            }
        } else {
            options = options.child(choice_option(other_label, group.other_selected, cx));
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
            .when(open, |view| view.child(self.cost_card(cost, cx)))
            .into_any_element()
    }

    fn cost_card(&self, cost: &CostView, cx: &mut Context<Self>) -> AnyElement {
        // Mirrors `.workflow-cost-analysis`: an accent rail, a left-to-right
        // accent wash over the muted surface, and chip-style summary values.
        let accent: Hsla = accent().into();
        let mut body = v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .px(px(20.))
            .py(px(16.))
            .bg(linear_gradient(
                90.,
                linear_color_stop(accent.opacity(0.13), 0.),
                linear_color_stop(accent.opacity(0.), 0.42),
            ))
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(cost_chip(format_duration(cost.duration_ms), accent, cx))
                    .child(cost_chip(
                        format!(
                            "{} {}",
                            grouped_number(cost.with_sub_agents.total_tokens),
                            t!("cost_tokens")
                        ),
                        accent,
                        cx,
                    )),
            )
            .child(self.cost_row(t!("cost_self").to_string(), &cost.self_usage, true, cx));
        if cost.has_sub_agents {
            body = body.child(self.cost_row(
                t!("cost_with_sub_agents").to_string(),
                &cost.with_sub_agents,
                false,
                cx,
            ));
        }
        if !cost.models.is_empty() {
            let mut models = v_flex().w_full().min_w_0().gap_2().pt_2();
            for model in &cost.models {
                let mut tokens = h_flex().w_full().flex_wrap().gap_x_3().gap_y_1();
                for (label, value) in [
                    (t!("cost_input").to_string(), model.input_tokens),
                    (t!("cost_cache").to_string(), model.cache_tokens),
                    (t!("cost_output").to_string(), model.output_tokens),
                ] {
                    tokens = tokens.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(cx.theme().foreground)
                            .child(format!("{label} {}", grouped_number(value))),
                    );
                }
                if let Some(rate) = model.cache_hit_rate {
                    tokens = tokens.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(cx.theme().foreground)
                            .child(format!(
                                "{} {}",
                                t!("cost_cache_hit_rate"),
                                format_percent(rate)
                            )),
                    );
                }
                let mut rates = h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_x_3()
                    .gap_y_1()
                    .opacity(0.86);
                for (label, rate) in [
                    (t!("cost_input").to_string(), model.input_per_million),
                    (t!("cost_cache").to_string(), model.cache_per_million),
                    (t!("cost_output").to_string(), model.output_per_million),
                ] {
                    rates = rates.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(cx.theme().foreground)
                            .child(format!("{label} {}", format_rate(rate))),
                    );
                }
                if let Some(multiplier) = model.multiplier {
                    rates = rates.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(cx.theme().foreground)
                            .child(format!("×{multiplier}")),
                    );
                }
                models = models.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .rounded(px(12.))
                        .bg(cx.theme().background.opacity(0.92))
                        .px(px(16.))
                        .py_2()
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_baseline()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().foreground)
                                        .child(model.backend_model.clone()),
                                )
                                .child(cost_amount(model.estimated_cost, accent)),
                        )
                        .child(tokens)
                        .child(rates),
                );
            }
            body = body.child(
                models
                    .border_t_1()
                    .border_dashed()
                    .border_color(accent.opacity(0.28)),
            );
        }
        if cost.is_partial {
            body = body.child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(warning())
                    .child(t!("cost_partial").to_string()),
            );
        }
        // Vue paints the rail with `inset 3px 0 0`, which CSS clips to the
        // border radius. GPUI clips children to a square rect, so the rail
        // squared off the card's left corners; the accent wash and border
        // carry the same emphasis without breaking the radius.
        h_flex()
            .relative()
            .w_full()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(accent.opacity(0.24))
            .bg(cx.theme().muted)
            .overflow_hidden()
            .child(body)
            .into_any_element()
    }

    fn cost_row(
        &self,
        label: String,
        totals: &CostTotals,
        primary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent: Hsla = accent().into();
        let color = if primary {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(label),
                    )
                    .child(cost_amount(totals.estimated_cost, accent)),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(color)
                    .opacity(0.86)
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
                    .child(Icon::new("compress").size(14.).color(accent()))
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
                    .child(
                        Icon::new(if open {
                            "double-arrow-up"
                        } else {
                            "double-arrow-down"
                        })
                        .size(14.)
                        .color(muted),
                    )
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
            .child(Icon::new(icon).size(15.).color(color))
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
                .child(
                    Icon::new(if open {
                        "double-arrow-up"
                    } else {
                        "double-arrow-down"
                    })
                    .size(12.)
                    .color(cx.theme().muted_foreground),
                )
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
                    .child(Icon::new(icon).size(18.).color(color))
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
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                if this.selected.is_none() {
                    return;
                }
                // Landing near the tail resumes following, so reading history is a
                // pause rather than a mode the user has to leave manually.
                if this.near_bottom() {
                    if !this.follow {
                        this.follow = true;
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
        ui::frame::window_frame(
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
                                .child(content)
                                .child(ui::composer::composer(self, cx)),
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
        )
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start in the system language; the runtime setting takes over on connect.
    rust_i18n::set_locale(chatspeed_gpui_observer::prefs::system_locale());
    let options = Options::parse();
    let worker = Worker::start(options.clone())?;
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
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
