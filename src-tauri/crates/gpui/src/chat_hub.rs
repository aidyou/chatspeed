//! Native child browser for the GPUI ChatHub dock.

use std::{cell::Cell, fs::{File, OpenOptions}, io, path::{Path, PathBuf}, rc::Rc};

use gpui_kit::{canvas, prelude::*, Context, Window};
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

pub(crate) const DOCK_WIDTH: f32 = 600.;

pub(crate) struct ChatHub {
    view: Rc<WebView>,
    growth: Option<(gpui_kit::Pixels, gpui_kit::Pixels)>,
    bounds: Rc<Cell<Option<[f32; 4]>>>,
    #[cfg(not(target_os = "macos"))]
    _context: wry::WebContext,
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

impl ChatHub {
    pub(crate) fn open(window: &Window, url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        if !web_url(url) { return Err("Expected an HTTP(S) address".into()); }
        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            let version = std::process::Command::new("/usr/bin/sw_vers").arg("-productVersion").output()?;
            if !version.status.success() || String::from_utf8_lossy(&version.stdout).split('.').next().and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0) < 14 {
                return Err("Persistent ChatHub requires macOS 14 or later".into());
            }
            WebViewBuilder::new().with_data_store_identifier(*b"CSGPUIChatHubV001")
        };
        let root = profile_root()?;
        let profile_lock = lock_profile(&root)?;
        #[cfg(target_os = "linux")]
        {
            gtk::gdk::set_allowed_backends("x11");
            gtk::init()?;
        }
        #[cfg(not(target_os = "macos"))]
        let mut context = wry::WebContext::new(Some(root.join("web-data")));
        #[cfg(not(target_os = "macos"))]
        let builder = WebViewBuilder::new_with_web_context(&mut context);
        let builder = builder
            .with_visible(false)
            .with_url(url)
            .with_navigation_handler(|url| web_url(&url))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny);
        #[cfg(target_os = "linux")]
        let view = {
            use raw_window_handle::{RawWindowHandle, WindowHandle, XlibWindowHandle};
            let handle = HasWindowHandle::window_handle(window)?;
            let raw = match handle.as_raw() {
                RawWindowHandle::Xlib(xlib) => xlib,
                RawWindowHandle::Xcb(xcb) => XlibWindowHandle::new(xcb.window.get().into()),
                _ => return Err("ChatHub on Linux requires X11 or XWayland".into()),
            };
            struct Parent<'a>(WindowHandle<'a>);
            impl HasWindowHandle for Parent<'_> {
                fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> { Ok(self.0) }
            }
            // The original borrowed handle keeps the same X11 window alive during creation.
            let parent = Parent(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(raw)) });
            builder.build_as_child(&parent)?
        };
        #[cfg(not(target_os = "linux"))]
        let view = builder.build_as_child(window)?;
        Ok(Self { view: Rc::new(view), bounds: Rc::new(Cell::new(None)), growth: None,
            #[cfg(not(target_os = "macos"))]
            _context: context,
            _profile_lock: profile_lock,
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn pump_gtk(&self, cx: &mut Context<crate::Observer>) {
        let view = Rc::downgrade(&self.view);
        cx.spawn(async move |_, cx| {
            while view.strong_count() != 0 {
                for _ in 0..32 {
                    if !gtk::events_pending() { break; }
                    gtk::main_iteration_do(false);
                }
                cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
            }
        }).detach();
    }

    fn expand(&mut self, window: &mut Window, cx: &gpui_kit::App) {
        use gpui_kit::{px, size};
        if window.is_maximized() || window.is_fullscreen() { return; }
        let Some(display) = window.display(cx) else { return; };
        let bounds = window.bounds();
        let area = display.visible_bounds();
        let growth = (area.origin.x + area.size.width - bounds.origin.x - bounds.size.width)
            .max(px(0.)).min(px(DOCK_WIDTH));
        if growth > px(0.) {
            self.growth = Some((bounds.size.width, growth));
            window.resize(size(bounds.size.width + growth, bounds.size.height));
        }
    }

    pub(crate) fn added_width(&self, width: gpui_kit::Pixels) -> gpui_kit::Pixels {
        self.growth.map_or(gpui_kit::px(0.), |(base, requested)| (width - base).max(gpui_kit::px(0.)).min(requested))
    }

    pub(crate) fn close(self, window: &mut Window) {
        let bounds = window.bounds();
        let growth = self.added_width(bounds.size.width);
        drop(self);
        if growth > gpui_kit::px(0.) && !window.is_maximized() && !window.is_fullscreen() {
            window.resize(gpui_kit::size(bounds.size.width - growth, bounds.size.height));
        }
    }

    pub(crate) fn reload(&self) -> wry::Result<()> {
        self.view.reload()
    }

    pub(crate) fn zoom(&self, factor: f64) -> wry::Result<()> {
        self.view.zoom(factor)
    }

    pub(crate) fn dock(&self) -> impl IntoElement {
        let view = Rc::downgrade(&self.view);
        let last = self.bounds.clone();
        canvas(|_, _, _| (), move |bounds, (), window, _| {
            let Some(view) = view.upgrade() else { return; };
            let next = [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height].map(f32::from);
            #[cfg(target_os = "linux")]
            let (next, rect) = {
                use wry::dpi::{PhysicalPosition, PhysicalSize};
                let scale = window.scale_factor();
                let [x, y, w, h] = next;
                let left = (x * scale).round();
                let top = (y * scale).round();
                let width = ((x + w) * scale).round() - left;
                let height = ((y + h) * scale).round() - top;
                ([left, top, width, height], Rect {
                    position: PhysicalPosition::new(left as i32, top as i32).into(),
                    size: PhysicalSize::new(width as u32, height as u32).into(),
                })
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
            if next[2] <= 0. || next[3] <= 0. { let _ = view.set_visible(false); last.set(None); return; }
            if last.get() != Some(next) && view.set_bounds(rect).is_ok() && view.set_visible(true).is_ok() { last.set(Some(next)); }
        }).flex_1().min_h_0().w_full()
    }
}

pub(crate) fn select_site(sites: serde_json::Value, window: &mut Window, cx: &mut Context<crate::Observer>) {
    use gpui_kit::component::{button::Button, WindowExt};
    if window.has_active_dialog(cx) { return; }
    let sites: Vec<_> = sites.as_array().into_iter().flatten().filter_map(|site| {
        let url = site["url"].as_str()?.to_owned();
        let name = site["name"].as_str()?.to_owned();
        web_url(&url).then_some((name, url))
    }).collect();
    let owner = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("ChatHub").child(gpui_kit::div().flex().flex_col().gap_2().children(sites.iter().enumerate().map(|(index, (name, url))| {
            let owner = owner.clone();
            let url = url.clone();
            Button::new(("chat-hub-site", index)).label(name.clone()).on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let _ = owner.update(cx, |this, cx| {
                    match ChatHub::open(window, &url) {
                        Ok(mut hub) => {
                            hub.expand(window, cx);
                            #[cfg(target_os = "linux")]
                            hub.pump_gtk(cx);
                            this.chat_hub = Some(hub);
                        }
                        Err(error) => this.show_toast(crate::ToastKind::Error, error.to_string(), cx),
                    }
                    cx.notify();
                });
            })
        })))
    });
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
