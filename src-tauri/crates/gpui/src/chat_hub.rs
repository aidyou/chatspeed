//! Native child browser for the GPUI ChatHub dock.
//!
//! Vue keeps one page per chat site and shows the active one in a shared right
//! dock; the GPUI window does the same with one wry child view per tab.

use std::{
    cell::Cell,
    fs::File,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{canvas, div, prelude::*, AnyElement, Context, Window};
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

pub(crate) const DOCK_WIDTH: f32 = 600.;
/// Narrowest docked page. 375 is the logical width of a common phone, so a page
/// still renders its mobile layout when the splitter is dragged all the way in.
pub(crate) const DOCK_MIN_WIDTH: f32 = 375.;
/// Width the workflow UI always keeps beside the page, mirroring the Tauri
/// carrier's `CHAT_HUB_MIN_HOST_WIDTH`.
pub(crate) const DOCK_MIN_HOST_WIDTH: f32 = 480.;
pub(crate) const SPLITTER_WIDTH: f32 = 5.;
/// How long the docked column takes to slide in or out.
const SLIDE: Duration = Duration::from_millis(180);
// The native resize can land after the animation's last request. Stop waiting
// if the compositor keeps the window at a different size.
const RESIZE_SETTLE: Duration = Duration::from_millis(180);

/// Clamps a requested page width the way the Tauri carrier does.
pub(crate) fn clamp_width(window_width: f32, requested: f32) -> f32 {
    if !window_width.is_finite() || !requested.is_finite() {
        return DOCK_WIDTH;
    }
    let maximum = (window_width - DOCK_MIN_HOST_WIDTH - SPLITTER_WIDTH).max(DOCK_MIN_WIDTH);
    requested.clamp(DOCK_MIN_WIDTH, maximum)
}

/// Fit the page into the space to the right if it has room for the minimum
/// width. Only borrow from the workflow when the screen cannot fit that.
fn expand_plan(window_width: f32, available: f32, requested: f32) -> (f32, f32) {
    let available = available.max(0.);
    let width = clamp_width(window_width + available.min(requested + SPLITTER_WIDTH), requested);
    let width = width.min((available - SPLITTER_WIDTH).max(DOCK_MIN_WIDTH));
    (width, available.min(width + SPLITTER_WIDTH))
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_expands_to_the_width_the_slide_targets() {
    for (window_width, available, requested) in [
        (1600., 900., 900.),
        (1600., 2000., 900.),
        (1000., 300., 900.),
        (1000., 0., 900.),
        (700., 200., 900.),
    ] {
        let (width, growth) = expand_plan(window_width, available, requested);
        // The clamp after the expansion must be a no-op, or the page would
        // visibly change width once the slide has already finished.
        assert_eq!(clamp_width(window_width + growth, width), width);
        assert!(width >= DOCK_MIN_WIDTH);
        assert!(growth <= available.max(0.));
        assert!(growth <= width + SPLITTER_WIDTH);
        if available >= DOCK_MIN_WIDTH + SPLITTER_WIDTH {
            assert_eq!(growth, width + SPLITTER_WIDTH);
        }
    }
    assert_eq!(expand_plan(1000., 450., 900.), (445., 450.));
    assert_eq!(expand_plan(1000., 300., 900.), (DOCK_MIN_WIDTH, 300.));
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_width_keeps_a_phone_page_and_the_workflow_ui_visible() {
    assert_eq!(clamp_width(1600., 900.), 900.);
    assert_eq!(clamp_width(1600., 2000.), 1115.);
    assert_eq!(clamp_width(1600., 100.), DOCK_MIN_WIDTH);
    // A narrow window cannot honor both limits; the page floor wins.
    assert_eq!(clamp_width(700., 900.), DOCK_MIN_WIDTH);
    assert_eq!(clamp_width(f32::NAN, 640.), DOCK_WIDTH);
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_slide_eases_from_closed_to_open() {
    let running = Reveal {
        started: Instant::now(),
        from: 0.,
        to: 600.,
        closing: false,
    };
    assert!((0. ..=600.).contains(&running.width()));
    let landed = Reveal {
        started: Instant::now() - SLIDE,
        from: 0.,
        to: 600.,
        closing: false,
    };
    assert!(landed.done(false));
    assert!(!landed.done(true));
    assert_eq!(landed.width(), 600.);
    let capped = Reveal {
        started: Instant::now() - SLIDE - RESIZE_SETTLE,
        ..landed
    };
    assert!(capped.done(true));
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_layout_follows_observed_growth_instead_of_the_animation_target() {
    let viewport = 1000.;
    let total = 600. + SPLITTER_WIDTH;
    let growth = WindowGrowth {
        base_window: 1040.,
        base_viewport: viewport,
        requested: total,
    };
    for (added, share) in [
        (0., 0.3),
        (2., 0.6),
        (240., 0.9),
        (560., 1.),
        (total, 1.),
        (180., 0.),
        (0., 0.),
    ] {
        let current = viewport + added;
        let reserved = growth.layout_extent(current, total, share);
        let splitter = reserved.min(SPLITTER_WIDTH);
        let page = (reserved - SPLITTER_WIDTH).max(0.);
        assert_eq!(current - splitter - page, viewport);
    }
    let native_target = growth.base_window + total;
    assert!(growth.resize_pending(native_target, viewport + 560., false));
    assert!(!growth.resize_pending(native_target, viewport + total, false));
    assert!(growth.resize_pending(growth.base_window, viewport + 80., true));
    assert!(!growth.resize_pending(growth.base_window, viewport, true));
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_layout_borrows_only_the_width_needed_for_the_minimum_page() {
    let (width, requested) = expand_plan(1000., 300., 600.);
    let total = width + SPLITTER_WIDTH;
    let growth = WindowGrowth {
        base_window: 1000.,
        base_viewport: 1000.,
        requested,
    };
    assert_eq!(growth.layout_extent(1000., total, 0.), 0.);
    assert_eq!(growth.layout_extent(1090., total, 0.5), 130.);
    assert_eq!(growth.layout_extent(1300., total, 1.), total);
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_close_starts_at_the_width_left_by_the_splitter() {
    let growth = WindowGrowth {
        base_window: 1000.,
        base_viewport: 1000.,
        requested: 600.,
    };
    assert_eq!(growth.layout_extent(1600., 400., 1.), 400.);
    assert_eq!(growth.layout_extent(1300., 400., 0.5), 200.);
    assert_eq!(growth.layout_extent(1000., 400., 0.), 0.);
}

// Wry's navigation handler cannot distinguish a link click from a redirect.
// Route external anchors through the native new-window handler instead, without
// exposing an IPC bridge to the site or taking over its own navigation.
const OPEN_LINKS_IN_BROWSER: &str = r#"
window.addEventListener('click', function (event) {
    if (!event.isTrusted || event.defaultPrevented || event.button !== 0) return;
    const link = event.composedPath().find(function (node) {
        return node instanceof HTMLAnchorElement && node.hasAttribute('href');
    });
    if (!link || link.hasAttribute('download')) return;
    if (link.protocol !== 'https:' && link.protocol !== 'http:') return;
    if (link.origin === location.origin) return;
    event.preventDefault();
    window.open(link.href, '_blank', 'noopener,noreferrer');
});
"#;

#[cfg(target_os = "macos")]
pub(crate) fn install_edit_menu(cx: &mut gpui_kit::App) {
    use gpui_kit::{component::input::{Copy, Cut, Paste, SelectAll}, Menu, MenuItem, OsAction};
    use rust_i18n::t;

    // Wry child views leave Command shortcuts to AppKit. Keep the edit menu
    // native so WebKit can use the original pasteboard, including images.
    cx.set_menus([
        Menu::new("ChatSpeed"),
        Menu::new(t!("menu_edit").to_string()).items([
            MenuItem::os_action(t!("menu_cut").to_string(), Cut, OsAction::Cut),
            MenuItem::os_action(t!("menu_copy").to_string(), Copy, OsAction::Copy),
            MenuItem::os_action(t!("menu_paste").to_string(), Paste, OsAction::Paste),
            MenuItem::os_action(t!("menu_select_all").to_string(), SelectAll, OsAction::SelectAll),
        ]),
    ]);
    install_edit_shortcut_monitor();
}

#[cfg(target_os = "macos")]
fn webview_has_keyboard_focus(view: &WebView) -> bool {
    use objc2_app_kit::NSView;
    use wry::WebViewExtMacOS;

    let webview = view.webview();
    let native: &NSView = &webview;
    // WebKit focuses an internal view, which need not include WKWebView
    // itself in its responder chain.
    native.window().and_then(|window| window.firstResponder())
        .and_then(|responder| responder.downcast::<NSView>().ok())
        .is_some_and(|responder| responder.isDescendantOf(native))
}

#[cfg(target_os = "macos")]
thread_local! {
    /// Only ChatHub's child views participate in the native shortcut bridge.
    static EDIT_VIEWS: std::cell::RefCell<Vec<std::rc::Weak<WebView>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[cfg(target_os = "macos")]
fn remember_edit_view(view: &Rc<WebView>) {
    EDIT_VIEWS.with_borrow_mut(|views| {
        views.retain(|view| view.strong_count() != 0);
        views.push(Rc::downgrade(view));
    });
}

/// Give focused WebKit editing shortcuts before AppKit can dispatch them to
/// stale GPUI focus. Leave unrelated app shortcuts and normal typing alone.
#[cfg(target_os = "macos")]
fn install_edit_shortcut_monitor() {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::cell::RefCell;
    use std::ptr::NonNull;

    thread_local! {
        /// AppKit calls the block until the monitor is removed; the token has to
        /// outlive the install call for the whole app run.
        static EDIT_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    EDIT_MONITOR.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes the key event this monitor was installed for.
            if dispatch_webview_command_key(unsafe { event.as_ref() }) {
                return std::ptr::null_mut();
            }
            event.as_ptr()
        });
        // SAFETY: The mask and block are valid, and app setup runs on the main thread.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
        };
        *slot.borrow_mut() = monitor;
    });
}

#[cfg(target_os = "macos")]
fn is_webview_editing_key(key: &str) -> bool {
    matches!(key, "a" | "c" | "v" | "x" | "z" | "\u{8}" | "\u{7f}" | "\u{f728}"
        | "\u{f700}" | "\u{f701}" | "\u{f702}" | "\u{f703}" | "\u{f729}" | "\u{f72b}"
        | "\u{f72c}" | "\u{f72d}")
}

#[cfg(all(test, target_os = "macos"))]
#[::core::prelude::v1::test]
fn webview_editing_keys_include_movement_and_deletion_but_not_app_shortcuts() {
    for key in ["\u{f702}", "\u{f703}", "\u{7f}", "\u{f728}", "z", "v"] {
        assert!(is_webview_editing_key(key));
    }
    for key in ["q", "w", "r", "", "\t"] {
        assert!(!is_webview_editing_key(key));
    }
}

#[cfg(target_os = "macos")]
fn dispatch_webview_command_key(event: &objc2_app_kit::NSEvent) -> bool {
    use objc2_app_kit::{NSApplication, NSEventModifierFlags as Flags, NSView};
    use wry::WebViewExtMacOS;

    let flags = event.modifierFlags();
    if !flags.contains(Flags::Command) {
        return false;
    }
    let Some(key) = event.charactersIgnoringModifiers().map(|key| key.to_string().to_ascii_lowercase()) else {
        return false;
    };
    // AppKit reports arrows and navigation keys as function-key characters.
    // Scope the bridge to editing so window and application shortcuts still work.
    if !is_webview_editing_key(&key) {
        return false;
    }
    let Some(marker) = objc2::MainThreadMarker::new() else { return false; };
    let Some(window) = event.window(marker).filter(|window| window.isKeyWindow()) else {
        return false;
    };
    let Some(responder) = window.firstResponder().and_then(|r| r.downcast::<NSView>().ok()) else {
        return false;
    };
    let view = EDIT_VIEWS.with_borrow(|views| {
        views.iter().filter_map(std::rc::Weak::upgrade).find(|view| {
            let webview = view.webview();
            let native: &NSView = &webview;
            !native.isHiddenOrHasHiddenAncestor() && responder.isDescendantOf(native)
        })
    });
    let Some(view) = view else { return false; };
    let webview = view.webview();
    let native: &NSView = &webview;
    // Keep the proven native pasteboard path (including images) for the standard
    // Edit actions. Send everything else as the original event to WebKit, so
    // it gets the modifiers, repeat state and its own editing/key bindings.
    if !flags.intersects(Flags::Control | Flags::Option | Flags::Shift | Flags::Function) {
        let action = match key.as_str() {
            "x" => Some(objc2::sel!(cut:)),
            "c" => Some(objc2::sel!(copy:)),
            "v" => Some(objc2::sel!(paste:)),
            "a" => Some(objc2::sel!(selectAll:)),
            _ => None,
        };
        if let Some(action) = action {
            let app = NSApplication::sharedApplication(marker);
            // SAFETY: The selector is a standard AppKit Edit action. Its target
            // must belong to the focused WebView, never the GPUI parent.
            let target = unsafe { app.targetForAction_to_from(action, None, None) };
            if let Some(target) = target.and_then(|target| target.downcast::<NSView>().ok()) {
                if target.isDescendantOf(native)
                    && unsafe { app.sendAction_to_from(action, Some(&target), None) }
                {
                    return true;
                }
            }
        }
    }
    responder.keyDown(event);
    true
}

#[cfg(target_os = "macos")]
fn relinquish_keyboard_focus(view: &WebView) {
    if webview_has_keyboard_focus(view) {
        let _ = view.focus_parent();
    }
}

/// One configured chat site, mirroring the Vue `ChatHub` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub logo: String,
}

impl Site {
    pub(crate) fn from_json(value: &serde_json::Value) -> Option<Self> {
        let url = value["url"].as_str()?.to_owned();
        if !web_url(&url) {
            return None;
        }
        Some(Self {
            id: value["id"].as_i64().unwrap_or_default(),
            name: value["name"].as_str().unwrap_or_default().to_owned(),
            url,
            logo: value["logo"].as_str().unwrap_or_default().to_owned(),
        })
    }
}

pub(crate) fn sites_from_json(sites: &[serde_json::Value]) -> Vec<Site> {
    // The runtime returns the settings center's persisted order.
    sites.iter().filter_map(Site::from_json).collect()
}

/// Proxy a ChatHub page has to reach the network through.
///
/// Mirrors the desktop's webview rule: only an explicit HTTP proxy applies, and a
/// proxy that asks for credentials cannot be passed to a webview.
#[derive(Clone, Debug)]
pub(crate) struct ProxySettings {
    host: String,
    port: u16,
    socks: bool,
}

/// Image requests can use authenticated proxies, unlike the native child view.
pub(crate) fn logo_proxy_from_config(config: &serde_json::Value) -> Option<String> {
    if config["proxy_type"].as_str()? != "http" {
        return None;
    }
    let server = config["proxy_server"].as_str()?.trim();
    if server.is_empty() {
        return None;
    }
    let mut url = if server.contains("://") {
        reqwest::Url::parse(server).ok()?
    } else {
        reqwest::Url::parse(&format!("http://{server}")).ok()?
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    if let Some(username) = config["proxy_username"]
        .as_str()
        .filter(|user| !user.is_empty())
    {
        url.set_username(username).ok()?;
        url.set_password(config["proxy_password"].as_str()).ok()?;
    }
    Some(url.into())
}

/// Reads the network settings the runtime published for its webviews.
pub(crate) fn proxy_from_config(config: &serde_json::Value) -> Option<ProxySettings> {
    let proxy_type = config["proxy_type"].as_str().unwrap_or("none");
    if proxy_type != "http" {
        return None;
    }
    let server = config["proxy_server"].as_str().unwrap_or("").trim();
    let username = config["proxy_username"].as_str().unwrap_or("");
    if server.is_empty() || !username.trim().is_empty() {
        return None;
    }
    let lower = server.to_ascii_lowercase();
    let (socks, rest) = if let Some(rest) = lower
        .strip_prefix("socks5://")
        .or_else(|| lower.strip_prefix("socks://"))
    {
        (true, rest)
    } else if let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    {
        (false, rest)
    } else if lower.contains("://") {
        return None;
    } else {
        (false, lower.as_str())
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    let default_port = if socks { 1080 } else { 80 };
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        // IPv6 literal: `[::1]` or `[::1]:8080`.
        let (host, tail) = stripped.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(default_port);
        (host.to_owned(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        match port.parse::<u16>() {
            Ok(port) => (host.to_owned(), port),
            Err(_) => (authority.to_owned(), default_port),
        }
    } else {
        (authority.to_owned(), default_port)
    };
    if host.is_empty() {
        return None;
    }
    Some(ProxySettings { host, port, socks })
}

#[cfg(target_os = "linux")]
type PageView = X11Page;
#[cfg(not(target_os = "linux"))]
type PageView = Rc<WebView>;

struct Page {
    site: Site,
    view: Option<PageView>,
    bounds: Rc<Cell<Option<Rect>>>,
    #[cfg(target_os = "linux")]
    applied_bounds: Option<Rect>,
}

/// One slide of the docked column, sampled from the monotonic clock.
///
/// `with_animation` is avoided for the reason the loading spinner avoids it:
/// the animation element renders at phase zero under the desktop's
/// reduced-motion preference, and gpui-fast only repaints an animated layer
/// while a frame is requested during the frame that changes it.
struct Reveal {
    started: Instant,
    from: f32,
    to: f32,
    /// A closing slide hides the native page once the column reaches zero.
    closing: bool,
}

impl Reveal {
    fn progress(&self) -> f32 {
        let elapsed = self.started.elapsed().as_secs_f32() / SLIDE.as_secs_f32();
        let t = elapsed.clamp(0., 1.);
        // Ease-out cubic: quick departure, settled arrival.
        1. - (1. - t).powi(3)
    }

    fn width(&self) -> f32 {
        self.from + (self.to - self.from) * self.progress()
    }

    fn done(&self, resize_pending: bool) -> bool {
        let elapsed = self.started.elapsed();
        elapsed >= SLIDE && (!resize_pending || elapsed >= SLIDE + RESIZE_SETTLE)
    }
}

struct WindowGrowth {
    base_window: f32,
    base_viewport: f32,
    requested: f32,
}

impl WindowGrowth {
    fn layout_extent(&self, viewport: f32, total_extent: f32, share: f32) -> f32 {
        // The resize request and the frame's viewport can be a frame apart.
        // Only borrow from the workflow when the final dock cannot fit outside it.
        let grown = self.observed_growth(viewport);
        let borrowed = (total_extent - self.requested).max(0.) * share;
        // A narrower dock after a manual drag must not jump back to the
        // original width when the close animation starts.
        grown * (total_extent / self.requested).min(1.) + borrowed
    }

    fn observed_growth(&self, viewport: f32) -> f32 {
        (viewport - self.base_viewport).clamp(0., self.requested)
    }

    fn resize_pending(&self, window: f32, viewport: f32, closing: bool) -> bool {
        let delta = if closing { 0. } else { self.requested };
        (window - self.base_window - delta).abs() >= 1.
            || (viewport - self.base_viewport - delta).abs() >= 1.
    }
}

/// The docked column: one native page per chat site, at most one visible.
pub(crate) struct Dock {
    pages: Vec<Page>,
    active: Option<i64>,
    visible: bool,
    /// User-chosen width of the docked column; the splitter writes it.
    width: f32,
    /// Running open or close slide of the column, `None` when it is at rest.
    reveal: Option<Reveal>,
    growth: Option<WindowGrowth>,
    /// The page paints this before the site has loaded, instead of black.
    background: wry::RGBA,
    proxy: Option<ProxySettings>,
    #[cfg(not(target_os = "macos"))]
    context: wry::WebContext,
    _profile_lock: File,
}

fn profile_root() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::home_dir().map(|home| home.join("Library/Application Support"));
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")));
    let base = base.filter(|path| path.is_absolute()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "ChatHub profile directory unavailable",
        )
    })?;
    Ok(chatspeed_runtime_client::production_app_data_dir(&base).join("gpui-chat-hub-v1"))
}

fn lock_profile(root: &Path) -> io::Result<File> {
    let mut dir = std::fs::DirBuilder::new();
    dir.recursive(true);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        dir.mode(0o700);
        options.mode(0o600);
    }
    dir.create(root)?;
    let lock = options.open(root.join("profile.lock"))?;
    lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "ChatHub profile is already in use",
        ),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(lock)
}

#[cfg(target_os = "macos")]
fn persistent_store_supported() -> bool {
    let Ok(version) = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
    else {
        return false;
    };
    version.status.success()
        && String::from_utf8_lossy(&version.stdout)
            .split('.')
            .next()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(0)
            >= 14
}

impl Dock {
    pub(crate) fn new(
        cx: &mut Context<crate::Observer>,
        background: wry::RGBA,
        proxy: Option<ProxySettings>,
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        if !persistent_store_supported() {
            return Err("Persistent ChatHub requires macOS 14 or later".into());
        }
        // Only the GTK dock drives the loop; other platforms ignore the context.
        #[cfg(not(target_os = "linux"))]
        let _ = cx;
        let root = profile_root().map_err(|error| error.to_string())?;
        let profile_lock = lock_profile(&root).map_err(|error| error.to_string())?;
        #[cfg(target_os = "linux")]
        {
            gtk::gdk::set_allowed_backends("x11");
            gtk::init().map_err(|error| error.to_string())?;
        }
        #[cfg(not(target_os = "macos"))]
        let context = wry::WebContext::new(Some(root.join("web-data")));
        let dock = Self {
            pages: Vec::new(),
            active: None,
            visible: false,
            width: DOCK_WIDTH,
            reveal: None,
            growth: None,
            background,
            proxy,
            #[cfg(not(target_os = "macos"))]
            context,
            _profile_lock: profile_lock,
        };
        // The GTK event loop drives every child view, so it runs once for the dock.
        #[cfg(target_os = "linux")]
        dock.pump_gtk(cx);
        Ok(dock)
    }

    pub(crate) fn sites(&self) -> impl Iterator<Item = &Site> {
        self.pages.iter().map(|page| &page.site)
    }

    pub(crate) fn active(&self) -> Option<&Site> {
        let id = self.active?;
        self.pages
            .iter()
            .find(|page| page.site.id == id)
            .map(|page| &page.site)
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible && self.active.is_some()
    }

    /// Target page width, excluding the splitter.
    pub(crate) fn width(&self) -> f32 {
        self.width
    }

    /// Clock-driven width used to request native growth, not to lay out the page.
    fn column_width(&self) -> f32 {
        match (&self.reveal, self.visible) {
            (Some(reveal), _) => reveal.width(),
            (None, true) => self.width,
            (None, false) => 0.,
        }
    }

    /// Whether the column still needs frames to finish its slide.
    pub(crate) fn sliding(&self) -> bool {
        self.reveal.is_some()
    }

    /// Total space to the right of the workflow, including the splitter.
    pub(crate) fn layout_extent(&self, viewport: f32) -> f32 {
        if !self.visible() {
            return 0.;
        }
        let share = self.slide_share();
        let total = self.width + SPLITTER_WIDTH;
        if self.sliding() {
            self.growth.as_ref().map_or(total * share, |growth| {
                growth.layout_extent(viewport, total, share)
            })
        } else {
            total
        }
    }

    fn slide_share(&self) -> f32 {
        (self.column_width() / self.width.max(1.)).clamp(0., 1.)
    }

    /// Request the next native width; layout waits for the actual viewport.
    pub(crate) fn slide_window_width(&self, window_width: f32) -> Option<f32> {
        let growth = self.growth.as_ref()?;
        self.reveal.as_ref()?;
        let target = growth.base_window + growth.requested * self.slide_share();
        ((target - window_width).abs() >= 1.).then_some(target)
    }

    pub(crate) fn set_width(&mut self, width: f32) {
        self.width = width;
        // Expansion may fit a smaller page than the stored preference.
        if let Some(reveal) = self.reveal.as_mut().filter(|reveal| !reveal.closing) {
            reveal.to = width;
        }
    }

    /// Starts a slide from wherever the column is now, so an interrupted
    /// animation continues from its current width.
    fn slide_to(&mut self, to: f32, closing: bool) {
        self.reveal = Some(Reveal {
            started: Instant::now(),
            from: self.column_width(),
            to,
            closing,
        });
    }

    /// Keep the slide layout until the last native resize reaches the viewport.
    pub(crate) fn advance_reveal(&mut self, window: f32, viewport: f32, can_resize: bool) {
        let Some(reveal) = &self.reveal else {
            return;
        };
        let resize_pending = can_resize
            && self.growth.as_ref().is_some_and(|growth| {
                growth.resize_pending(window, viewport, reveal.closing)
            });
        if !reveal.done(resize_pending) {
            return;
        }
        let closing = reveal.closing;
        if can_resize && !closing && self.growth.is_some() {
            // Preserve the last visible extent even if the native size was
            // rounded or the compositor capped the available growth.
            self.width = (self.layout_extent(viewport) - SPLITTER_WIDTH).max(DOCK_MIN_WIDTH);
        }
        self.reveal = None;
        if closing {
            self.visible = false;
            self.growth = None;
            self.sync_visibility();
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Opens (or re-activates) one site and makes the dock visible.
    pub(crate) fn open(
        &mut self,
        window: &mut Window,
        cx: &gpui_kit::App,
        site: Site,
    ) -> Result<(), String> {
        if !self.pages.iter().any(|page| page.site.id == site.id) {
            if !web_url(&site.url) {
                return Err("Expected an HTTP(S) address".into());
            }
            #[cfg(not(target_os = "linux"))]
            let view = Some(self.build(
                window,
                cx,
                &site.url,
                Rect {
                    position: wry::dpi::LogicalPosition::new(-1024, -1024).into(),
                    size: wry::dpi::LogicalSize::new(1, 1).into(),
                },
            )?);
            #[cfg(target_os = "macos")]
            if let Some(view) = &view {
                remember_edit_view(view);
            }
            self.pages.push(Page {
                site: site.clone(),
                #[cfg(not(target_os = "linux"))]
                view,
                #[cfg(target_os = "linux")]
                view: None,
                bounds: Rc::new(Cell::new(None)),
                #[cfg(target_os = "linux")]
                applied_bounds: None,
            });
        }
        self.active = Some(site.id);
        // A page opened while the column is still closing slides back out from
        // wherever the close got to.
        let opening = !self.visible
            || self
                .reveal
                .as_ref()
                .is_some_and(|reveal| reveal.closing);
        // The slide starts before the dock claims its column, so a hidden dock
        // slides out from zero instead of from the width it is heading to.
        if opening {
            self.slide_to(self.width, false);
        }
        self.visible = true;
        if self.growth.is_none() {
            self.expand(window, cx);
        }
        self.sync_visibility();
        Ok(())
    }

    pub(crate) fn activate(&mut self, id: i64) {
        if self.pages.iter().any(|page| page.site.id == id) {
            self.active = Some(id);
        }
        self.sync_visibility();
    }

    /// Flips the dock and reports whether the column is opening.
    ///
    /// A closing slide already reads as hidden, so a second press reopens it
    /// instead of hiding it twice.
    pub(crate) fn toggle_visible(&mut self) -> bool {
        if self.active.is_none() {
            return false;
        }
        let hidden = self
            .reveal
            .as_ref()
            .map_or(!self.visible, |reveal| reveal.closing);
        if hidden {
            self.slide_to(self.width, false);
            self.visible = true;
            self.sync_visibility();
            true
        } else {
            self.slide_to(0., true);
            false
        }
    }

    #[cfg(target_os = "macos")]
    fn active_view(&self) -> Option<&Rc<WebView>> {
        self.pages
            .iter()
            .find(|page| Some(page.site.id) == self.active)
            .and_then(|page| page.view.as_ref())
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn focused_view(&self) -> Option<Rc<WebView>> {
        if !self.visible() {
            return None;
        }
        self.active_view()
            .filter(|view| webview_has_keyboard_focus(view))
            .cloned()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn return_keyboard_focus(&self) {
        if let Some(view) = self.active_view() {
            relinquish_keyboard_focus(view);
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn return_keyboard_focus(&self) {
        if let Some(view) = self.pages.iter().find(|page| Some(page.site.id) == self.active)
            .and_then(|page| page.view.as_ref()) {
            view.host.return_focus();
        }
    }

    pub(crate) fn close_tab(&mut self, id: i64) {
        #[cfg(target_os = "macos")]
        if self.active == Some(id) {
            self.return_keyboard_focus();
        }
        self.pages.retain(|page| page.site.id != id);
        if self.active == Some(id) {
            self.active = self.pages.last().map(|page| page.site.id);
        }
        if self.pages.is_empty() {
            // The last page is gone, so there is nothing left to slide out.
            self.reveal = None;
            self.visible = false;
        }
        self.sync_visibility();
    }

    fn sync_visibility(&mut self) {
        for page in &mut self.pages {
            let shown = self.visible && self.active == Some(page.site.id);
            if !shown {
                // A remapped native view needs the current dock geometry, even
                // when GPUI measures the same rectangle as before it was hidden.
                #[cfg(not(target_os = "linux"))]
                let was_shown = page.bounds.take().is_some();
                #[cfg(target_os = "linux")]
                let was_shown = {
                    page.bounds.set(None);
                    page.applied_bounds.take().is_some()
                };
                if let (true, Some(view)) = (was_shown, &page.view) {
                    #[cfg(target_os = "macos")]
                    relinquish_keyboard_focus(view);
                    #[cfg(not(target_os = "linux"))]
                    let _ = view.set_visible(false);
                    #[cfg(target_os = "linux")]
                    view.host.hide();
                }
            }
        }
    }

    fn build(
        &mut self,
        window: &mut Window,
        cx: &gpui_kit::App,
        url: &str,
        rect: Rect,
    ) -> Result<PageView, String> {
        let opener = cx.to_async();
        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            WebViewBuilder::new().with_data_store_identifier(*b"CSGPUIChatHubV01")
        };
        #[cfg(not(target_os = "macos"))]
        let builder = WebViewBuilder::new_with_web_context(&mut self.context);
        let builder = builder
            .with_visible(cfg!(target_os = "linux"))
            .with_bounds(rect)
            .with_url(url)
            .with_background_color(self.background)
            .with_navigation_handler(|url| web_url(&url))
            .with_initialization_script(OPEN_LINKS_IN_BROWSER)
            .with_new_window_req_handler(move |url, _| {
                forward_browser_link(url, |url| {
                    // Defer app access until the native WebView callback has returned.
                    opener.spawn(async move |cx| cx.update(|cx| cx.open_url(&url))).detach();
                })
            });
        let builder = match self.proxy.clone() {
            Some(proxy) => {
                let endpoint = wry::ProxyEndpoint {
                    host: proxy.host,
                    port: proxy.port.to_string(),
                };
                builder.with_proxy_config(if proxy.socks {
                    wry::ProxyConfig::Socks5(endpoint)
                } else {
                    wry::ProxyConfig::Http(endpoint)
                })
            }
            None => builder,
        };
        #[cfg(target_os = "linux")]
        {
            use wry::WebViewBuilderExtUnix;
            let host = X11Host::new(x11_parent_id(window)?, rect)?;
            let view = builder
                .with_focused(false)
                .build_gtk(&host.container)
                .map_err(|error| error.to_string())?;
            host.install_focus(&view);
            host.layout(rect);
            Ok(X11Page { view: Some(view), host })
        }
        #[cfg(not(target_os = "linux"))]
        {
            builder
                .build_as_child(window)
                .map(Rc::new)
                .map_err(|error| error.to_string())
        }
    }

    #[cfg(target_os = "linux")]
    fn pump_gtk(&self, cx: &mut Context<crate::Observer>) {
        let window = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            while window.upgrade().is_some() {
                for _ in 0..32 {
                    if !gtk::events_pending() {
                        break;
                    }
                    gtk::main_iteration_do(false);
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
    }

    pub(crate) fn expand(&mut self, window: &mut Window, cx: &gpui_kit::App) {
        use gpui_kit::{px, size};
        if self.growth.is_some() {
            // A reversed closing slide still belongs to the original window.
            return;
        }
        if window.is_maximized() || window.is_fullscreen() {
            // There is no room to grow, so the page must already fit the
            // window it has when the slide starts.
            self.set_width(clamp_width(
                f32::from(window.viewport_size().width),
                self.width,
            ));
            return;
        }
        let Some(display) = window.display(cx) else {
            self.set_width(clamp_width(
                f32::from(window.viewport_size().width),
                self.width,
            ));
            return;
        };
        let bounds = window.bounds();
        let area = display.visible_bounds();
        let available = f32::from(
            (area.origin.x + area.size.width - bounds.origin.x - bounds.size.width).max(px(0.)),
        );
        let viewport = f32::from(window.viewport_size().width);
        let (width, growth) = expand_plan(viewport, available, self.width);
        self.set_width(width);
        if growth > 0. {
            self.growth = Some(WindowGrowth {
                base_window: f32::from(bounds.size.width),
                base_viewport: viewport,
                requested: growth,
            });
            if self.reveal.is_none() {
                window.resize(size(bounds.size.width + px(growth), bounds.size.height));
            }
        }
    }

    pub(crate) fn added_width(&self, width: gpui_kit::Pixels) -> gpui_kit::Pixels {
        gpui_kit::px(self.growth.as_ref().map_or(0., |growth| {
            (f32::from(width) - growth.base_window).clamp(0., growth.requested)
        }))
    }

    /// Hands back the width the dock took from the screen.
    pub(crate) fn release(&mut self, window: &mut Window) {
        // A closing slide gives the width back frame by frame and forgets the
        // growth when it finishes.
        if self.reveal.is_some() {
            return;
        }
        let grown = self.added_width(window.bounds().size.width);
        self.growth = None;
        if window.is_maximized() || window.is_fullscreen() {
            return;
        }
        let bounds = window.bounds();
        if grown > gpui_kit::px(0.) {
            window.resize(gpui_kit::size(
                bounds.size.width - grown,
                bounds.size.height,
            ));
        }
    }

    pub(crate) fn needs_release(&self) -> bool {
        self.is_empty() && self.growth.is_some() && self.reveal.is_none()
    }

    #[cfg(target_os = "linux")]
    fn apply_layout(
        &mut self,
        window: &mut Window,
        cx: &gpui_kit::App,
        id: i64,
        bounds: &Rc<Cell<Option<Rect>>>,
        rect: Option<Rect>,
    ) -> Result<(), String> {
        if !self.visible || self.active != Some(id) || bounds.get() != rect {
            return Ok(());
        }
        let Some(index) = self
            .pages
            .iter()
            .position(|page| page.site.id == id && Rc::ptr_eq(&page.bounds, bounds))
        else {
            return Ok(());
        };
        let applied = self.pages[index].applied_bounds;
        if applied == rect {
            return Ok(());
        }
        match (rect, &self.pages[index].view) {
            (Some(rect), None) => {
                let url = self.pages[index].site.url.clone();
                match self.build(window, cx, &url, rect) {
                    Ok(view) => self.pages[index].view = Some(view),
                    Err(error) => {
                        self.close_tab(id);
                        return Err(error);
                    }
                }
            }
            (Some(rect), Some(view)) => view.host.layout(rect),
            (None, Some(view)) => view.host.hide(),
            (None, None) => {}
        }
        self.pages[index].applied_bounds = rect;
        Ok(())
    }

    /// The active page's canvas; the other pages stay hidden until selected.
    #[cfg(target_os = "linux")]
    pub(crate) fn dock(&self, cx: &Context<crate::Observer>) -> AnyElement {
        let sliding = self.reveal.is_some();
        let Some(page) = self
            .pages
            .iter()
            .find(|page| Some(page.site.id) == self.active)
        else {
            return div().flex_1().min_h_0().w_full().into_any_element();
        };
        let id = page.site.id;
        let bounds_cell = Rc::downgrade(&page.bounds);
        let owner = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                if sliding {
                    // gpui-fast keeps an animated layer repainting only while a
                    // frame is requested during paint, like the loading spinner.
                    window.request_animation_frame();
                }
                let Some(bounds_cell) = bounds_cell.upgrade() else {
                    return;
                };
                let rect = x11_dock_rect(bounds, window.viewport_size(), window.scale_factor());
                if bounds_cell.replace(rect) == rect {
                    return;
                }
                // Creating a GTK child also maps it. Defer until after layout so
                // its first visible rectangle is the dock, not the window origin.
                window.defer(cx, move |window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if let Some(dock) = &mut this.chat_hub {
                            if let Err(error) = dock.apply_layout(window, cx, id, &bounds_cell, rect) {
                                this.show_toast(crate::ToastKind::Error, error, cx);
                            }
                        }
                    });
                });
            },
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .into_any_element()
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn dock(&self, _: &Context<crate::Observer>) -> AnyElement {
        let sliding = self.reveal.is_some();
        let active = self.active.and_then(|id| {
            let page = self.pages.iter().find(|page| page.site.id == id)?;
            Some((Rc::downgrade(page.view.as_ref()?), page.bounds.clone()))
        });
        let Some((view, last)) = active else {
            return div().flex_1().min_h_0().w_full().into_any_element();
        };
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                if sliding {
                    // gpui-fast keeps an animated layer repainting only while a
                    // frame is requested during paint, like the loading spinner.
                    window.request_animation_frame();
                }
                let Some(view) = view.upgrade() else {
                    return;
                };
                let w = f32::from(bounds.size.width);
                let h = f32::from(bounds.size.height);
                if w <= 0. || h <= 0. {
                    #[cfg(target_os = "macos")]
                    relinquish_keyboard_focus(&view);
                    let _ = view.set_visible(false);
                    last.set(None);
                    return;
                }
                use wry::dpi::{LogicalPosition, LogicalSize};
                let rect = Rect {
                    position: LogicalPosition::new(
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                    )
                    .into(),
                    size: LogicalSize::new(w, h).into(),
                };
                if last.get() != Some(rect)
                    && view.set_bounds(rect).is_ok()
                    && view.set_visible(true).is_ok()
                {
                    last.set(Some(rect));
                }
            },
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .into_any_element()
    }
}

#[cfg(target_os = "linux")]
fn x11_dock_rect(
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    viewport: gpui_kit::Size<gpui_kit::Pixels>,
    scale: f32,
) -> Option<Rect> {
    use gpui_kit::px;
    use wry::dpi::{PhysicalPosition, PhysicalSize};
    let to_pixel = |value| (f32::from(value) * scale).round() as i32;
    let left = to_pixel(bounds.origin.x.max(px(0.)));
    let top = to_pixel(bounds.origin.y.max(px(0.)));
    let right = to_pixel((bounds.origin.x + bounds.size.width).min(viewport.width));
    let bottom = to_pixel((bounds.origin.y + bounds.size.height).min(viewport.height));
    (right > left && bottom > top).then(|| Rect {
        position: PhysicalPosition::new(left, top).into(),
        size: PhysicalSize::new((right - left) as u32, (bottom - top) as u32).into(),
    })
}

#[cfg(target_os = "linux")]
fn x11_parent_id(window: &Window) -> Result<u32, String> {
    use raw_window_handle::RawWindowHandle;
    let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::Xcb(handle) => Ok(handle.window.get()),
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).map_err(|error| error.to_string()),
        _ => Err("ChatHub on Linux requires X11 or XWayland".into()),
    }
}

#[cfg(target_os = "linux")]
struct X11Page {
    view: Option<WebView>,
    host: X11Host,
}

#[cfg(target_os = "linux")]
impl Drop for X11Page {
    fn drop(&mut self) {
        use gtk::prelude::*;

        self.host.return_focus();
        // The main X11 window may already be gone, which destroys its native
        // children before GTK can dispose their widgets.
        gtk::gdk::error_trap_push();
        drop(self.view.take());
        // SAFETY: The WebView has been dropped and no code uses this widget
        // after disposal. Destroy synchronously while the error trap is active.
        unsafe { self.host.window.destroy() };
        gtk::gdk::error_trap_pop_ignored();
    }
}

#[cfg(target_os = "linux")]
struct X11Host {
    window: gtk::Window,
    container: gtk::Box,
    connection: Rc<x11rb::rust_connection::RustConnection>,
    parent: u32,
    child: u32,
}

#[cfg(target_os = "linux")]
impl X11Host {
    fn new(parent: u32, rect: Rect) -> Result<Self, String> {
        use gtk::{glib::translate::ToGlibPtr, prelude::*};
        use x11rb::protocol::xproto::ConnectionExt;

        #[link(name = "gdk-3")]
        extern "C" {
            fn gdk_x11_window_get_xid(window: *mut gtk::gdk::ffi::GdkWindow) -> std::ffi::c_ulong;
        }

        // GTK must retain the GdkWindow it creates, along with its event and
        // IME state. Wry's X11 child adapter replaces that window with a
        // foreign one, then destroys it before GTK finishes unmapping it.
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_decorated(false);
        window.set_focus_on_map(false);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&container);
        window.realize();
        let native = window.window().ok_or_else(|| wry::Error::UnsupportedWindowHandle.to_string())?;
        native.set_override_redirect(true);
        let display = native.display();
        // Realize the native XID on GDK's connection before using it on ours.
        display.sync();
        // SAFETY: ChatHub initializes GTK with only its X11 backend enabled.
        let child = u32::try_from(unsafe { gdk_x11_window_get_xid(native.to_glib_none().0) })
            .map_err(|error| error.to_string())?;
        let (connection, _) = x11rb::connect(Some(display.name().as_str())).map_err(|error| error.to_string())?;
        let position = rect.position.to_physical::<i16>(f64::from(window.scale_factor()));
        connection.reparent_window(child, parent, position.x, position.y).map_err(|error| error.to_string())?
            .check().map_err(|error| error.to_string())?;
        Ok(Self { window, container, connection: Rc::new(connection), parent, child })
    }

    fn install_focus(&self, view: &WebView) {
        use gtk::prelude::*;
        use wry::WebViewExtUnix;
        use x11rb::protocol::xproto::{ConnectionExt, InputFocus};

        let connection = self.connection.clone();
        let child = self.child;
        let host = self.window.downgrade();
        view.webview().connect_button_press_event(move |widget, event| {
            if let Some(host) = host.upgrade().filter(|host| host.is_mapped()) {
                if !host.has_toplevel_focus() {
                    // An embedded override-redirect window is not activated by the WM.
                    if let Ok(request) = connection.set_input_focus(InputFocus::PARENT, child, event.time()) {
                        let _ = request.check();
                    }
                }
                widget.grab_focus();
            }
            // WebKit still receives the original click to place its caret.
            gtk::glib::Propagation::Proceed
        });
    }

    fn return_focus(&self) {
        use gtk::prelude::*;
        use x11rb::protocol::xproto::{ConnectionExt, InputFocus};

        if !self.window.is_mapped() {
            return;
        }
        let focus = self.connection.get_input_focus().ok().and_then(|request| request.reply().ok());
        if focus.is_some_and(|focus| focus.focus == self.child) {
            if let Ok(request) = self.connection.set_input_focus(InputFocus::PARENT, self.parent, x11rb::CURRENT_TIME) {
                let _ = request.check();
            }
        }
    }

    fn hide(&self) {
        use gtk::prelude::*;
        self.return_focus();
        self.window.hide();
    }

    fn layout(&self, rect: Rect) {
        use gtk::prelude::*;
        let scale = f64::from(self.window.scale_factor());
        let position = rect.position.to_logical::<i32>(scale);
        let size = rect.size.to_logical::<i32>(scale);
        let width = size.width.max(1);
        let height = size.height.max(1);
        self.window.set_default_size(width, height);
        self.window.resize(width, height);
        self.window.size_allocate(&gtk::Allocation::new(0, 0, width, height));
        if let Some(native) = self.window.window() {
            native.move_resize(position.x, position.y, width, height);
        }
        if !self.window.is_visible() {
            self.window.show_all();
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[::core::prelude::v1::test]
#[ignore = "requires an isolated X11 display"]
fn x11_webview_focus_returns_to_parent_and_tears_down() {
    use gtk::prelude::*;
    use wry::{dpi::{PhysicalPosition, PhysicalSize}, WebViewBuilderExtUnix};
    use x11rb::{connection::Connection, protocol::xproto::{ConnectionExt, CreateWindowAux, InputFocus, WindowClass}};

    gtk::gdk::set_allowed_backends("x11");
    gtk::init().unwrap();
    let (connection, screen) = x11rb::connect(None).unwrap();
    let root = connection.setup().roots[screen].root;
    let parent = connection.generate_id().unwrap();
    connection.create_window(0, parent, root, 0, 0, 640, 480, 0,
        WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new()).unwrap().check().unwrap();
    connection.map_window(parent).unwrap().check().unwrap();
    let rect = Rect {
        position: PhysicalPosition::new(220, 80).into(),
        size: PhysicalSize::new(400, 300).into(),
    };
    let host = X11Host::new(parent, rect).unwrap();
    let view = WebViewBuilder::new().with_incognito(true).with_focused(false)
        .with_html("<!doctype html><input aria-label='ChatHub focus probe'>")
        .build_gtk(&host.container).unwrap();
    host.install_focus(&view);
    host.layout(rect);
    host.window.window().unwrap().display().sync();
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
    let child = host.child;
    assert_eq!(connection.query_tree(child).unwrap().reply().unwrap().parent, parent);
    connection.set_input_focus(InputFocus::PARENT, child, x11rb::CURRENT_TIME).unwrap().check().unwrap();
    host.return_focus();
    assert_eq!(connection.get_input_focus().unwrap().reply().unwrap().focus, parent);
    host.hide();
    host.layout(rect);
    let page = X11Page { view: Some(view), host };
    connection.destroy_window(parent).unwrap().check().unwrap();
    drop(page);
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[::core::prelude::v1::test]
fn x11_dock_rect_keeps_the_panel_offset_and_rounds_edges() {
    use gpui_kit::{point, px, size, Bounds};
    use wry::dpi::{PhysicalPosition, PhysicalSize};

    let bounds = Bounds {
        origin: point(px(620.4), px(66.4)),
        size: size(px(506.4), px(610.4)),
    };
    assert_eq!(
        x11_dock_rect(bounds, size(px(1126.), px(677.)), 1.25),
        Some(Rect {
            position: PhysicalPosition::new(776, 83).into(),
            size: PhysicalSize::new(632, 763).into(),
        }),
    );
}

#[cfg(all(test, target_os = "linux"))]
#[::core::prelude::v1::test]
fn x11_dock_rect_rejects_empty_and_offscreen_layouts() {
    use gpui_kit::{point, px, size, Bounds};

    let viewport = size(px(1126.), px(677.));
    for (x, width) in [(620., 0.), (1200., 100.), (-1024., 1.)] {
        assert_eq!(
            x11_dock_rect(
                Bounds {
                    origin: point(px(x), px(68.)),
                    size: size(px(width), px(600.)),
                },
                viewport,
                1.,
            ),
            None,
        );
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn logo_proxy_preserves_http_scheme_and_credentials() {
    let config = serde_json::json!({
        "proxy_type": "http",
        "proxy_server": "https://proxy.example:8443",
        "proxy_username": "user@example",
        "proxy_password": "test password"
    });
    let proxy = logo_proxy_from_config(&config).expect("configured logo proxy");
    assert_eq!(
        proxy,
        "https://user%40example:test%20password@proxy.example:8443/"
    );
    assert!(proxy_from_config(&config).is_none());
    assert!(logo_proxy_from_config(&serde_json::json!({"proxy_type": "none"})).is_none());
}

fn web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn forward_browser_link(url: String, open: impl FnOnce(String)) -> NewWindowResponse {
    if web_url(&url) {
        if let Ok(url) = reqwest::Url::parse(&url) {
            open(url.to_string());
        }
    }
    // Even when forwarding succeeds, the embedded view should not spawn a window.
    NewWindowResponse::Deny
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_forwards_web_popups_without_creating_embedded_windows() {
    let mut opened = Vec::new();
    for url in [
        "https://example.com/docs?q=test#start",
        "http://example.org/",
        "https://",
        "file:///tmp/test",
        "javascript:alert(1)",
        "about:blank",
    ] {
        assert!(matches!(
            forward_browser_link(url.into(), |url| opened.push(url)),
            NewWindowResponse::Deny,
        ));
    }
    assert_eq!(opened, ["https://example.com/docs?q=test#start", "http://example.org/"]);
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_profile_lock_releases_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let lock = lock_profile(dir.path()).unwrap();
    assert_eq!(
        lock_profile(dir.path()).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(lock);
    assert!(lock_profile(dir.path()).is_ok());
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_rejects_non_web_navigation() {
    assert!(web_url("https://example.com/"));
    assert!(web_url("http://localhost:8080/"));
    for url in [
        "file:///tmp/test",
        "javascript:alert(1)",
        "data:text/html,test",
        "about:blank",
    ] {
        assert!(!web_url(url));
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_site_list_preserves_settings_order_and_filters_invalid_urls() {
    let payload = serde_json::json!([
        {"id": 3, "name": "C", "url": "https://c.example/", "logo": "https://c.example/i.png"},
        {"id": 1, "name": "A", "url": "file:///tmp/a", "logo": ""},
        {"id": 2, "name": "B", "url": "https://b.example/", "logo": ""}
    ]);
    let sites = sites_from_json(payload.as_array().expect("site list"));
    assert_eq!(sites.iter().map(|site| site.id).collect::<Vec<_>>(), [3, 2]);
    assert_eq!(sites[0].logo, "https://c.example/i.png");
    assert!(sites_from_json(&[]).is_empty());
}
