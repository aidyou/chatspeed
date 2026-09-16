//! Proxy the docked ChatHub page goes out through.

use tauri::{AppHandle, Wry};
use wry::ProxyConfig;

use crate::libs::webview_proxy::WebviewProxy;

/// Proxy the embedded page has to use, taken from the network settings.
///
/// The rules themselves are shared with the other webviews of the application (see
/// [`WebviewProxy`]). What belongs here is the lifetime of this page: it is created once
/// and reused for every entry, so the settings are read while it is built and a proxy
/// configured later reaches the page only after it was closed and opened again.
pub fn page_proxy(app: &AppHandle<Wry>) -> Option<ProxyConfig> {
    WebviewProxy::current(app).map(|proxy| proxy.wry_config())
}
