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
/// How long the docked column takes to slide in or out.
const SLIDE: Duration = Duration::from_millis(180);

/// Clamps a requested page width the way the Tauri carrier does.
pub(crate) fn clamp_width(window_width: f32, requested: f32) -> f32 {
    if !window_width.is_finite() || !requested.is_finite() {
        return DOCK_WIDTH;
    }
    let maximum = (window_width - DOCK_MIN_HOST_WIDTH).max(DOCK_MIN_WIDTH);
    requested.clamp(DOCK_MIN_WIDTH, maximum)
}

/// Resolves the page width and the window growth together.
///
/// The width is clamped against the window the dock is about to get, not the
/// one it has now, so a slide can target the final width and the clamp after
/// the expansion has nothing left to change.
fn expand_plan(window_width: f32, available: f32, requested: f32) -> (f32, f32) {
    let available = available.max(0.);
    let width = clamp_width(window_width + available.min(requested), requested);
    (width, available.min(width))
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
        assert!(growth <= width);
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn dock_width_keeps_a_phone_page_and_the_workflow_ui_visible() {
    assert_eq!(clamp_width(1600., 900.), 900.);
    assert_eq!(clamp_width(1600., 2000.), 1120.);
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
    assert!(landed.done());
    assert_eq!(landed.width(), 600.);
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
    use gpui_kit::{component::input::Paste, Menu, MenuItem, OsAction};

    // Wry child views leave Command shortcuts to the menu bar. A native
    // paste: action gives WebKit the original pasteboard, including images.
    cx.set_menus([
        Menu::new("ChatSpeed"),
        Menu::new(rust_i18n::t!("menu_edit").to_string()).items([MenuItem::os_action(
            rust_i18n::t!("menu_paste").to_string(),
            Paste,
            OsAction::Paste,
        )]),
    ]);
    // When GPUI sees the key equivalent first, forward it before the editor's
    // Paste binding can consume the event while WebKit has native focus.
    cx.intercept_keystrokes(|event, window, cx| {
        let key = &event.keystroke;
        if key.key != "v" || !key.modifiers.platform || key.modifiers.control
            || key.modifiers.alt || key.modifiers.shift || key.modifiers.function
        {
            return;
        }
        let Some(observer) = window.root::<crate::Observer>().flatten() else {
            return;
        };
        let Some(view) = observer.read(cx).chat_hub.as_ref().and_then(Dock::focused_view) else {
            return;
        };
        // GPUI's logical focus can still point to the composer. Once WebKit
        // owns native focus, never fall through to that stale Paste binding.
        cx.stop_propagation();
        paste_into_webview(&view);
    }).detach();
    // The menu wins the key equivalent before GPUI sees it, so Cmd+V needs the
    // native monitor installed above to reach the page with the full pasteboard.
    install_paste_monitor();
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
fn paste_into_webview(view: &WebView) {
    use objc2_app_kit::{NSApplication, NSView};
    use wry::WebViewExtMacOS;

    let webview = view.webview();
    let native: &NSView = &webview;
    let app = NSApplication::sharedApplication(native.into());
    let Some(sender) = app.mainMenu().and_then(|menu| menu.itemAtIndex(1))
        .and_then(|item| item.submenu()).and_then(|menu| menu.itemAtIndex(0)) else {
        return;
    };
    let Some(action) = sender.action() else { return; };
    // SAFETY: The selector and sender are from the native Paste menu item.
    let target = unsafe { app.targetForAction_to_from(action, None, Some(&sender)) };
    let Some(target) = target.and_then(|target| target.downcast::<NSView>().ok()) else {
        return;
    };
    if target.isDescendantOf(native) {
        // SAFETY: AppKit resolved this target for the same selector and sender.
        unsafe { app.sendAction_to_from(action, Some(&target), Some(&sender)) };
    }
}

/// Intercepts Cmd+V before AppKit's menu handles it, while a docked page owns
/// the keyboard.
///
/// The menu key equivalent runs before the window sees the key, so Cmd+V went
/// through the app's GPUI Paste action: the pasteboard stayed inside GPUI focus
/// and the page never received it, which is why a screenshot only pasted from
/// the page's own context menu. A local event monitor runs before dispatch, so
/// the page can be given the native `paste:` selector with the original
/// pasteboard. Every other case (GPUI inputs, dialogs, other shortcuts) is left
/// to the menu unchanged.
#[cfg(target_os = "macos")]
pub(crate) fn install_paste_monitor() {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::cell::RefCell;
    use std::ptr::NonNull;

    thread_local! {
        /// AppKit calls the block until the monitor is removed; the token has to
        /// outlive the install call for the whole app run.
        static PASTE_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }

    PASTE_MONITOR.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes the key event this monitor was installed for.
            let event_ref = unsafe { event.as_ref() };
            if is_command_v(event_ref) && paste_into_focused_webview() {
                // The page consumed the paste; the menu must not also run it.
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
fn is_command_v(event: &objc2_app_kit::NSEvent) -> bool {
    use objc2_app_kit::NSEventModifierFlags;

    let flags = event.modifierFlags();
    if !flags.contains(NSEventModifierFlags::Command) {
        return false;
    }
    let blocking = NSEventModifierFlags::Control
        | NSEventModifierFlags::Option
        | NSEventModifierFlags::Shift;
    if flags.intersects(blocking) {
        return false;
    }
    event
        .charactersIgnoringModifiers()
        .is_some_and(|characters| characters.to_string().eq_ignore_ascii_case("v"))
}

/// Sends the native `paste:` selector to the focused web page.
///
/// This is the path the page's context menu uses: WebKit reads the original
/// pasteboard, so screenshots paste, while a GPUI Paste action can only feed
/// GPUI elements. Returns false when the key window's first responder is not
/// inside a web view or refuses the action, so the menu path still runs.
#[cfg(target_os = "macos")]
fn paste_into_focused_webview() -> bool {
    use objc2::runtime::AnyClass;
    use objc2_app_kit::{NSApplication, NSView};

    let Some(marker) = objc2::MainThreadMarker::new() else {
        return false;
    };
    let Some(webview) = AnyClass::get(c"WKWebView") else {
        return false;
    };
    let app = NSApplication::sharedApplication(marker);
    let Some(window) = app.keyWindow() else { return false; };
    let Some(responder) = window.firstResponder() else { return false; };
    let Ok(view) = responder.clone().downcast::<NSView>() else { return false; };
    if !belongs_to_webview(&view, webview) {
        return false;
    }
    // SAFETY: `paste:` is AppKit's own responder action and the responder is
    // the key window's current first responder.
    unsafe { app.sendAction_to_from(objc2::sel!(paste:), Some(&responder), None) }
}

#[cfg(target_os = "macos")]
fn belongs_to_webview(
    view: &objc2_app_kit::NSView,
    webview: &objc2::runtime::AnyClass,
) -> bool {
    use objc2::runtime::NSObjectProtocol;

    if view.isKindOfClass(webview) {
        return true;
    }
    // SAFETY: Walking the superview chain of the key window's first responder
    // is the documented way to find the view that hosts it.
    let mut parent = unsafe { view.superview() };
    while let Some(current) = parent {
        if current.isKindOfClass(webview) {
            return true;
        }
        parent = unsafe { current.superview() };
    }
    false
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

struct Page {
    site: Site,
    view: Option<Rc<WebView>>,
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

    fn done(&self) -> bool {
        self.started.elapsed() >= SLIDE
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
    growth: Option<(gpui_kit::Pixels, gpui_kit::Pixels)>,
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

    /// Width of the docked column, already clamped when it was last set.
    ///
    /// This is the width the slide is heading for; `column_width` is the width
    /// the frame actually gives it.
    pub(crate) fn width(&self) -> f32 {
        self.width
    }

    /// Width the column takes in this frame.
    pub(crate) fn column_width(&self) -> f32 {
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

    /// Window width the slide asks for, so the workflow column keeps its size
    /// while the page grows or shrinks. `None` once the window is already there.
    ///
    /// The share is taken from the column itself, not from the animation
    /// direction, so a slide interrupted halfway resumes where it stopped.
    pub(crate) fn slide_window_width(&self, window_width: f32) -> Option<f32> {
        let (base, requested) = self.growth?;
        if self.reveal.is_none() {
            return None;
        }
        let share = (self.column_width() / self.width.max(1.)).clamp(0., 1.);
        let target = f32::from(base) + f32::from(requested) * share;
        ((target - window_width).abs() >= 1.).then_some(target)
    }

    pub(crate) fn set_width(&mut self, width: f32) {
        self.width = width;
        // A splitter drag during the opening slide retargets it instead of
        // snapping once the slide lands on the old width.
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

    /// Samples the running slide. Returns true when it just finished, so the
    /// frame can be laid out and painted with the column already gone.
    pub(crate) fn advance_reveal(&mut self) -> bool {
        let Some(closing) = self
            .reveal
            .as_ref()
            .filter(|reveal| reveal.done())
            .map(|reveal| reveal.closing)
        else {
            return false;
        };
        self.reveal = None;
        if closing {
            self.visible = false;
            // The slide already handed the window width back frame by frame.
            self.growth = None;
            self.sync_visibility();
        }
        true
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
            let view = Some(Rc::new(self.build(
                window,
                cx,
                &site.url,
                Rect {
                    position: wry::dpi::LogicalPosition::new(-1024, -1024).into(),
                    size: wry::dpi::LogicalSize::new(1, 1).into(),
                },
            )?));
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
                    let _ = view.set_visible(false);
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
    ) -> Result<WebView, String> {
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
            use raw_window_handle::{RawWindowHandle, WindowHandle, XlibWindowHandle};
            let handle =
                HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
            let raw = match handle.as_raw() {
                RawWindowHandle::Xlib(xlib) => xlib,
                RawWindowHandle::Xcb(xcb) => XlibWindowHandle::new(xcb.window.get().into()),
                _ => return Err("ChatHub on Linux requires X11 or XWayland".into()),
            };
            struct Parent<'a>(WindowHandle<'a>);
            impl HasWindowHandle for Parent<'_> {
                fn window_handle(
                    &self,
                ) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
                    Ok(self.0)
                }
            }
            // The original borrowed handle keeps the same X11 window alive during creation.
            let parent = Parent(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(raw)) });
            builder
                .build_as_child(&parent)
                .map_err(|error| error.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            builder
                .build_as_child(window)
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
        let (width, growth) =
            expand_plan(f32::from(window.viewport_size().width), available, self.width);
        // The page must already be heading for the width it will keep after the
        // window grows, otherwise the post-expansion clamp visibly corrects it.
        self.set_width(width);
        if growth > 0. {
            let growth = px(growth);
            self.growth = Some((bounds.size.width, growth));
            // A running slide grows the window frame by frame so the workflow
            // column keeps its size; otherwise hand the space over now.
            if self.reveal.is_none() {
                window.resize(size(bounds.size.width + growth, bounds.size.height));
            }
        }
    }

    pub(crate) fn added_width(&self, width: gpui_kit::Pixels) -> gpui_kit::Pixels {
        self.growth.map_or(gpui_kit::px(0.), |(base, requested)| {
            (width - base).max(gpui_kit::px(0.)).min(requested)
        })
    }

    /// Hands back the width the dock took from the screen.
    pub(crate) fn release(&mut self, window: &mut Window) {
        // A closing slide gives the width back frame by frame and forgets the
        // growth when it finishes.
        if self.reveal.is_some() {
            return;
        }
        let Some((base, requested)) = self.growth else {
            return;
        };
        self.growth = None;
        if window.is_maximized() || window.is_fullscreen() {
            return;
        }
        let bounds = window.bounds();
        let grown = (bounds.size.width - base)
            .max(gpui_kit::px(0.))
            .min(requested);
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
        let result = match (rect, &self.pages[index].view) {
            (Some(rect), None) => {
                let url = self.pages[index].site.url.clone();
                self.build(window, cx, &url, rect).and_then(|view| {
                    resize_x11_child(&view, rect, false)?;
                    self.pages[index].view = Some(Rc::new(view));
                    Ok(())
                })
            }
            (Some(rect), Some(view)) => resize_x11_child(view, rect, applied.is_none()),
            (None, Some(view)) => view.set_visible(false).map_err(|error| error.to_string()),
            (None, None) => Ok(()),
        };
        if result.is_ok() {
            self.pages[index].applied_bounds = rect;
        } else {
            self.close_tab(id);
        }
        result
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
fn resize_x11_child(view: &WebView, rect: Rect, show: bool) -> Result<(), String> {
    use gtk::prelude::*;
    use wry::WebViewExtUnix;
    let widget = view.webview();
    let host = widget
        .toplevel()
        .and_then(|widget| widget.downcast::<gtk::Window>().ok())
        .ok_or_else(|| wry::Error::UnsupportedWindowHandle.to_string())?;
    let scale = f64::from(widget.scale_factor());
    let size = rect.size.to_logical::<i32>(scale);
    let width = size.width.max(1);
    let height = size.height.max(1);
    // Wry's immediate allocation alone leaves GTK free to restore the old
    // toplevel size on its next layout or after a hide/show cycle.
    host.set_default_size(width, height);
    host.resize(width, height);
    view.set_bounds(rect).map_err(|error| error.to_string())?;
    if show {
        // gtk_window_move only queues a move on an unmapped host. Also move
        // the native child while hidden so XMapWindow cannot show stale bounds.
        if let Some(native) = host.window() {
            let position = rect.position.to_logical::<i32>(scale);
            native.move_resize(position.x, position.y, width, height);
        }
        view.set_visible(true).map_err(|error| error.to_string())?;
    }
    Ok(())
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
