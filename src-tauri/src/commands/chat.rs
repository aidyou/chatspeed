//! # AI Commands
//!
//! This module provides Tauri commands for interacting with AI chat interfaces.
//! It allows sending messages to different AI providers, receiving responses,
//! and stopping ongoing chats.
//!
//! ## Usage
//! To use the AI commands, you need to call the `chat_completion` and `stop_chat` functions.
//!
//! ### Example
//!
//! #### Chat with AI
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//!
//! const result = await invoke('chat_completion', {
//!     apiProtocol: 'openai',
//!     apiUrl: 'https://api.openai.com/v1/chat/completions',
//!     model: 'gpt-3.5-turbo',
//!     apiKey: 'your-api-key',
//!     messages: [
//!         { role: 'system', content: 'your prompt here' },
//!         { role: 'user', content: 'Hello, how are you?' }
//!     ],
//!     maxTokens: 100
//! })
//! window.addEventListener('chat_stream', (event) => {
//!   console.log('chat_stream', event)
//! })
//! ```
//!
//! #### Stop Chat
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//!
//! const result = await invoke('stop_chat', { apiProtocol: 'openai' })
//! ```
//!
//! ## How to add new AI providers
//!
//! - To add a new AI provider, you need to implement the `AiChatTrait` and `Stoppable` traits for the provider.
//!   You can use the existing providers as examples.
//! - Add new methods to the `AiChatEnum` enum.
//! - Then add new states to the `ChatState`::new method.
//! - Finally, add the new AI provider to the `init_chats!` macro.

use super::chat_web_search::chat_completion_with_search;
use crate::ai::interaction::chat_completion::{
    list_models_async, start_new_chat_interaction, ChatProtocol, ChatState,
};
use crate::ai::traits::chat::{ChatResponse, MCPToolDeclaration, MessageType, ModelDetails};
use crate::commands::constants::URL_REGEX;
use crate::constants::{CFG_CHP_SERVER, CFG_SEARCH_ENGINE};
use crate::db::MainStore;
use crate::http::chp::{Chp, SearchProvider};
use crate::libs::lang::{get_available_lang, lang_to_iso_639_1};
use crate::workflow::tools::{DeepSearch, ModelName};
use crate::workflow::ToolManager;

use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::State;
use whatlang::detect;

pub fn setup_chat_proxy(
    main_state: &Arc<std::sync::Mutex<MainStore>>,
    metadata: &mut Option<Value>,
) -> Result<(), String> {
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
            let ps = md.get("proxyServer").and_then(Value::as_str).unwrap_or("");
            if ps.starts_with("http://") || ps.starts_with("https://") {
                return Ok(());
            }
        }

        // 如果代理类型是"bySetting"，从配置中获取
        // if proxy_type is "bySetting", get proxy type from config
        if proxy_type == "bySetting" {
            let config_store = main_state.lock().map_err(|e| {
                t!("db.failed_to_lock_main_store", error = e.to_string()).to_string()
            })?;
            proxy_type = config_store.get_config("proxy_type", "none".to_string());
            if let Some(md_obj) = md.as_object_mut() {
                md_obj.insert("proxyType".to_string(), json!(proxy_type));
            }
        }
        if proxy_type == "http" {
            let config_store = main_state.lock().map_err(|e| {
                t!("db.failed_to_lock_main_store", error = e.to_string()).to_string()
            })?;
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

#[tauri::command]
pub async fn list_models(
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: String,
    api_url: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<ModelDetails>, String> {
    let mut metadata = Some(json!({"proxyType":"bySetting"}));
    setup_chat_proxy(&main_state, &mut metadata)?;

    list_models_async(api_protocol, api_url, api_key, metadata).await
}

/// Tauri command to interact with the AI chat system.
/// This command handles sending messages to the AI and receiving responses.
///
/// # Arguments
/// - `window` - The Tauri window instance, automatically injected by Tauri
/// - `chat_state` - The state of the chat system, automatically injected by Tauri
/// - `main_state` - The main application state, automatically injected by Tauri
/// - `api_protocol` - The API provider to use for the chat.
/// - `api_url` - The API URL to use for the chat.
/// - `model` - The model to use for the chat.
/// - `api_key` - The API key to use for the chat.
/// - `chat_id` - Unique identifier for this chat session
/// - `messages` - The messages to send to the chat.
/// - `network_enabled` - Whether to enable network search for URLs in the user message
/// - `metadata` - Optional extra parameters for the chat.
///
/// # Returns
/// A `Result` containing () or an error message.
#[tauri::command]
pub async fn chat_completion(
    window: tauri::Window,
    chat_state: State<'_, Arc<ChatState>>,
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: String,
    api_url: Option<String>, // Changed to Option<String> to match JS invoke
    model: String,
    api_key: Option<String>, // Changed to Option<String>
    chat_id: String,
    mut messages: Vec<Value>,
    network_enabled: Option<bool>,
    mut metadata: Option<Value>, // This comes from frontend, contains model params & UI flags
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    log::debug!("Starting chat completion for protocol: {}", api_protocol);

    if messages.is_empty() {
        return Err(t!("chat.empty_messages").to_string());
    }
    if model.is_empty() {
        return Err(t!("chat.empty_model").to_string());
    }
    if chat_id.is_empty() {
        return Err(t!("chat.empty_chat_id").to_string());
    }

    let protocol: ChatProtocol = api_protocol.try_into().map_err(|e: String| e.to_string())?;

    // Ensure the channel for this window is created/retrieved.
    // This is crucial for global_message_processor_loop to send UI updates.
    let _window_sender = chat_state
        .channels
        .get_or_create_channel(window.clone())
        .await
        .map_err(|e| format!("Failed to get or create window channel: {}", e))?;

    setup_chat_proxy(&main_state, &mut metadata)
        .map_err(|e| t!("chat.failed_to_setup_proxy", error = e).to_string())?;

    // 如果网络搜索已启用，匹配出最后一条用户提问的信息中的 url
    // 将 url 的内容抓取出来拼接到最后一条用户消息中
    if network_enabled == Some(true) {
        // 获取爬虫接口地址
        let crawler_url = main_state
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
            .get_config(CFG_CHP_SERVER, String::new());

        let content_to_search = messages
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .map(|s| s.to_string());

        if let Some(content) = content_to_search {
            let urls: Vec<String> = URL_REGEX
                .find_iter(&content)
                .map(|m| m.as_str().to_string())
                .collect();

            if urls.is_empty() {
                // If no URLs, proceed to chat_completion_with_search
                // Note: chat_completion_with_search might need similar adjustments
                // to call start_new_chat_interaction eventually.
                // For now, assuming it handles its own flow or will be refactored.
                return chat_completion_with_search(
                    window,
                    chat_state,
                    main_state,
                    protocol,
                    api_url.as_deref(),
                    model,
                    api_key.as_deref(),
                    chat_id,
                    messages,
                    metadata,
                )
                .await;
            }

            match crawler_from_content(&crawler_url, urls).await {
                Ok(crawled_contents) => {
                    if !crawled_contents.is_empty() {
                        if let Some(last_message) = messages.last_mut() {
                            if last_message.get("role").and_then(Value::as_str) == Some("user") {
                                let new_content = t!(
                                    "chat.user_message_with_references",
                                    content = content,
                                    crawl_content = &crawled_contents
                                )
                                .to_string();
                                if let Some(obj) = last_message.as_object_mut() {
                                    obj.insert("content".to_string(), Value::String(new_content));
                                }
                            }
                        }
                    }
                    if let Some(sender) = chat_state.channels.get_sender(&window.label()).await {
                        let _ = sender.try_send(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            crawled_contents,
                            MessageType::Reference,
                            metadata.clone(),
                            None,
                        ));
                    }
                }
                Err(err) => {
                    log::error!("Failed to crawl content: {}", err);
                }
            }
        }
    }

    // Prepare final_metadata: ensure windowLabel is present.
    // Frontend JS sends metadata with `windowLabel`.
    let final_metadata = if let Some(mut md_val) = metadata {
        if let Some(md_obj) = md_val.as_object_mut() {
            // Ensure "windowLabel" (from JS) or "label" (fallback) or window.label() is present
            if !md_obj.contains_key("windowLabel") && !md_obj.contains_key("label") {
                md_obj.insert("windowLabel".to_string(), json!(window.label()));
            }
        }
        Some(md_val)
    } else {
        Some(json!({"windowLabel": window.label()}))
    };

    let tools_enabled_in_metadata = final_metadata
        .as_ref()
        .and_then(|md_val| md_val.get("toolsEnabled").and_then(Value::as_bool))
        .unwrap_or(false);

    let tools: Option<Vec<MCPToolDeclaration>> = if tools_enabled_in_metadata {
        Some(
            chat_state
                .tool_manager
                .get_tool_calling_spec(None)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    #[cfg(debug_assertions)]
    log::debug!("Processed messages count: {}", messages.len());

    start_new_chat_interaction(
        chat_state.inner().clone(),
        protocol,
        api_url.as_deref(),
        model,
        api_key.as_deref(),
        chat_id,
        messages,
        tools,
        final_metadata,
    )
    .await
}

/// Use the crawler_url server interface to fetch information about the URLs contained in the content.
///
/// # Arguments
/// * `crawler_url` - The URL of the crawler server.
/// * `content` - The content to be searched.
///
/// # Returns
///
/// A JSON string containing information about the URLs found in the content.
pub async fn crawler_from_content(crawler_url: &str, urls: Vec<String>) -> Result<String, String> {
    if urls.is_empty() {
        return Ok("".to_string());
    }

    // 抓取每个 URL 的内容
    let mut crawled_contents = Vec::new();
    let crawler = Chp::new(crawler_url.to_string(), None);
    let mut i = 1;
    for url in urls {
        match crawler.web_crawler(&url, None).await {
            Ok(data) => {
                if !data.content.is_empty() {
                    crawled_contents.push(json!({
                        "id": i,
                        "url": url,
                        "title": data.title,
                        "content": data.content
                    }));
                    i += 1;
                }
            }
            Err(err) => {
                log::error!("Failed to crawl URL {}: {}", url, err);
            }
        }
    }
    if crawled_contents.is_empty() {
        return Ok("".to_string());
    }

    serde_json::to_string(&crawled_contents).map_err(|e| {
        t!(
            "chat.json_serialize_crawler_results_failed",
            error = e.to_string()
        )
        .to_string()
    })
}

/// Tauri command to stop the ongoing chat for a specific API provider.
/// This command sets the stop flag for the selected chat interface.
///
/// # Arguments
/// - `state` - The state of the chat system, automatically injected by Tauri
/// - `api_protocol` - The API provider for which to stop the chat.
///
/// # Returns
/// A `Result` indicating success or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('stop_chat', { apiProtocol: 'openai' })
/// ```
#[tauri::command]
pub async fn stop_chat(
    state: State<'_, Arc<ChatState>>,
    api_protocol: String,
    chat_id: &str,
) -> Result<(), String> {
    let protocol: ChatProtocol = api_protocol
        .clone()
        .try_into()
        .map_err(|e: String| format!("Invalid API protocol: {}", e))?;
    let mut chats = state.chats.lock().await;

    // Find the specific chat instance
    if let Some(protocol_chats) = chats.get_mut(&protocol) {
        if let Some(chat) = protocol_chats.get_mut(chat_id) {
            // Use get_mut to avoid removing
            chat.set_stop_flag(true).await;
            // Remove the chat instance from the map
            protocol_chats.remove(chat_id);

            #[cfg(debug_assertions)]
            {
                log::debug!(
                    "Removed chat instance for chat_id: {} on stop command under protocol: {}.",
                    chat_id,
                    protocol
                );
            }

            return Ok(());
        }
    }
    Err(t!("chat.chat_not_found").to_string())
}

/// Detects the language of a given text and returns the corresponding language code.
///
/// # Arguments
/// - `text` - The text to detect the language of.
///
/// # Returns
/// A `Result` containing the language code or an error message.
#[tauri::command]
pub fn detect_language(text: &str) -> Result<Value, String> {
    let detected_lang = detect(text);

    if let Some(info) = detected_lang {
        let languages = get_available_lang()
            .map_err(|e| t!("chat.failed_to_get_available_languages", error = e).to_string())?;
        let lang_code = lang_to_iso_639_1(&info.lang().code()).map_err(|e| {
            t!(
                "chat.failed_to_convert_language_code",
                error = e.to_string()
            )
            .to_string()
        })?;

        if let Some(lang_name) = languages.get(lang_code) {
            Ok(json!({ "lang": lang_name.to_string(), "code": lang_code }))
        } else {
            Ok(json!({ "lang": info.lang().name().to_string(), "code": lang_code }))
        }
    } else {
        Err(t!("chat.failed_to_detect_language").to_string())
    }
}

/// Tauri command to perform a deep search for a given question.
/// This command uses the configured search engine providers to search for the question on the web.
///
/// # Arguments
/// - `window` - The Tauri window instance, automatically injected by Tauri
/// - `chat_state` - The state of the chat system, automatically injected by Tauri
/// - `main_state` - The main application state, automatically injected by Tauri
/// - `chat_id` - Unique identifier for this chat session
/// - `question` - The question to search for.
/// - `metadata` - Optional extra parameters for the chat.
///
/// # Returns
/// A `Result` indicating success or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('deep_search', {
///     chatId: 'unique-chat-id',
///     question: 'What is the best programming language?'
///     metadata: {
///         "label": "main"
///     }
/// })
/// ```
#[tauri::command]
pub async fn deep_search(
    window: tauri::Window,
    chat_state: State<'_, Arc<ChatState>>,
    main_store: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    chat_id: &str,
    question: &str,
    metadata: Option<Value>,
) -> Result<(), String> {
    // Get crawler URL, it use to fetch web page contents
    let crawler_url = main_store
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
        .get_config(CFG_CHP_SERVER, String::new());

    // Get search engine providers from config
    // e.g. ["google", "bing", "duckduckgo"]
    let search_providers = main_store
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
        .get_config(CFG_SEARCH_ENGINE, vec![])
        .into_iter()
        .filter_map(|s: String| s.trim().parse::<SearchProvider>().ok())
        .collect::<Vec<SearchProvider>>();

    let tx = chat_state.channels.get_or_create_channel(window).await?;
    let tx_clone = tx.clone();
    let callback = move |chunk: Arc<ChatResponse>| {
        let _ = tx_clone.try_send(chunk);
    };

    let ds = DeepSearch::new(
        chat_state.inner().clone(),
        None,
        crawler_url,
        search_providers,
        Arc::new(callback),
    );
    let stop_flag = ds.get_stop_flag();
    chat_state
        .active_searches
        .lock()
        .await
        .insert(chat_id.to_string(), stop_flag);

    // Get the model configured for the deep search
    let reasoning_model = ToolManager::get_model(main_store.inner(), ModelName::Reasoning.as_ref())
        .map_err(|e| {
            t!(
                "workflow.failed_to_get_model",
                model_type = ModelName::Reasoning.as_ref(),
                error = e.to_string()
            )
            .to_string()
        })?;
    ds.add_model(ModelName::Reasoning, reasoning_model).await;
    let general_model = ToolManager::get_model(main_store.inner(), ModelName::General.as_ref())
        .map_err(|e| {
            t!(
                "workflow.failed_to_get_model",
                model_type = ModelName::General.as_ref(),
                error = e.to_string()
            )
            .to_string()
        })?;
    ds.add_model(ModelName::General, general_model).await;

    let _ = ds
        .execute_deep_search(chat_id, question, metadata, 20)
        .await?;

    Ok(())
}

/// Tauri command to stop the ongoing deep search for a specific chat session.
/// This command sets the stop flag for the selected chat interface.
///
/// # Arguments
/// - `state` - The state of the chat system, automatically injected by Tauri
/// - `chat_id` - Unique identifier for this chat session
///
/// # Returns
/// A `Result` indicating success or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('stop_deep_search', { chatId: 'unique-chat-id' })
/// ```
#[tauri::command]
pub async fn stop_deep_search(
    state: State<'_, Arc<ChatState>>,
    chat_id: &str,
) -> Result<(), String> {
    let mut active_searches = state.active_searches.lock().await;
    if let Some(flag) = active_searches.get(chat_id) {
        flag.store(true, Ordering::Release);
        active_searches.remove(chat_id);
        Ok(())
    } else {
        Err(t!("chat.search_not_found").to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::chat::URL_REGEX;

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
