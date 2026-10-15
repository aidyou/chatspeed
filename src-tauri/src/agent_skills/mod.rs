//! Native carrier of the trusted Agent Skills surface.
//!
//! Agent Skills is the built-in `agent-skills` management surface. Unlike an ordinary plugin
//! panel it is a trusted *application page* (`/agent-skills`) and not a plugin asset bundle, so
//! it is rendered by a Tauri webview, which is the only carrier that receives the desktop IPC the
//! surface needs. It is still docked into the right dock of the Workflow window exactly like a
//! ChatHub tab or a plugin panel, so the user keeps one dock column with a tab strip instead of a
//! second top-level window.
//!
//! The surface is a same-window child webview ([`tauri::Window::add_child`]):
//!
//! - Windows and macOS place the child view at the rectangle the frontend measured.
//! - Linux builds the child webview into the window's default `GtkBox`, which ignores a rectangle,
//!   so the inner `webkit2gtk::WebView` is moved out of that box into a bounded holder of the
//!   shared dock surface ([`crate::native_dock`]) through [`tauri::Webview::with_webview`].
//!
//! Only the built-in `agent-skills` bundle that the runtime reported as enabled with a verified UI
//! may reach this carrier; the command that shows it re-proves that in the runtime inventory while
//! holding the plugin UI gate.
//!
//! An ordinary plugin panel keeps its own isolation: it is built with plain `wry`, so it still
//! receives no Tauri IPC at all. This module is the only docked carrier that uses a Tauri webview,
//! and it is reachable only for the trusted surface.

mod carrier;
mod view;

pub use view::{run_on_view_thread, AgentSkillsView};