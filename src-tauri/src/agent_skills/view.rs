//! State and rules of the docked Agent Skills surface.
//!
//! The state owns at most one native view: the trusted surface is a single page, and the dock
//! offers exactly one tab for it. Every entry point must run on the platform main thread, where
//! the webview and the GTK tree live; the external commands post their work there with
//! `AppHandle::run_on_main_thread`.

use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, Manager, Url, WebviewBuilder, WebviewUrl, Window, Wry};

use crate::chat_hub::CHAT_HUB_HOST_WINDOW_LABEL;
use crate::native_dock::{self, DockBounds};

use super::carrier;

/// Label of the trusted Agent Skills webview docked into the Workflow window.
///
/// It is a webview label, never a window label: the surface shares the Workflow window with the
/// workflow UI, so it can never become a second top-level window.
pub const AGENT_SKILLS_WEBVIEW_LABEL: &str = "agent_skills";

/// Route of the trusted management surface inside the application bundle.
pub const AGENT_SKILLS_ROUTE: &str = "/agent-skills";

/// Owns the single docked Agent Skills view.
pub struct AgentSkillsView {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The live native view, when the surface is open.
    page: Option<carrier::Page>,
}

// SAFETY: the page owns a GTK widget and a Tauri webview on Linux, which may only be touched on
// the main thread. Every entry point of this state runs there.
unsafe impl Send for Inner {}

impl Default for AgentSkillsView {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentSkillsView {
    /// Creates an empty view; no native webview exists until the surface is shown.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Shows or re-places the surface at the rectangle the frontend measured.
    ///
    /// The first show builds the webview; every later show only moves the live view and updates
    /// the corner radius and the frame band, so the browsing session survives a resize and a
    /// reveal. `bounds` is fitted to the window, so a measurement caught mid-frame is refused
    /// instead of being handed to a native view.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        bounds: DockBounds,
        corner_radius: f64,
    ) -> Result<(), String> {
        let window = host_tauri_window(app)?;
        let (window_width, window_height) =
            native_dock::window_size(&window).map_err(|error| error.to_string())?;
        let bounds = bounds
            .sanitize(window_width, window_height)
            .map_err(|error| error.to_string())?;
        let radius = native_dock::clamp_corner_radius(corner_radius);

        let mut inner = self.lock()?;
        match inner.page.as_mut() {
            Some(page) => page.show(app, bounds, radius),
            None => {
                let page = carrier::Page::create(app, bounds, radius)?;
                if let Err(error) = page.show(app, bounds, radius) {
                    // The webview already exists, so release it before reporting: an untracked live
                    // view would keep its label taken and make a later open collide with a surface
                    // nobody owns.
                    let _ = page.destroy(app);
                    return Err(error);
                }
                inner.page = Some(page);
                Ok(())
            }
        }
    }

    /// Hides the surface without releasing its webview, so its session survives.
    pub fn hide(&self) -> Result<(), String> {
        let inner = self.lock()?;
        match inner.page.as_ref() {
            Some(page) => page.set_visible(false),
            None => Ok(()),
        }
    }

    /// Releases the surface: the child webview is closed and its dock holder detached.
    ///
    /// Closing the child never touches the Workflow window it is docked into, so a surface that
    /// left the dock (or was invalidated) leaves the window and every other docked view in place.
    pub fn close(&self, app: &AppHandle<Wry>) -> Result<(), String> {
        let mut inner = self.lock()?;
        if inner.page.is_none() {
            return Ok(());
        }

        // The tracked handle stays until the child is actually gone: a close that fails has to be
        // retryable, otherwise the next open would believe the surface was released while its
        // webview was still live under the same label.
        if let Some(page) = inner.page.as_ref() {
            page.destroy(app)?;
        }
        inner.page = None;

        Ok(())
    }

    /// Drops the tracked handle when the Workflow window goes away.
    ///
    /// This is the window-destroy path: the webview is being torn down with its window, so only
    /// the handle is released here and no native call is made on it.
    pub fn release(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.page = None;
        }
    }

    /// Locks the state, reporting a poisoned lock as a plain error instead of a panic.
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, String> {
        self.inner
            .lock()
            .map_err(|_| "the Agent Skills view state is poisoned".to_string())
    }
}

/// The scheme, host and port that together identify the application origin the surface may reach.
///
/// The trusted surface ships with the application, so it may only ever navigate within the
/// application's own origin: the bundled assets in a release build and the configured dev server
/// while developing. The origin is carried as its three parts instead of an [`Url`] so a
/// navigation can be compared against it exactly, which is what keeps the surface off every other
/// host, port and scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AppOrigin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl AppOrigin {
    /// The origin of an already-resolved application URL, such as the configured dev server.
    fn from_url(url: &Url) -> Self {
        Self {
            scheme: url.scheme().to_string(),
            host: url.host_str().unwrap_or_default().to_string(),
            // The effective port, so an omitted default port and an explicit one are the same
            // origin instead of two.
            port: url.port_or_known_default(),
        }
    }

    /// The origin the bundled assets are served from in a release build.
    ///
    /// `WebviewUrl::App` resolves to the platform's custom protocol: `tauri://localhost` on Linux
    /// and macOS and `http://tauri.localhost` on Windows. The Windows protocol uses `http`, whose
    /// effective port is the default `80` even when the URL omits it, so the origin is compared
    /// against that port exactly like a navigation URL is.
    fn production() -> Self {
        if cfg!(windows) {
            Self {
                scheme: "http".to_string(),
                host: "tauri.localhost".to_string(),
                port: Some(80),
            }
        } else {
            Self {
                scheme: "tauri".to_string(),
                host: "localhost".to_string(),
                port: None,
            }
        }
    }

    /// Whether a URL addresses this exact origin.
    fn matches(&self, url: &Url) -> bool {
        self.scheme == url.scheme()
            && self.host == url.host_str().unwrap_or_default()
            && self.port == url.port_or_known_default()
    }
}

/// The application origin the trusted surface may navigate within, resolved once per webview.
///
/// A debug build serves the app from the dev server Tauri is configured with (the same URL
/// `WebviewUrl::App` resolves against), so the surface follows the actual development origin
/// instead of a guessed one; a release build serves the bundled assets over the platform's custom
/// protocol.
fn expected_origin(app: &AppHandle<Wry>) -> AppOrigin {
    if cfg!(debug_assertions) {
        if let Some(dev_url) = app.config().build.dev_url.as_ref() {
            return AppOrigin::from_url(dev_url);
        }
    }

    AppOrigin::production()
}

/// Builds the trusted surface webview with the rules that hold on every platform.
///
/// The webview is a Tauri child of the Workflow window, so it receives the desktop IPC the
/// management surface needs, unlike an ordinary plugin panel. What it may do with that reach is
/// bounded here: it may only navigate to the application's own origin and route (never remote
/// content), and a new window request is refused instead of opening an unmanaged window.
pub(crate) fn agent_skills_builder(app: &AppHandle<Wry>) -> WebviewBuilder<Wry> {
    let expected = expected_origin(app);

    WebviewBuilder::new(
        AGENT_SKILLS_WEBVIEW_LABEL,
        WebviewUrl::App(AGENT_SKILLS_ROUTE.into()),
    )
    .on_navigation(move |url| is_agent_skills_origin(url, &expected))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
}

/// The Workflow window the surface is docked into, as the window builder needs it.
pub(super) fn host_tauri_window(app: &AppHandle<Wry>) -> Result<Window<Wry>, String> {
    app.get_window(CHAT_HUB_HOST_WINDOW_LABEL)
        .ok_or_else(|| {
            format!(
                "the Workflow window '{}' is not available",
                CHAT_HUB_HOST_WINDOW_LABEL
            )
        })
}

/// Whether a navigation stays on the trusted surface's own origin and route.
///
/// The surface ships with the application, so it may only ever navigate within the application's
/// own origin and the one route it renders. The origin is compared exactly (scheme, host and
/// port), and credentials and a query string are refused, so the surface can neither be pointed at
/// another host nor redirected elsewhere with a query or an embedded authority. A fragment only
/// addresses a place inside the page and is kept.
fn is_agent_skills_origin(url: &Url, expected: &AppOrigin) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.query().is_some() {
        return false;
    }

    url.path() == AGENT_SKILLS_ROUTE && expected.matches(url)
}

/// Runs one Agent Skills view operation on the platform main thread and waits for its result.
///
/// The webview and the GTK tree may only be touched on that thread, so the work is posted there
/// and its result is awaited on the caller: the frontend then learns about a failure immediately
/// instead of the surface silently staying away. The wait happens on a blocking worker, never on
/// the main thread, so the callback can always run to completion.
pub async fn run_on_view_thread(
    app: &AppHandle<Wry>,
    operation: impl FnOnce(&AgentSkillsView, &AppHandle<Wry>) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread_app = app.clone();

    app.run_on_main_thread(move || {
        let result = match thread_app.try_state::<AgentSkillsView>() {
            Some(view) => operation(view.inner(), &thread_app),
            None => Err("the Agent Skills view state is not managed".to_string()),
        };
        let _ = sender.send(result);
    })
    .map_err(|error| error.to_string())?;

    tokio::task::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|error| format!("the Agent Skills view task failed: {error}"))
        .and_then(|received| match received {
            Ok(result) => result,
            Err(_) => Err("the Agent Skills view task did not report a result".to_string()),
        })
}

#[cfg(test)]
mod tests {
    /// The production half of this file, so an assertion can never match its own text.
    fn production_source() -> &'static str {
        include_str!("view.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("view.rs carries a test module")
    }

    #[test]
    fn the_surface_only_ever_navigates_within_the_application_origin() {
        let dev = origin("http", "localhost", Some(1420));

        // The dev server origin is the same application while developing, with or without a
        // fragment that only addresses a place inside the page.
        assert!(super::is_agent_skills_origin(
            &url("http://localhost:1420/agent-skills"),
            &dev
        ));
        assert!(super::is_agent_skills_origin(
            &url("http://localhost:1420/agent-skills#settings"),
            &dev
        ));

        // Remote content, a local file and a foreign scheme are all refused.
        assert!(!super::is_agent_skills_origin(
            &url("https://example.com/agent-skills"),
            &dev
        ));
        assert!(!super::is_agent_skills_origin(&url("file:///etc/passwd"), &dev));
        assert!(!super::is_agent_skills_origin(&url("about:blank"), &dev));
        assert!(!super::is_agent_skills_origin(
            &url("chatspeed://open?url=https://example.com"),
            &dev
        ));
    }

    /// The surface is locked to one origin and one route: another host, port, scheme, route or an
    /// embedded authority is a different place, and a query could name a redirect target.
    #[test]
    fn the_surface_refuses_another_host_port_scheme_route_or_authority() {
        let dev = origin("http", "localhost", Some(1420));

        // Another host, even one that borrows the `tauri` scheme or looks like the app host.
        assert!(!super::is_agent_skills_origin(
            &url("tauri://evil/agent-skills"),
            &dev
        ));
        assert!(!super::is_agent_skills_origin(
            &url("http://evil.test/agent-skills"),
            &dev
        ));
        // Another port on the same host is a different origin, so an arbitrary local server is not
        // the application.
        assert!(!super::is_agent_skills_origin(
            &url("http://localhost:1421/agent-skills"),
            &dev
        ));
        assert!(!super::is_agent_skills_origin(
            &url("http://127.0.0.1:1420/agent-skills"),
            &dev
        ));
        // A different route inside the same origin is refused.
        assert!(!super::is_agent_skills_origin(&url("http://localhost:1420/"), &dev));
        assert!(!super::is_agent_skills_origin(
            &url("http://localhost:1420/workflow"),
            &dev
        ));
        // A query string could carry a redirect target, so a query is refused.
        assert!(!super::is_agent_skills_origin(
            &url("http://localhost:1420/agent-skills?redirect=https://evil.test"),
            &dev
        ));
        // Credentials embedded in the URL are refused even on the right host and port.
        assert!(!super::is_agent_skills_origin(
            &url("http://evil@localhost:1420/agent-skills"),
            &dev
        ));
        assert!(!super::is_agent_skills_origin(
            &url("http://user:pass@localhost:1420/agent-skills"),
            &dev
        ));
    }

    /// The production origin is the platform's custom protocol; only that exact scheme, host and
    /// port is the bundled surface.
    #[test]
    fn the_production_origin_accepts_only_the_bundled_scheme() {
        let linux = origin("tauri", "localhost", None);
        assert!(super::is_agent_skills_origin(
            &url("tauri://localhost/agent-skills"),
            &linux
        ));
        // A port on the custom scheme is not the bundled surface.
        assert!(!super::is_agent_skills_origin(
            &url("tauri://localhost:1234/agent-skills"),
            &linux
        ));
        // A remote host with the same scheme is still refused.
        assert!(!super::is_agent_skills_origin(
            &url("tauri://evil/agent-skills"),
            &linux
        ));
        // The Windows protocol is a different origin, so it is refused here.
        assert!(!super::is_agent_skills_origin(
            &url("http://tauri.localhost/agent-skills"),
            &linux
        ));

        let windows = origin("http", "tauri.localhost", Some(80));
        assert!(super::is_agent_skills_origin(
            &url("http://tauri.localhost/agent-skills"),
            &windows
        ));
        assert!(!super::is_agent_skills_origin(
            &url("https://tauri.localhost/agent-skills"),
            &windows
        ));
    }

    /// The expected origin is read exactly, so an omitted default port is the same origin as the
    /// explicit one and the production origin is the platform's custom protocol.
    #[test]
    fn the_expected_origin_is_read_exactly() {
        assert_eq!(
            super::AppOrigin::from_url(&url("http://localhost:80/agent-skills")),
            origin("http", "localhost", Some(80))
        );
        assert_eq!(
            super::AppOrigin::from_url(&url("http://localhost/agent-skills")),
            origin("http", "localhost", Some(80))
        );
        assert_eq!(
            super::AppOrigin::production(),
            if cfg!(windows) {
                origin("http", "tauri.localhost", Some(80))
            } else {
                origin("tauri", "localhost", None)
            }
        );
    }

    /// A URL the surface is judged against, so a bad literal fails loudly in the test.
    fn url(raw: &str) -> super::Url {
        raw.parse().expect("url")
    }

    /// One origin built directly, so a test can exercise both platform forms on any platform.
    fn origin(scheme: &str, host: &str, port: Option<u16>) -> super::AppOrigin {
        super::AppOrigin {
            scheme: scheme.to_string(),
            host: host.to_string(),
            port,
        }
    }

    /// Guard for the carrier: the surface is a same-window child webview, never a second window,
    /// and it is never given the freedom to navigate to remote content or to open one.
    #[test]
    fn the_surface_is_a_same_window_child_webview() {
        let source = production_source();

        assert!(source.contains("tauri::webview::NewWindowResponse::Deny"));
        assert!(source.contains("WebviewUrl::App(AGENT_SKILLS_ROUTE.into())"));
        assert!(source.contains(".on_navigation(move |url| is_agent_skills_origin(url, &expected))"));
        // The navigation rule is built from the application's own origin, so a bare scheme or an
        // arbitrary loopback port can never be accepted.
        assert!(source.contains("let expected = expected_origin(app);"));
        assert!(!source.contains(r#"url.scheme() == "tauri""#));
        assert!(!source.contains(r#"Some("127.0.0.1")"#));
        // The window builder would create a separate top-level window and must never appear.
        assert!(!source.contains(concat!("Webview", "WindowBuilder")));
        assert!(!source.contains("create_agent_skills_window"));
    }
}