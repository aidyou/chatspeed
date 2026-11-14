//! Resolve optional Network proxy APIs without raising the macOS deployment target.
//!
//! Wry's `mac-proxy` feature links these symbols unconditionally. Keeping the lookup
//! here lets one binary use explicit proxies on newer macOS and system networking on
//! older releases, including when it is built with an older SDK.

use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

use objc2::{rc::Retained, sel};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol};
use objc2_web_kit::WKWebViewConfiguration;
use wry::ProxyConfig;

type CreateEndpoint = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut NSObject;
type CreateHttpProxy = unsafe extern "C" fn(*mut NSObject, *mut NSObject) -> *mut NSObject;
type CreateSocksProxy = unsafe extern "C" fn(*mut NSObject) -> *mut NSObject;

struct NetworkApi {
    create_endpoint: CreateEndpoint,
    create_http_proxy: CreateHttpProxy,
    create_socks_proxy: CreateSocksProxy,
}

impl NetworkApi {
    fn load() -> Option<Self> {
        // SAFETY: the absolute system framework path is NUL terminated. The handle
        // stays open for the process lifetime while cached function pointers are used.
        let handle = unsafe {
            libc::dlopen(
                c"/System/Library/Frameworks/Network.framework/Network".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_LOCAL,
            )
        };
        if handle.is_null() {
            return None;
        }
        // SAFETY: the handle belongs to the loaded system framework and all names
        // are NUL terminated. No pointer is called before every lookup succeeds.
        let symbols = unsafe {
            [
                libc::dlsym(handle, c"nw_endpoint_create_host".as_ptr()),
                libc::dlsym(handle, c"nw_proxy_config_create_http_connect".as_ptr()),
                libc::dlsym(handle, c"nw_proxy_config_create_socksv5".as_ptr()),
            ]
        };
        // SAFETY: non-null symbols came from Network.framework with its public
        // C ABI. A successful load retains the framework for the process lifetime.
        let api = unsafe { Self::from_symbols(symbols) };
        if api.is_none() {
            // SAFETY: no function pointer escapes the failed lookup.
            unsafe { libc::dlclose(handle) };
        }
        api
    }

    /// All symbols must have the Network.framework C signatures and remain loaded.
    unsafe fn from_symbols(symbols: [*mut c_void; 3]) -> Option<Self> {
        if symbols.iter().any(|symbol| symbol.is_null()) {
            return None;
        }
        // SAFETY: callers guarantee these addresses have the declared signatures.
        Some(unsafe {
            Self {
                create_endpoint: std::mem::transmute::<*mut c_void, CreateEndpoint>(symbols[0]),
                create_http_proxy: std::mem::transmute::<*mut c_void, CreateHttpProxy>(symbols[1]),
                create_socks_proxy: std::mem::transmute::<*mut c_void, CreateSocksProxy>(
                    symbols[2],
                ),
            }
        })
    }
}

fn network_api() -> Option<&'static NetworkApi> {
    static API: OnceLock<Option<NetworkApi>> = OnceLock::new();
    API.get_or_init(NetworkApi::load).as_ref()
}

pub(super) fn is_available() -> bool {
    network_api().is_some()
}

/// Configure the data store before either Wry or Tauri starts the first request.
pub(super) fn configuration(proxy: &ProxyConfig) -> Option<Retained<WKWebViewConfiguration>> {
    let api = network_api()?;
    let mtm = MainThreadMarker::new()?;
    let endpoint = match proxy {
        ProxyConfig::Http(endpoint) | ProxyConfig::Socks5(endpoint) => endpoint,
    };
    let host = CString::new(endpoint.host.as_str()).ok()?;
    let port = CString::new(endpoint.port.as_str()).ok()?;
    // SAFETY: configuration construction and data-store access occur on the main
    // thread. Its default store preserves Wry's existing browsing profile.
    let (config, data_store) = unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let data_store = config.websiteDataStore();
        (config, data_store)
    };
    if !data_store.respondsToSelector(sel!(setProxyConfigurations:)) {
        log::warn!("WebKit has no explicit proxy support; keeping system networking");
        return None;
    }

    // SAFETY: dynamically resolved functions have their verified C signatures;
    // strings live through the call and Create-rule objects transfer +1 ownership.
    let proxy = unsafe {
        let endpoint = Retained::from_raw((api.create_endpoint)(host.as_ptr(), port.as_ptr()))?;
        let endpoint_ptr = Retained::as_ptr(&endpoint).cast_mut();
        Retained::from_raw(match proxy {
            ProxyConfig::Http(_) => (api.create_http_proxy)(endpoint_ptr, std::ptr::null_mut()),
            ProxyConfig::Socks5(_) => (api.create_socks_proxy)(endpoint_ptr),
        })?
    };
    let proxies = NSArray::arrayWithObject(&*proxy);
    // SAFETY: availability was checked on this data store. Network proxy configs
    // are Objective-C objects, and the setter retains the array and its contents.
    unsafe {
        let _: () = objc2::msg_send![&*data_store, setProxyConfigurations: &*proxies];
    }
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn endpoint_stub(_: *const c_char, _: *const c_char) -> *mut NSObject {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn http_stub(_: *mut NSObject, _: *mut NSObject) -> *mut NSObject {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn socks_stub(_: *mut NSObject) -> *mut NSObject {
        std::ptr::null_mut()
    }

    #[test]
    fn unavailable_network_symbols_are_optional() {
        let symbols = [
            endpoint_stub as *mut c_void,
            http_stub as *mut c_void,
            socks_stub as *mut c_void,
        ];
        for index in 0..symbols.len() {
            let mut incomplete = symbols;
            incomplete[index] = std::ptr::null_mut();
            // SAFETY: non-null entries are test functions with the matching ABI.
            assert!(unsafe { NetworkApi::from_symbols(incomplete) }.is_none());
        }
        // SAFETY: every entry is a test function with the matching ABI.
        let api = unsafe { NetworkApi::from_symbols(symbols) }
            .expect("a complete symbol table enables explicit proxies");
        // SAFETY: the stubs accept null and do not dereference their arguments.
        unsafe {
            assert!((api.create_endpoint)(std::ptr::null(), std::ptr::null()).is_null());
            assert!((api.create_http_proxy)(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
            assert!((api.create_socks_proxy)(std::ptr::null_mut()).is_null());
        }
    }
}
