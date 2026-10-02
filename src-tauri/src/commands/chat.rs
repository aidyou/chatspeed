//! # AI Commands
//!
//! Tauri adapters for the runtime-owned chat and model surface.
//!
//! The desktop no longer owns chat execution. `list_models`, `chat_completion`
//! and `stop_chat` are thin adapters over the standalone runtime's typed
//! `/control/v1` chat/model routes: the runtime holds the chat/tool state and the
//! provider proxy, and its sessions resolve every model call against that owner.
//! The command wrappers therefore never inject `MainStore`, `ChatState` or a
//! `ToolManager`, and they never open the runtime database.
//!
//! `chat_completion` preserves its Tauri-only `window` boundary: the runtime's
//! SSE chunks are forwarded onto the same `chat_stream` window event the frontend
//! already listens on. See `runtime_chat.rs` for the adapter.
//!
//! `detect_language` is a pure, desktop-local utility and stays in the client.
//! `setup_chat_proxy` also stays: it is a plain helper the desktop ccproxy
//! modules import, not a second chat owner.
//!
//! ## Usage
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//!
//! // The turn runs on the standalone runtime; its chunks arrive on the
//! // window's `chat_stream` event exactly as before.
//! await invoke('chat_completion', {
//!     providerId: 1,
//!     model: 'gpt-3.5-turbo',
//!     chatId: 'chat-1',
//!     messages: [{ role: 'user', content: 'Hello' }]
//! })
//! ```
//!
//! ## Module wiring
//!
//! `runtime_chat` is included from here with an explicit `#[path]` so this unit
//! does not have to edit `lib.rs` (several sibling units touch the module list
//! there). When that concurrency is done the parent may hoist the declaration to
//! `lib.rs` as `#[cfg(feature = "desktop")] mod runtime_chat;`.

use crate::ai::traits::chat::ModelDetails;
use crate::db::MainStore;
use crate::error::AppError;
use crate::libs::lang::{get_available_lang, lang_to_iso_639_1};
use crate::runtime_client::RuntimeSupervisor;
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;
use whatlang::detect;

#[path = "../runtime_chat.rs"]
mod runtime_chat;

/// Fills a chat request's `metadata` with the proxy configuration the store
/// holds, so model calls honour the user's proxy settings.
///
/// This is a plain helper argument, not a Tauri state injection: the desktop
/// `ccproxy` modules import it at its historical
/// `crate::commands::chat::setup_chat_proxy` path. The desktop-free runtime
/// builds the same helper from `crate::ccproxy::proxy_settings` (which is gated
/// to the non-desktop build), so the two copies cannot diverge.
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

/// Lists the models a provider exposes.
///
/// The read needs the runtime's proxy configuration and network owner, so it is
/// served by the runtime owner over `POST /control/v1/models/list`. The declared
/// wire is preserved so the frontend keeps calling the same command.
#[tauri::command]
pub async fn list_models(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    api_protocol: String,
    api_url: Option<&str>,
    api_key: Option<&str>,
    metadata: Option<Value>,
) -> Result<Vec<ModelDetails>, String> {
    runtime_chat::list_models(
        supervisor.inner().as_ref(),
        api_protocol,
        api_url,
        api_key,
        metadata,
    )
    .await
}

/// Starts one AI chat turn and streams the reply to the requesting window.
///
/// The background model execution belongs to the runtime; this command only asks
/// the runtime to start the turn and then forwards its stream onto the window's
/// `chat_stream` event.
#[tauri::command]
pub async fn chat_completion(
    window: tauri::Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    provider_id: i64,
    model: String,
    chat_id: String,
    messages: Vec<Value>,
    network_enabled: Option<bool>,
    mcp_enabled: Option<bool>,
    metadata: Option<Value>,
) -> Result<(), String> {
    runtime_chat::chat_completion(
        window,
        supervisor.inner().as_ref(),
        provider_id,
        model,
        chat_id,
        messages,
        network_enabled,
        mcp_enabled,
        metadata,
    )
    .await
}

/// Stops the ongoing chat for one provider/chat id.
///
/// Stopping mutates the runtime-owned chat state, so it is served by the runtime
/// over `POST /control/v1/chats/{chat_id}/stop`.
#[tauri::command]
pub async fn stop_chat(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    api_protocol: String,
    chat_id: &str,
) -> Result<(), String> {
    runtime_chat::stop_chat(supervisor.inner().as_ref(), api_protocol, chat_id).await
}

/// Detects the language of a given text and returns the corresponding language code.
///
/// # Arguments
/// - `text` - The text to detect the language of.
///
/// # Returns
/// A `Result` containing the language code or an error message.
#[tauri::command]
pub fn detect_language(text: &str) -> crate::error::Result<Value> {
    let detected_lang = detect(text);

    if let Some(info) = detected_lang {
        let languages = get_available_lang().map_err(|e| AppError::General {
            message: t!("chat.failed_to_get_available_languages", error = e).to_string(),
        })?;
        let lang_code = lang_to_iso_639_1(&info.lang().code()).map_err(|e| AppError::General {
            message: t!(
                "chat.failed_to_convert_language_code",
                error = e.to_string()
            )
            .to_string(),
        })?;

        if let Some(lang_name) = languages.get(lang_code) {
            Ok(json!({ "lang": lang_name.to_string(), "code": lang_code }))
        } else {
            Ok(json!({ "lang": info.lang().name().to_string(), "code": lang_code }))
        }
    } else {
        Err(AppError::General {
            message: t!("chat.failed_to_detect_language").to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::constants::URL_REGEX;

    #[test]
    fn test_url_regex() {
        // 测试有效 URL
        assert!(URL_REGEX.is_match("https://www.example.com"));
        assert!(URL_REGEX.is_match("http://example.com/path/to/resource"));
        assert!(URL_REGEX.is_match("https://sub.domain.co.uk/page.html"));
        assert!(URL_REGEX.is_match("http://localhost:8080/")); // localhost
        assert!(URL_REGEX.is_match("http://127.0.0.1:8080/")); // IP 地址
        assert!(URL_REGEX.is_match("http://192.168.1.1/")); // IP 地址

        // 测试带参数的 URL
        assert!(URL_REGEX.is_match("https://example.com?query=string"));
        assert!(URL_REGEX.is_match("http://example.com/path?param=value"));

        // 测试 URL 后面有空格或标点
        let text = "访问 https://example.com 查看详情";
        let matches: Vec<_> = URL_REGEX.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(matches, vec!["https://example.com"]);

        // 测试 URL 在句子结尾
        let text = "我的网站是https://example.com/q=a,b,c+d's+f，请查收";
        let matches: Vec<_> = URL_REGEX.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(matches, vec!["https://example.com/q=a,b,c+d's+f"]);

        // 测试无效 URL
        assert!(!URL_REGEX.is_match("ftp://example.com")); // 不支持的协议
        assert!(!URL_REGEX.is_match("https://")); // 缺少域名
        assert!(!URL_REGEX.is_match("https://example")); // 无效顶级域名
        assert!(!URL_REGEX.is_match("https://中文.com")); // 包含非ASCII字符

        // 测试包含非 ASCII 字符的路径
        let text = "https://example.com/路径";
        let matches: Vec<_> = URL_REGEX.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(matches, vec!["https://example.com/"]); // 应该匹配到 https://example.com/

        // 测试 URL 后面紧跟非空白字符
        let text = "访问https://example.com查看详情";
        let matches: Vec<_> = URL_REGEX.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(matches, vec!["https://example.com"]);

        let text = "帮我看下五粮液的最新消息： https://www.google.com/search?num=10&newwindow=1&sca_esv=d71ecd6c338007cf&q=%E4%BA%94%E7%B2%AE%E6%B6%B2&tbm=nws&source=lnms&fbs=ABzOT_AGBMogrnfXHu6GxeqSvos9XSASLdCNmBvs6Xj8xORx7DdQ5Qf-hUGrUlZE47p3nt_wRsvqT5kI5zzGpsTUFL1NtHWXBuWBXA_8FX0YOa2iQL8pUZj731v_jueJcMs4Skhde7wdO_KaQJ7zTQYQ-3mpAkgqEy6NfQtvSUgwlMp4znN99vkXSsWAcywgev8Dk-ZbucaF&sa=X&ved=2ahUKEwim2puh-N6LAxUPrlYBHUxgI04Q0pQJegQIHxAB&biw=1084&bih=1057&dpr=2.2";
        let matches: Vec<_> = URL_REGEX.find_iter(text).map(|m| m.as_str()).collect();
        assert_eq!(matches, vec!["https://www.google.com/search?num=10&newwindow=1&sca_esv=d71ecd6c338007cf&q=%E4%BA%94%E7%B2%AE%E6%B6%B2&tbm=nws&source=lnms&fbs=ABzOT_AGBMogrnfXHu6GxeqSvos9XSASLdCNmBvs6Xj8xORx7DdQ5Qf-hUGrUlZE47p3nt_wRsvqT5kI5zzGpsTUFL1NtHWXBuWBXA_8FX0YOa2iQL8pUZj731v_jueJcMs4Skhde7wdO_KaQJ7zTQYQ-3mpAkgqEy6NfQtvSUgwlMp4znN99vkXSsWAcywgev8Dk-ZbucaF&sa=X&ved=2ahUKEwim2puh-N6LAxUPrlYBHUxgI04Q0pQJegQIHxAB&biw=1084&bih=1057&dpr=2.2"]);
    }
}
