//! Proxy the docked ChatHub page goes out through.
//!
//! The page is a plain webview, so it does not follow the proxy the models, the search
//! tools and the updater use: the general network settings are read here and handed to
//! wry while the page is built.
//!
//! Two rules shape what is forwarded:
//!
//! - Only an explicit `http` proxy is applied. `none` lets the page go out directly,
//!   and `system` is left to the webview, which already follows the system proxy.
//! - macOS only accepts one from macOS 14, where the proxy setting of
//!   `WKWebsiteDataStore` exists. On an older release the proxy is skipped, because
//!   handing wry one would reach a WebKit key that is not there.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Wry};
use url::Url;
use wry::{ProxyConfig, ProxyEndpoint};

use crate::db::MainStore;

/// First macOS release whose webview accepts an explicit proxy.
#[cfg(target_os = "macos")]
const MACOS_PROXY_MINIMUM_VERSION: u32 = 14;

/// Proxy the embedded page has to use, taken from the network settings.
///
/// The page is created once and reused, so the settings are read while it is built: a
/// proxy configured later only reaches the page after it was closed and opened again.
/// Nothing is applied when no proxy is configured, and on a system whose webview cannot
/// be given one the page keeps using the system proxy instead.
pub fn page_proxy(app: &AppHandle<Wry>) -> Option<ProxyConfig> {
    if !page_proxy_is_supported() {
        log::debug!(
            "[ChatHub] this system has no webview proxy support, the configured proxy is not applied"
        );
        return None;
    }

    let store = app.try_state::<Arc<MainStore>>()?;
    let proxy_type = store.get_config("proxy_type", String::new());
    let server = store.get_config("proxy_server", String::new());
    let proxy = proxy_from_settings(&proxy_type, &server)?;

    // wry only carries a host and a port, so a proxy that asks for credentials cannot be
    // used by the embedded page; saying so keeps a failing request explainable.
    let username = store.get_config("proxy_username", String::new());
    if !username.trim().is_empty() {
        log::warn!(
            "[ChatHub] the configured proxy needs credentials the embedded page cannot send, only its host and port are used"
        );
    }

    Some(proxy)
}

/// Turns the configured proxy type and server into the proxy a webview understands.
///
/// A server the webview cannot use yields nothing instead of a page that cannot load
/// anything at all.
fn proxy_from_settings(proxy_type: &str, server: &str) -> Option<ProxyConfig> {
    if proxy_type != "http" {
        return None;
    }

    let server = server.trim();
    if server.is_empty() {
        return None;
    }

    let Ok(url) = Url::parse(server) else {
        log::warn!(
            "[ChatHub] ignoring the configured proxy '{}': it is not a URL",
            server
        );
        return None;
    };

    let Some(host) = url.host_str().filter(|host| !host.is_empty()) else {
        log::warn!(
            "[ChatHub] ignoring the configured proxy '{}': it has no host",
            server
        );
        return None;
    };

    let endpoint = |port: u16| ProxyEndpoint {
        host: host.to_string(),
        port: port.to_string(),
    };

    match url.scheme() {
        "http" => Some(ProxyConfig::Http(endpoint(url.port().unwrap_or(80)))),
        // An `https` proxy is reached through the same CONNECT tunnel, so the webview
        // takes it as the same kind of endpoint.
        "https" => Some(ProxyConfig::Http(endpoint(url.port().unwrap_or(443)))),
        "socks" | "socks5" => Some(ProxyConfig::Socks5(endpoint(url.port().unwrap_or(1080)))),
        scheme => {
            log::warn!(
                "[ChatHub] ignoring the configured proxy '{}': '{}' is not supported",
                server,
                scheme
            );
            None
        }
    }
}

/// Whether the webview of this platform can be given an explicit proxy.
fn page_proxy_is_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_major_version().is_some_and(|major| major >= MACOS_PROXY_MINIMUM_VERSION)
    }

    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Major version of the macOS release the application runs on.
#[cfg(target_os = "macos")]
fn macos_major_version() -> Option<u32> {
    // `kern.osproductversion` holds the release, for example `14.5`.
    let name = b"kern.osproductversion\0";
    let mut size = 0usize;

    // SAFETY: the name is a valid NUL terminated sysctl name, and a null buffer asks the
    // kernel for the size of the value only.
    let sized = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if sized != 0 || size == 0 {
        log::warn!(
            "[ChatHub] failed to read the macOS version, the configured proxy is not applied"
        );
        return None;
    }

    let mut buffer = vec![0u8; size];
    // SAFETY: the buffer holds exactly the size the kernel reported for this value.
    let read = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        log::warn!(
            "[ChatHub] failed to read the macOS version, the configured proxy is not applied"
        );
        return None;
    }

    // The value is a NUL terminated string such as `14.5`.
    String::from_utf8_lossy(&buffer)
        .trim_matches('\0')
        .split('.')
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard for the network settings boundary: only a proxy the webview can actually
    /// use is forwarded, so a setting that cannot work leaves the page direct instead of
    /// making it load nothing at all.
    #[test]
    fn only_a_usable_proxy_setting_reaches_the_webview() {
        match proxy_from_settings("http", "http://127.0.0.1:7890") {
            Some(ProxyConfig::Http(endpoint)) => {
                assert_eq!(endpoint.host, "127.0.0.1");
                assert_eq!(endpoint.port, "7890");
            }
            other => panic!("expected an HTTP proxy, got {other:?}"),
        }

        // wry supports SOCKSv5 as well, so such a server is forwarded too.
        match proxy_from_settings("http", "socks5://127.0.0.1:1080") {
            Some(ProxyConfig::Socks5(endpoint)) => assert_eq!(endpoint.host, "127.0.0.1"),
            other => panic!("expected a SOCKSv5 proxy, got {other:?}"),
        }

        // A server without a port falls back to the default one of its scheme.
        match proxy_from_settings("http", "http://proxy.local") {
            Some(ProxyConfig::Http(endpoint)) => assert_eq!(endpoint.port, "80"),
            other => panic!("expected an HTTP proxy, got {other:?}"),
        }

        // Everything else leaves the page without a proxy of its own: the two other
        // types, an empty or unusable server, and a scheme wry cannot use.
        assert!(proxy_from_settings("none", "http://127.0.0.1:7890").is_none());
        assert!(proxy_from_settings("system", "http://127.0.0.1:7890").is_none());
        assert!(proxy_from_settings("http", "").is_none());
        assert!(proxy_from_settings("http", "   ").is_none());
        assert!(proxy_from_settings("http", "127.0.0.1:7890").is_none());
        assert!(proxy_from_settings("http", "ftp://127.0.0.1:21").is_none());
    }

    /// The version gate decides whether a proxy is applied at all, so reading it has to
    /// work on macOS: a version that cannot be read would silently disable the proxy.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_version_is_readable() {
        assert!(macos_major_version().is_some_and(|major| major >= 11));
    }
}
