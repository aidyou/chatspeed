//! Proxy the webviews of this application use.
//!
//! That two different webviews need the same rule is what makes this a shared module
//! instead of a helper of one feature: the embedded ChatHub page is a plain `wry`
//! webview, while the scraper opens a Tauri web view window, and both have to leave
//! through the proxy the general network settings configure. The models, the search
//! tools and the updater read those settings on their own.
//!
//! Two rules shape what is forwarded:
//!
//! - Only an explicit `http` proxy is applied. `none` lets a webview go out directly,
//!   and `system` is left to the webview, which already follows the system proxy.
//! - macOS only accepts one from macOS 14, where the WebKit key it needs exists. On an
//!   older release the proxy is skipped instead of reaching a key that is not there.
//!
//! Neither webview library accepts a proxy after the webview exists, so the settings
//! are read while it is built.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Wry};
use url::Url;
use wry::{ProxyConfig, ProxyEndpoint};

use crate::db::MainStore;

/// First macOS release whose webviews accept an explicit proxy.
#[cfg(target_os = "macos")]
const MACOS_PROXY_MINIMUM_VERSION: u32 = 14;

/// How a webview reaches the proxy server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyScheme {
    Http,
    Socks5,
}

/// Proxy a webview has to use, as the general network settings describe it.
#[derive(Debug, Clone)]
pub struct WebviewProxy {
    scheme: ProxyScheme,
    host: String,
    port: u16,
}

impl WebviewProxy {
    /// Reads the proxy a webview has to use, when the settings ask for one and this
    /// system can apply it.
    pub fn current(app: &AppHandle<Wry>) -> Option<Self> {
        if !Self::is_supported() {
            log::debug!(
                "This system has no webview proxy support, the configured proxy is not applied"
            );
            return None;
        }

        let store = app.try_state::<Arc<MainStore>>()?;
        let proxy_type = store.get_config("proxy_type", String::new());
        let server = store.get_config("proxy_server", String::new());
        let proxy = Self::from_settings(&proxy_type, &server)?;

        // A webview proxy carries a host and a port only, so a proxy that asks for
        // credentials cannot be used; saying so keeps a failing request explainable.
        let username = store.get_config("proxy_username", String::new());
        if !username.trim().is_empty() {
            log::warn!(
                "The configured proxy needs credentials a webview cannot send, only its host and port are used"
            );
        }

        Some(proxy)
    }

    /// Turns a configured proxy type and server into a proxy a webview understands.
    ///
    /// A server no webview can use yields nothing instead of a page that cannot load
    /// anything at all.
    pub fn from_settings(proxy_type: &str, server: &str) -> Option<Self> {
        if proxy_type != "http" {
            return None;
        }

        let server = server.trim();
        if server.is_empty() {
            return None;
        }

        let Ok(url) = Url::parse(server) else {
            log::warn!(
                "Ignoring the configured proxy '{}': it is not a URL",
                server
            );
            return None;
        };

        let Some(host) = url.host_str().filter(|host| !host.is_empty()) else {
            log::warn!("Ignoring the configured proxy '{}': it has no host", server);
            return None;
        };

        // `host_str` reports an IPv6 host inside brackets, which the endpoint of a
        // webview proxy does not take; the brackets are added back where a URL needs them.
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);

        let (scheme, default_port) = match url.scheme() {
            "http" => (ProxyScheme::Http, 80),
            // An `https` proxy is reached through the same CONNECT tunnel.
            "https" => (ProxyScheme::Http, 443),
            "socks" | "socks5" => (ProxyScheme::Socks5, 1080),
            other => {
                log::warn!(
                    "Ignoring the configured proxy '{}': '{}' is not supported",
                    server,
                    other
                );
                return None;
            }
        };

        Some(Self {
            scheme,
            host: host.to_string(),
            port: url.port().unwrap_or(default_port),
        })
    }

    /// The proxy in the form `wry` takes while a webview is built.
    pub fn wry_config(&self) -> ProxyConfig {
        let endpoint = ProxyEndpoint {
            host: self.host.clone(),
            port: self.port.to_string(),
        };

        match self.scheme {
            ProxyScheme::Http => ProxyConfig::Http(endpoint),
            ProxyScheme::Socks5 => ProxyConfig::Socks5(endpoint),
        }
    }

    /// The proxy in the form Tauri takes for one of its web view windows.
    ///
    /// `None` when the proxy cannot be written as a URL, which leaves the window
    /// without a proxy instead of keeping it from opening.
    pub fn proxy_url(&self) -> Option<Url> {
        let scheme = match self.scheme {
            ProxyScheme::Http => "http",
            ProxyScheme::Socks5 => "socks5",
        };

        // An IPv6 host reaches the URL without its brackets, which it needs back.
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };

        match Url::parse(&format!("{}://{}:{}", scheme, host, self.port)) {
            Ok(url) => Some(url),
            Err(error) => {
                log::warn!(
                    "Failed to build the proxy URL for '{}': {}",
                    self.host,
                    error
                );
                None
            }
        }
    }

    /// Whether the webviews of this platform can be given an explicit proxy.
    fn is_supported() -> bool {
        #[cfg(target_os = "macos")]
        {
            macos_major_version().is_some_and(|major| major >= MACOS_PROXY_MINIMUM_VERSION)
        }

        #[cfg(not(target_os = "macos"))]
        {
            true
        }
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
        log::warn!("Failed to read the macOS version, the configured proxy is not applied");
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
        log::warn!("Failed to read the macOS version, the configured proxy is not applied");
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

    /// Guard for the network settings boundary: only a proxy a webview can actually use
    /// is forwarded, so a setting that cannot work leaves the webview direct instead of
    /// making it load nothing at all.
    #[test]
    fn only_a_usable_proxy_setting_reaches_a_webview() {
        let http = WebviewProxy::from_settings("http", "http://127.0.0.1:7890")
            .expect("a plain HTTP proxy has to be accepted");
        assert_eq!(http.scheme, ProxyScheme::Http);
        assert_eq!(http.host, "127.0.0.1");
        assert_eq!(http.port, 7890);

        // Both webview libraries take the same endpoint, so both forms are checked.
        match http.wry_config() {
            ProxyConfig::Http(endpoint) => {
                assert_eq!(endpoint.host, "127.0.0.1");
                assert_eq!(endpoint.port, "7890");
            }
            other => panic!("expected an HTTP proxy for wry, got {other:?}"),
        }

        let url = http.proxy_url().expect("the proxy has to become a URL");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(7890));

        // wry supports SOCKSv5 as well, so such a server is forwarded too.
        let socks = WebviewProxy::from_settings("http", "socks5://127.0.0.1:1080")
            .expect("a SOCKSv5 proxy has to be accepted");
        assert_eq!(socks.scheme, ProxyScheme::Socks5);
        assert!(matches!(socks.wry_config(), ProxyConfig::Socks5(_)));

        // A server without a port falls back to the default one of its scheme.
        let default_port = WebviewProxy::from_settings("http", "http://proxy.local")
            .expect("a server without a port has to be accepted");
        assert_eq!(default_port.port, 80);

        // Everything else leaves the webview without a proxy of its own: the two other
        // types, an empty or unusable server, and a scheme no webview can use.
        assert!(WebviewProxy::from_settings("none", "http://127.0.0.1:7890").is_none());
        assert!(WebviewProxy::from_settings("system", "http://127.0.0.1:7890").is_none());
        assert!(WebviewProxy::from_settings("http", "").is_none());
        assert!(WebviewProxy::from_settings("http", "   ").is_none());
        assert!(WebviewProxy::from_settings("http", "127.0.0.1:7890").is_none());
        assert!(WebviewProxy::from_settings("http", "ftp://127.0.0.1:21").is_none());
    }

    /// An IPv6 proxy host reaches the URL without its brackets, so they have to be put
    /// back for the Tauri window to accept it.
    #[test]
    fn an_ipv6_proxy_host_stays_a_valid_url() {
        let proxy = WebviewProxy::from_settings("http", "http://[::1]:7890")
            .expect("an IPv6 proxy has to be accepted");

        assert_eq!(proxy.host, "::1");
        let url = proxy.proxy_url().expect("the proxy has to become a URL");
        // `host_str` reports an IPv6 host inside brackets again.
        assert_eq!(url.host_str(), Some("[::1]"));
        assert_eq!(url.port(), Some(7890));
    }

    /// The version gate decides whether a proxy is applied at all, so reading it has to
    /// work on macOS: a version that cannot be read would silently disable the proxy.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_version_is_readable() {
        assert!(macos_major_version().is_some_and(|major| major >= 11));
    }
}
