//! Native carrier of a plugin's static UI panel.
//!
//! A plugin UI is a static asset bundle the runtime serves from its loopback gateway. This
//! module is the only owner of the native webviews that render those assets, and it is
//! deliberately built on `wry` directly instead of the Tauri webview APIs: a Tauri webview
//! created inside the Workflow window would inherit that window's capabilities, so the panel
//! could reach ChatSpeed commands. A plain `wry` webview has no Tauri IPC at all, no custom
//! protocol and no remote origin, because the only URL it may load is an asset of the exact
//! loopback prefix the runtime generates in Rust.
//!
//! A panel is placed at the rectangle the frontend measured for the right dock, and it never
//! changes the workflow layout. Every panel and every ChatHub tab share one bounded dock surface
//! ([`crate::native_dock`]), so a panel is a bounded overlay child on Linux and a child view at
//! its rectangle on Windows and macOS: no native widget ever covers the workflow UI outside its
//! rectangle, which is what kept the workflow UI from receiving clicks while a panel was open.
//!
//! Every entry point must run on the platform main thread, where the GTK tree and the
//! webviews live. The external commands post their work there with
//! `AppHandle::run_on_main_thread`, exactly like the ChatHub page commands do.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, Wry};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

use crate::chat_hub::host_window;
use crate::native_dock;
#[cfg(target_os = "linux")]
use crate::native_dock::DockOwner;

/// Rectangle of one plugin panel inside the Workflow window, in logical pixels.
///
/// It is the same rectangle type every native dock view uses ([`crate::native_dock::DockBounds`]),
/// so a plugin panel and a ChatHub tab are placed by the same rules. The frontend measures it on
/// the panel's own placeholder, so the host only places the native view: how much room the
/// workflow layout keeps free stays the frontend's business.
pub use crate::native_dock::DockBounds as PluginUiBounds;

/// Origin of the loopback gateway every plugin UI asset is served from.
///
/// The runtime binds the gateway on `127.0.0.1`, so an origin that is not exactly this one is
/// never a plugin UI source and is refused before it reaches a webview.
const LOOPBACK_ORIGIN: &str = "http://127.0.0.1:";

/// Longest asset path a plugin panel may load.
///
/// It mirrors the runtime's own bundle path limit (`is_safe_relative_path`) so the host and
/// the gateway agree on which paths are reachable.
const MAX_ASSET_PATH: usize = 512;

/// Owns every plugin UI panel, keyed by the tab that opened it.
///
/// A tab keeps its own webview so navigating between panels costs nothing. One panel is
/// destroyed by [`PluginUiHost::close`], every panel by [`PluginUiHost::close_all`] while the
/// window is still alive, and [`PluginUiHost::clear`] only releases the handles of a window
/// that is already going away.
///
/// Every method must be called on the platform main thread; see the module documentation.
pub struct PluginUiHost {
    tabs: Mutex<HashMap<String, Tab>>,
}

/// One plugin panel: the native view and the asset it is currently showing.
struct Tab {
    /// The window the panel was opened in, kept so a close can reach the shared dock surface
    /// without a second handle from the caller.
    app: AppHandle<Wry>,
    url: String,
    page: carrier::Page,
}

impl std::fmt::Debug for PluginUiHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PluginUiHost").finish_non_exhaustive()
    }
}

impl Default for PluginUiHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginUiHost {
    /// Creates an empty host; no native view exists until a panel is shown.
    pub fn new() -> Self {
        Self {
            tabs: Mutex::new(HashMap::new()),
        }
    }

    /// Reveals `tab_id` at `bounds`, keeping every other tab's session alive but hidden.
    ///
    /// `url` and `prefix` are built inside the runtime and never come from the frontend:
    /// `url` is accepted only when it is an asset of the exact loopback `prefix`, so a panel
    /// can never be pointed at a remote origin or at another plugin's bundle.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        tab_id: &str,
        url: &str,
        prefix: &str,
        bounds: PluginUiBounds,
    ) -> Result<(), String> {
        ensure_main_thread()?;

        if tab_id.is_empty() {
            return Err("the plugin UI tab id is empty".to_string());
        }
        if !is_loopback_prefix(prefix) {
            return Err("the plugin UI prefix is not a loopback gateway base".to_string());
        }
        if !is_allowed_asset_url(prefix, url) {
            return Err("the plugin UI url is not a safe asset of the gateway prefix".to_string());
        }
        // The rectangle is fitted to the window here, so every carrier places the panel inside a
        // geometry the window can hold.
        let bounds = sanitize_bounds(app, bounds)?;

        carrier::show(self, app, tab_id, url, prefix, bounds)
    }

    /// Hides every panel without destroying it, so each tab keeps its session.
    pub fn hide(&self) -> Result<(), String> {
        ensure_main_thread()?;

        let tabs = self.lock_tabs()?;
        for tab in tabs.values() {
            tab.page.set_visible(false)?;
        }

        Ok(())
    }

    /// Destroys one panel, detaching its native view and dropping its browsing context.
    ///
    /// Closing a tab the host does not know is a no-op, so a command that races the frontend
    /// cannot fail on a panel that is already gone.
    pub fn close(&self, tab_id: &str) -> Result<(), String> {
        ensure_main_thread()?;

        let tab = self.lock_tabs()?.remove(tab_id);
        if let Some(tab) = tab {
            tab.page.destroy(&tab.app)?;
        }

        Ok(())
    }

    /// Destroys every panel and releases the dock views of this owner, for a tab set that is being
    /// invalidated while the Workflow window stays open.
    ///
    /// Unlike [`PluginUiHost::clear`], every native view is detached before it is dropped and the
    /// dock holders of this owner are removed, so the window (and a docked ChatHub tab, which
    /// belongs to the other owner) is left exactly as it was before the first panel opened.
    pub fn close_all(&self, app: &AppHandle<Wry>) -> Result<(), String> {
        ensure_main_thread()?;

        {
            let mut tabs = self.lock_tabs()?;
            for (_, tab) in tabs.drain() {
                tab.page.destroy(&tab.app)?;
            }
        }

        carrier::close_all(self, app)
    }

    /// Drops every panel handle, for when the Workflow window goes away.
    ///
    /// This is the window-destroy path: the widgets are already being torn down with the
    /// window, so the handles are only released here and no GTK call is made on them. Use
    /// [`PluginUiHost::close`] or [`PluginUiHost::close_all`] to remove panels while the window
    /// is still alive.
    pub fn clear(&self) -> Result<(), String> {
        if let Ok(mut tabs) = self.tabs.lock() {
            tabs.clear();
        }

        Ok(())
    }

    /// Locks the tab map, reporting a poisoned lock as a plain error instead of a panic.
    fn lock_tabs(&self) -> Result<MutexGuard<'_, HashMap<String, Tab>>, String> {
        self.tabs
            .lock()
            .map_err(|_| "the plugin UI host state is poisoned".to_string())
    }
}

/// Whether `prefix` is the loopback gateway base a plugin UI may be served from.
///
/// It must be `http://127.0.0.1:<port>/<path>/`: a real loopback port, a path that is a
/// directory (so the asset always trails it), and no query or fragment that could smuggle a
/// token into the origin the webview is loaded from.
fn is_loopback_prefix(prefix: &str) -> bool {
    let Some(rest) = prefix.strip_prefix(LOOPBACK_ORIGIN) else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };

    !port.is_empty()
        && port.len() <= 5
        && port.chars().all(|character| character.is_ascii_digit())
        && !path.is_empty()
        && path.ends_with('/')
        && !prefix.contains(['?', '#'])
        && !prefix.chars().any(char::is_whitespace)
}

/// Whether `url` is a plugin UI asset of `prefix`.
///
/// The prefix is matched byte for byte, so `.../plugin_a/` never admits `.../plugin_ab/...`,
/// and what trails it must be a safe bundle-relative asset path.
fn is_allowed_asset_url(prefix: &str, url: &str) -> bool {
    match url.strip_prefix(prefix) {
        Some(asset) => is_safe_asset_path(asset),
        None => false,
    }
}

/// Whether `asset` is a bundle-relative asset path with no query or fragment.
///
/// This mirrors the runtime's own bundle rule (`is_safe_relative_path`): relative,
/// `/`-separated and free of `.`/`..`/drive/`~`/control components, with the addition that a
/// URL part can carry neither a query nor a fragment.
fn is_safe_asset_path(asset: &str) -> bool {
    if asset.is_empty() || asset.len() > MAX_ASSET_PATH {
        return false;
    }
    if asset.starts_with('/') || asset.starts_with('\\') || asset.ends_with('/') {
        return false;
    }
    if asset.contains(['\\', '?', '#']) {
        return false;
    }

    asset.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part != "~"
            && !part.contains(':')
            && !part.chars().any(char::is_control)
    })
}

/// Fits a measured panel rectangle into the Workflow window, or refuses one that cannot describe
/// a view.
///
/// The rectangle comes from the frontend's own measurement, so a resize caught mid-frame can
/// carry a non-finite or empty value. The shared dock rule refuses such a rectangle instead of
/// handing it to a native view, and pulls an offset outside the window back to its edge.
fn sanitize_bounds(app: &AppHandle<Wry>, bounds: PluginUiBounds) -> Result<PluginUiBounds, String> {
    let host = host_window(app).map_err(|error| error.to_string())?;
    let (width, height) = native_dock::window_size(&host).map_err(|error| error.to_string())?;

    bounds
        .sanitize(width, height)
        .map_err(|error| error.to_string())
}

/// Whether the current thread owns the native UI.
///
/// The webviews and GTK widgets may only be touched on that thread. On Linux the GTK main
/// context answers the question; on Windows and macOS the window thread is the platform main
/// thread and there is no cheap probe, so the caller's contract is trusted there.
fn ensure_main_thread() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if !gtk::glib::MainContext::default().is_owner() {
        return Err("the plugin UI host must run on the main thread".to_string());
    }

    Ok(())
}

/// Builds a plugin UI webview with the rules that hold on every platform.
///
/// The panel may only navigate to a safe asset of the gateway `prefix`, a new window request
/// is refused instead of opening an unmanaged window, and no IPC, custom protocol or proxy is
/// ever installed: the loopback gateway is reached directly and the panel can reach nothing
/// else. The gateway's own `Content-Security-Policy` keeps the loaded document from pulling
/// remote subresources.
fn page_builder<'a>(context: &'a mut WebContext, url: &str, prefix: &str) -> WebViewBuilder<'a> {
    let allowed_prefix = prefix.to_string();

    WebViewBuilder::new_with_web_context(context)
        .with_url(url)
        .with_navigation_handler(move |target| is_allowed_asset_url(&allowed_prefix, &target))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
}

#[cfg(target_os = "linux")]
mod carrier {
    use super::*;
    use wry::{WebContext, WebView, WebViewBuilderExtUnix};

    use crate::native_dock::DockHolder;

    /// Reveals `tab_id` at `bounds`, keeping every other tab alive but hidden.
    pub(super) fn show(
        host: &PluginUiHost,
        app: &AppHandle<Wry>,
        tab_id: &str,
        url: &str,
        prefix: &str,
        bounds: PluginUiBounds,
    ) -> Result<(), String> {
        let mut tabs = host.lock_tabs()?;

        if !tabs.contains_key(tab_id) {
            let page = Page::create(app, url, prefix, bounds)?;
            tabs.insert(
                tab_id.to_string(),
                Tab {
                    app: app.clone(),
                    url: url.to_string(),
                    page,
                },
            );
        }

        // A single panel is active at a time, so revealing one hides the rest without
        // touching their sessions.
        for (id, tab) in tabs.iter_mut() {
            if id == tab_id {
                if tab.url != url {
                    tab.page.load_url(url)?;
                    tab.url = url.to_string();
                }
                tab.page.set_bounds(bounds)?;
                tab.page.set_visible(true)?;
            } else {
                tab.page.set_visible(false)?;
            }
        }

        Ok(())
    }

    /// Removes every dock view this owner placed, leaving a ChatHub tab of the other owner in
    /// place. The shared overlay is released once no view is left.
    pub(super) fn close_all(_host: &PluginUiHost, app: &AppHandle<Wry>) -> Result<(), String> {
        native_dock::with_surface(app, |surface| {
            surface.remove_owner(app, DockOwner::Plugin);
            Ok(())
        })
        .map_err(|error| error.to_string())
    }

    /// One plugin panel: a webview inside a bounded holder of the shared dock surface.
    pub(super) struct Page {
        webview: WebView,
        holder: DockHolder,
    }

    // SAFETY: both handles are GTK objects that may only be touched on the main thread, and
    // every entry point of the host runs there.
    unsafe impl Send for Page {}

    impl Page {
        /// Builds the panel into its own bounded holder at the measured rectangle.
        ///
        /// The holder is an overlay child aligned to the start of the shared overlay, so the panel
        /// only receives input inside its own rectangle: the workflow UI keeps every event no panel
        /// covers.
        fn create(
            app: &AppHandle<Wry>,
            url: &str,
            prefix: &str,
            bounds: PluginUiBounds,
        ) -> Result<Self, String> {
            let holder = native_dock::with_surface(app, |surface| {
                surface.holder(app, DockOwner::Plugin, bounds)
            })
            .map_err(|error| error.to_string())?;

            // An ephemeral context is private to this tab: it shares no profile, no cookies
            // and no storage with the ChatHub page or with another tab.
            let mut context = WebContext::new(None);
            let webview = page_builder(&mut context, url, prefix)
                .build_gtk(holder.container())
                .map_err(|error| format!("failed to build the plugin UI webview: {error}"))?;

            holder.set_visible(true);

            Ok(Self { webview, holder })
        }

        fn set_bounds(&self, bounds: PluginUiBounds) -> Result<(), String> {
            self.holder.set_bounds(bounds);

            Ok(())
        }

        pub(super) fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.holder.set_visible(visible);

            Ok(())
        }

        fn load_url(&self, url: &str) -> Result<(), String> {
            self.webview
                .load_url(url)
                .map_err(|error| format!("failed to navigate the plugin UI: {error}"))
        }

        pub(super) fn destroy(&self, app: &AppHandle<Wry>) -> Result<(), String> {
            // Detaching the holder is what drops the webview's last parent; the holder goes away
            // with the panel handle, and the overlay with the last dock view.
            self.holder.set_visible(false);
            native_dock::with_surface(app, |surface| {
                surface.remove(app, &self.holder);
                Ok(())
            })
            .map_err(|error| error.to_string())
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod carrier {
    use super::*;
    use tauri::WebviewWindow;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebContext, WebView};

    /// Reveals `tab_id` at `bounds`, keeping every other tab alive but hidden.
    pub(super) fn show(
        host: &PluginUiHost,
        app: &AppHandle<Wry>,
        tab_id: &str,
        url: &str,
        prefix: &str,
        bounds: PluginUiBounds,
    ) -> Result<(), String> {
        let window = host_window(app).map_err(|error| error.to_string())?;
        let mut tabs = host.lock_tabs()?;

        if !tabs.contains_key(tab_id) {
            let page = Page::create(&window, url, prefix, bounds)?;
            tabs.insert(
                tab_id.to_string(),
                Tab {
                    app: app.clone(),
                    url: url.to_string(),
                    page,
                },
            );
        }

        // A single panel is active at a time, so revealing one hides the rest without
        // touching their sessions.
        for (id, tab) in tabs.iter_mut() {
            if id == tab_id {
                if tab.url != url {
                    tab.page.load_url(url)?;
                    tab.url = url.to_string();
                }
                tab.page.set_bounds(bounds)?;
                tab.page.set_visible(true)?;
            } else {
                tab.page.set_visible(false)?;
            }
        }

        Ok(())
    }

    /// One plugin panel, a child view of the Workflow window placed at its rectangle.
    pub(super) struct Page {
        webview: WebView,
    }

    // SAFETY: a `wry` webview may only be touched on the main thread, and every entry point of
    // the host runs there.
    unsafe impl Send for Page {}

    impl Page {
        fn create(
            window: &WebviewWindow<Wry>,
            url: &str,
            prefix: &str,
            bounds: PluginUiBounds,
        ) -> Result<Self, String> {
            // An ephemeral context is private to this tab: it shares no profile, no cookies
            // and no storage with the ChatHub page or with another tab.
            let mut context = WebContext::new(None);
            let webview = page_builder(&mut context, url, prefix)
                .with_bounds(rect(bounds))
                .build_as_child(window)
                .map_err(|error| format!("failed to build the plugin UI webview: {error}"))?;

            Ok(Self { webview })
        }

        fn set_bounds(&self, bounds: PluginUiBounds) -> Result<(), String> {
            self.webview
                .set_bounds(rect(bounds))
                .map_err(|error| format!("failed to place the plugin UI panel: {error}"))
        }

        pub(super) fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.webview
                .set_visible(visible)
                .map_err(|error| format!("failed to show the plugin UI panel: {error}"))
        }

        fn load_url(&self, url: &str) -> Result<(), String> {
            self.webview
                .load_url(url)
                .map_err(|error| format!("failed to navigate the plugin UI: {error}"))
        }

        pub(super) fn destroy(&self, _app: &AppHandle<Wry>) -> Result<(), String> {
            // A child view is destroyed with its handle, which the caller drops right after
            // this returns.
            Ok(())
        }
    }

    /// No overlay exists on this carrier: the host already dropped every child view.
    pub(super) fn close_all(_host: &PluginUiHost, _app: &AppHandle<Wry>) -> Result<(), String> {
        Ok(())
    }

    /// Rectangle of a panel as the child view API takes it.
    fn rect(bounds: PluginUiBounds) -> Rect {
        Rect {
            position: LogicalPosition::new(bounds.x, bounds.y).into(),
            size: LogicalSize::new(bounds.width, bounds.height).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gateway prefix the runtime hands the host.
    const PREFIX: &str = "http://127.0.0.1:51234/ui/capability/agent-skills/";

    #[test]
    fn a_panel_is_only_served_from_the_loopback_gateway() {
        assert!(is_loopback_prefix(PREFIX));
        // A remote origin is refused whatever path it carries.
        assert!(!is_loopback_prefix(
            "http://example.com/ui/capability/agent-skills/"
        ));
        // So is a loopback origin without a port, or without a trailing directory.
        assert!(!is_loopback_prefix(
            "http://127.0.0.1/ui/capability/agent-skills/"
        ));
        assert!(!is_loopback_prefix("http://127.0.0.1:51234"));
        assert!(!is_loopback_prefix(
            "http://127.0.0.1:51234/ui/capability/agent-skills"
        ));
        // A query can never be part of a base the webview is loaded from.
        assert!(!is_loopback_prefix("http://127.0.0.1:51234/ui/?token=1"));
    }

    #[test]
    fn a_panel_url_must_stay_under_the_prefix_on_a_safe_asset() {
        assert!(is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/agent-skills/index.html"
        ));
        assert!(is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/agent-skills/assets/app.js"
        ));

        // Another plugin's bundle is not this panel's asset.
        assert!(!is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/other/index.html"
        ));
        // A remote origin cannot be reached through a matching path.
        assert!(!is_allowed_asset_url(
            PREFIX,
            "http://example.com/ui/capability/agent-skills/index.html"
        ));
        // A query, a fragment, or a traversal never reaches the webview.
        assert!(!is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/agent-skills/index.html?v=1"
        ));
        assert!(!is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/agent-skills/index.html#top"
        ));
        assert!(!is_allowed_asset_url(
            PREFIX,
            "http://127.0.0.1:51234/ui/capability/agent-skills/../secret"
        ));
        // The prefix itself names no asset.
        assert!(!is_allowed_asset_url(PREFIX, PREFIX));
    }

    #[test]
    fn the_carrier_keeps_the_plugin_ui_isolated_from_the_host() {
        let source = production_source();

        // The panel is built with `wry` directly (`WebViewBuilder`); the Tauri webview type
        // differs only in capitalization (`WebviewBuilder`), and it must never appear.
        assert!(!source.contains("WebviewBuilder"));
        // No IPC or custom protocol is installed on the panel.
        assert!(!source.contains("with_ipc_handler"));
        assert!(!source.contains("with_asynchronous_custom_protocol"));
        assert!(!source.contains("with_custom_protocol"));
        // A new window request is refused instead of opening an unmanaged window.
        assert!(source.contains("NewWindowResponse::Deny"));
        // Every load is judged by the gateway prefix.
        assert!(source.contains("with_navigation_handler"));
        // A panel's browsing context is private and ephemeral.
        assert!(source.contains("WebContext::new(None)"));
    }

    /// Guard for the input path: a panel is a bounded holder of the shared dock surface, never a
    /// full-area container that would swallow the workflow UI's pointer events.
    #[test]
    fn a_panel_is_a_bounded_holder_of_the_dock_surface() {
        let source = production_source();

        assert!(source.contains("native_dock::with_surface"));
        assert!(source.contains("surface.holder(app, DockOwner::Plugin, bounds)"));
        assert!(source.contains("surface.remove_owner(app, DockOwner::Plugin)"));
        // The full-area container that used to swallow the window's pointer events is gone.
        assert!(!source.contains("Fixed::new"));
        assert!(!source.contains("set_hexpand(true)"));
        assert!(!source.contains("set_vexpand(true)"));
    }

    /// The production half of this file, so an assertion can never match its own text.
    fn production_source() -> &'static str {
        include_str!("host.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("host.rs carries a test module")
    }
}
