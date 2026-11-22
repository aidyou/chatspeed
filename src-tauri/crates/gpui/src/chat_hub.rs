//! Native child browser for the GPUI ChatHub dock.
//!
//! Vue keeps one page per chat site and shows the active one in a shared right
//! dock; the GPUI window does the same with one wry child view per tab.

use std::{cell::Cell, fs::File, fs::OpenOptions, io, path::{Path, PathBuf}, rc::Rc};

use gpui_kit::{canvas, div, prelude::*, AnyElement, Context, Window};
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

pub(crate) const DOCK_WIDTH: f32 = 600.;

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

pub(crate) fn sites_from_json(sites: &serde_json::Value) -> Vec<Site> {
    let mut sites: Vec<Site> = sites
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Site::from_json)
        .collect();
    sites.sort_by_key(|site| site.id);
    sites
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
    view: Rc<WebView>,
    bounds: Rc<Cell<Option<[f32; 4]>>>,
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
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")));
    let base = base.filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ChatHub profile directory unavailable"))?;
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
        std::fs::TryLockError::WouldBlock => io::Error::new(io::ErrorKind::WouldBlock, "ChatHub profile is already in use"),
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
            let view = self.build(window, &site.url)?;
            self.pages.push(Page {
                site: site.clone(),
                view: Rc::new(view),
                bounds: Rc::new(Cell::new(None)),
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

    pub(crate) fn close_tab(&mut self, id: i64) {
        self.pages.retain(|page| page.site.id != id);
        if self.active == Some(id) {
            self.active = self.pages.last().map(|page| page.site.id);
        }
        if self.pages.is_empty() {
            self.visible = false;
        }
        self.sync_visibility();
    }

    fn sync_visibility(&self) {
        for page in &self.pages {
            let shown = self.visible && self.active == Some(page.site.id);
            if !shown {
                // Forgetting the last rectangle makes the next frame re-apply the
                // geometry: a page that was only hidden otherwise keeps whatever
                // size the platform restored, which showed up as a tiny view.
                page.bounds.set(None);
            }
            let _ = page.view.set_visible(shown);
        }
    }

    fn build(&mut self, window: &mut Window, url: &str) -> Result<WebView, String> {
        if !web_url(url) {
            return Err("Expected an HTTP(S) address".into());
        }
        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            WebViewBuilder::new().with_data_store_identifier(*b"CSGPUIChatHubV01")
        };
        #[cfg(not(target_os = "macos"))]
        let builder = WebViewBuilder::new_with_web_context(&mut self.context);
        let builder = builder
            .with_visible(false)
            .with_url(url)
            .with_background_color(self.background)
            .with_navigation_handler(|url| web_url(&url))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny);
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
            let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
            let raw = match handle.as_raw() {
                RawWindowHandle::Xlib(xlib) => xlib,
                RawWindowHandle::Xcb(xcb) => XlibWindowHandle::new(xcb.window.get().into()),
                _ => return Err("ChatHub on Linux requires X11 or XWayland".into()),
            };
            struct Parent<'a>(WindowHandle<'a>);
            impl HasWindowHandle for Parent<'_> {
                fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
                    Ok(self.0)
                }
            }
            // The original borrowed handle keeps the same X11 window alive during creation.
            let parent = Parent(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(raw)) });
            builder.build_as_child(&parent).map_err(|error| error.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            builder.build_as_child(window).map_err(|error| error.to_string())
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

    pub(crate) fn close(mut self, window: &mut Window) {
        self.release(window);
    }

    /// The active page's canvas; the other pages stay hidden until selected.
    pub(crate) fn dock(&self) -> AnyElement {
        let active = self.active.and_then(|id| {
            self.pages
                .iter()
                .find(|page| page.site.id == id)
                .map(|page| (page.view.clone(), page.bounds.clone()))
        });
        let Some((view, last)) = active else {
            return div().flex_1().min_h_0().w_full().into_any_element();
        };
        let view = Rc::downgrade(&view);
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                let next = [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height]
                    .map(f32::from);
                #[cfg(target_os = "linux")]
                let (next, rect) = {
                    use wry::dpi::{PhysicalPosition, PhysicalSize};
                    let scale = window.scale_factor();
                    let [x, y, w, h] = next;
                    let left = (x * scale).round();
                    let top = (y * scale).round();
                    let width = ((x + w) * scale).round() - left;
                    let height = ((y + h) * scale).round() - top;
                    (
                        [left, top, width, height],
                        Rect {
                            position: PhysicalPosition::new(left as i32, top as i32).into(),
                            size: PhysicalSize::new(width as u32, height as u32).into(),
                        },
                    )
                };
                #[cfg(not(target_os = "linux"))]
                let rect = {
                    let _ = window;
                    use wry::dpi::{LogicalPosition, LogicalSize};
                    Rect {
                        position: LogicalPosition::new(next[0], next[1]).into(),
                        size: LogicalSize::new(next[2], next[3]).into(),
                    }
                };
                if next[2] <= 0. || next[3] <= 0. {
                    let _ = view.set_visible(false);
                    last.set(None);
                    return;
                }
                if last.get() != Some(next)
                    && view.set_bounds(rect).is_ok()
                    && view.set_visible(true).is_ok()
                {
                    last.set(Some(next));
                }
            },
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .into_any_element()
    }
}

fn web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_profile_lock_releases_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let lock = lock_profile(dir.path()).unwrap();
    assert_eq!(lock_profile(dir.path()).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    drop(lock);
    assert!(lock_profile(dir.path()).is_ok());
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn chat_hub_rejects_non_web_navigation() {
    assert!(web_url("https://example.com/"));
    assert!(web_url("http://localhost:8080/"));
    for url in ["file:///tmp/test", "javascript:alert(1)", "data:text/html,test", "about:blank"] {
        assert!(!web_url(url));
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn site_list_skips_entries_without_a_web_url() {
    let sites = sites_from_json(&serde_json::json!([
        {"id": 2, "name": "B", "url": "https://b.example/", "logo": ""},
        {"id": 1, "name": "A", "url": "file:///tmp/a", "logo": ""},
        {"id": 3, "name": "C", "url": "https://c.example/", "logo": "https://c.example/i.png"}
    ]));
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0].name, "B");
    assert_eq!(sites[1].logo, "https://c.example/i.png");
}
