//! Proxy resolution for a chat request's metadata.
//!
//! The function below is the same helper the desktop command module exposes as
//! `crate::commands::chat::setup_chat_proxy`. It is Tauri-free, but it lives in
//! the desktop-only command file, so the desktop-free runtime builds it here
//! from `src/ccproxy/` — the module that actually consumes it — instead of
//! copying it into the runtime crate. Consolidating the two copies by turning
//! the command module's version into a re-export is a follow-up that touches a
//! file outside the runtime migration scope.

use crate::db::MainStore;
use serde_json::{json, Value};
use std::sync::Arc;

/// Fills a chat request's `metadata` with the proxy configuration the store
/// holds, so model calls honour the user's proxy settings.
///
/// A model that already carries its own proxy server is left untouched.
pub fn setup_chat_proxy(
    main_state: Arc<MainStore>,
    metadata: &mut Option<Value>,
) -> crate::error::Result<()> {
    // If the proxy type is http, get the proxy server and username/password from the config
    if let Some(md) = metadata.as_mut() {
        // metadata is Value::Object
        // 从元数据中获取代理类型字符串
        let mut proxy_type = md
            .get("proxyType")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string();

        // 如果模型本身已经设置了代理服务器(proxyServer)，则直接返回即可
        // if proxy_type is "http" and proxyServer is set, return directly
        if proxy_type == "http" {
            let has_proxy_servers = md
                .get("proxyServers")
                .and_then(Value::as_array)
                .is_some_and(|servers| !servers.is_empty());
            let ps = md.get("proxyServer").and_then(Value::as_str).unwrap_or("");
            if has_proxy_servers || ps.starts_with("http://") || ps.starts_with("https://") {
                return Ok(());
            }
        }

        // If proxy type is "bySetting", get it from config
        // if proxy_type is "bySetting", get proxy type from config
        if proxy_type == "bySetting" {
            let config_store = &*main_state;
            proxy_type = config_store.get_config("proxy_type", "none".to_string());
            if let Some(md_obj) = md.as_object_mut() {
                md_obj.insert("proxyType".to_string(), json!(proxy_type));
            }
        }
        if proxy_type == "http" {
            let config_store = &*main_state;
            let proxy_server = config_store.get_config("proxy_server", String::new());
            if !proxy_server.is_empty() {
                if let Some(obj) = md.as_object_mut() {
                    obj.insert("proxyServer".to_string(), json!(proxy_server));
                    obj.insert(
                        "proxyUsername".to_string(),
                        json!(config_store.get_config("proxy_username", String::new())),
                    );
                    obj.insert(
                        "proxyPassword".to_string(),
                        json!(config_store.get_config("proxy_password", String::new())),
                    );
                }
            }
        }
    }
    Ok(())
}
