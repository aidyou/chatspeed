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
};

use gpui_kit::{canvas, div, prelude::*, AnyElement, Context, Window};
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

pub(crate) const DOCK_WIDTH: f32 = 600.;

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

/// The docked column: one native page per chat site, at most one visible.
pub(crate) struct Dock {
    pages: Vec<Page>,
    active: Option<i64>,
    visible: bool,
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
            visible: true,
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

    pub(crate) fn toggle_visible(&mut self) {
        if self.active.is_none() {
            return;
        }
        self.visible = !self.visible;
        self.sync_visibility();
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
            return;
        }
        let Some(display) = window.display(cx) else {
            return;
        };
        let bounds = window.bounds();
        let area = display.visible_bounds();
        let growth = (area.origin.x + area.size.width - bounds.origin.x - bounds.size.width)
            .max(px(0.))
            .min(px(DOCK_WIDTH));
        if growth > px(0.) {
            self.growth = Some((bounds.size.width, growth));
            window.resize(size(bounds.size.width + growth, bounds.size.height));
        }
    }

    pub(crate) fn added_width(&self, width: gpui_kit::Pixels) -> gpui_kit::Pixels {
        self.growth.map_or(gpui_kit::px(0.), |(base, requested)| {
            (width - base).max(gpui_kit::px(0.)).min(requested)
        })
    }

    /// Hands back the width the dock took from the screen.
    pub(crate) fn release(&mut self, window: &mut Window) {
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
        self.is_empty() && self.growth.is_some()
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
        let active = self.active.and_then(|id| {
            let page = self.pages.iter().find(|page| page.site.id == id)?;
            Some((Rc::downgrade(page.view.as_ref()?), page.bounds.clone()))
        });
        let Some((view, last)) = active else {
            return div().flex_1().min_h_0().w_full().into_any_element();
        };
        canvas(
            |_, _, _| (),
            move |bounds, (), _, _| {
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
