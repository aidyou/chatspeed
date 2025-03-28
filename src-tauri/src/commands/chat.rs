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
//! window.addEventListener('ai_chunk', (event) => {
//!   console.log('ai_chunk', event)
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
use crate::ai::interaction::chat_completion::{complete_chat_async, ChatProtocol, ChatState};
use crate::ai::traits::chat::{ChatResponse, MessageType};
use crate::commands::constants::{TIME_IND, URL_REGEX};
use crate::constants::CFG_CHP_SERVER;
use crate::db::MainStore;
use crate::http::chp::Chp;
use crate::libs::lang::{get_available_lang, lang_to_iso_639_1};

use chrono::Local;
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;
use whatlang::detect;

pub fn setup_chat_proxy(
    main_state: &Arc<std::sync::Mutex<MainStore>>,
    metadata: &mut Option<Value>,
) -> Result<(), String> {
    // If the proxy type is http, get the proxy server and username/password from the config
    if let Some(md) = metadata.as_mut() {
        // 从元数据中获取代理类型字符串
        let mut proxy_type = md
            .get("proxyType")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string();

        // 如果代理类型是"bySetting"，从配置中获取
        if proxy_type == "bySetting" {
            let config_store = main_state.lock().map_err(|e| e.to_string())?;
            proxy_type = config_store.get_config("proxy_type", "none".to_string());
            md["proxyType"] = json!(proxy_type);
        }
        if proxy_type == "http" {
            let config_store = main_state.lock().map_err(|e| e.to_string())?;
            let proxy_server = config_store.get_config("proxy_server", String::new());
            if !proxy_server.is_empty() {
                md["proxyServer"] = json!(proxy_server);
                md["proxyUsername"] =
                    json!(config_store.get_config("proxy_username", String::new()));
                md["proxyPassword"] =
                    json!(config_store.get_config("proxy_password", String::new()));
            }
        }
    }
    Ok(())
}

pub fn setup_chat_context(
    main_state: &Arc<std::sync::Mutex<MainStore>>,
    messages: &mut Vec<Value>,
    metadata: &mut Option<Value>,
) {
    let mut use_context = true;
    if let Some(md) = metadata.as_mut() {
        // set to false if the frontend passed useContext = false
        if let Some(uc) = md.get("useContext") {
            use_context = uc.as_bool().unwrap_or(false);
        }
    }

    // Add context to messages
    if use_context {
        let mut context = vec![];

        let content = messages
            .last()
            .and_then(|m| m["content"].as_str())
            .map(|s| s.to_string());

        if let Some(content) = content {
            if TIME_IND.iter().any(|&keyword| content.contains(keyword)) {
                context.push(t!(
                    "chat.context.time",
                    time = Local::now().format("%Y-%m-%d %H:%M:%ST%Z").to_string()
                ));
            }
        }

        // 只在第一轮对话提供上下文信息
        // Just add context to first message
        if messages.len() == 1 {
            if let Ok(config_store) = main_state.lock() {
                let location = config_store.get_config("location", String::new());
                if !location.is_empty() {
                    context.push(t!("chat.context.location", location = location));
                }
                let role = config_store.get_config("role", String::new());
                if !role.is_empty() {
                    context.push(t!("chat.context.role", role = role));
                }
                let language = config_store.get_config("primary_language", String::new());
                if !language.is_empty() {
                    // get available languages
                    if let Ok(available_languages) = get_available_lang() {
                        if available_languages.contains_key(&language) {
                            context.push(t!(
                                "chat.context.language",
                                language = available_languages[&language]
                            ));
                        }
                    }
                }
            }
        }

        // Update messages with context
        if !context.is_empty() {
            if let Some(last_message) = messages.last_mut() {
                let new_content = t!(
                    "chat.context.content",
                    context = context.join(" "),
                    content = last_message["content"].as_str().unwrap_or_default()
                )
                .to_string();
                last_message["content"] = Value::String(new_content);
            }
            #[cfg(debug_assertions)]
            log::debug!("Context added: {}", context.join(" "));
        }
    }
}

/// Tauri command to interact with the AI chat system.
/// This command handles sending messages to the AI and receiving responses.
///
/// # Arguments
/// - `window` - The Tauri window instance, automatically injected by Tauri
/// - `state` - The state of the chat system, automatically injected by Tauri
/// - `main_state` - The main application state, automatically injected by Tauri
/// - `api_protocol` - The API provider to use for the chat.
/// - `api_url` - The API URL to use for the chat.
/// - `model` - The model to use for the chat.
/// - `api_key` - The API key to use for the chat.
/// - `chat_id` - Unique identifier for this chat session
/// - `messages` - The messages to send to the chat.
/// - `network_enabled` - Whether to enable network search for URLs in the user message
/// - `_deep_search_enabled` - Whether to enable deep search (currently unused)
/// - `metadata` - Optional extra parameters for the chat.
///
/// # Returns
/// A `Result` containing the response string or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('chat_completion', {
///     apiProtocol: 'openai',
///     apiUrl: 'https://api.openai.com/v1/chat/completions',
///     model: 'gpt-3.5-turbo',
///     apiKey: 'your-api-key',
///     chatId: 'unique-chat-id',
///     messages: [
///         {role: 'system', content: 'your prompt here'},
///         { role: 'user', content: 'Hello, how are you?' }
///     ],
///     maxTokens: 100
/// })
/// window.addEventListener('ai_chunk', (event) => {
///   console.log('ai_chunk', event)
/// })
/// ```
#[tauri::command]
pub async fn chat_completion(
    window: tauri::Window, // The Tauri window instance, automatically injected by Tauri
    chat_state: State<'_, Arc<ChatState>>, // The state of the chat system, automatically injected by Tauri
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: String,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    mut messages: Vec<Value>,
    network_enabled: Option<bool>,
    mut metadata: Option<Value>,
) -> Result<(), String> {
    let tx = chat_state
        .channels
        .get_or_create_channel(window.clone())
        .await?;

    let protocol: ChatProtocol = api_protocol.try_into().map_err(|e: String| e.to_string())?;

    setup_chat_proxy(&main_state, &mut metadata)?;

    // 如果网络搜索已启用，匹配出最后一条用户提问的信息中的 url
    // 将 url 的内容抓取出来拼接到最后一条用户消息中
    if network_enabled == Some(true) {
        // 获取爬虫接口地址
        let crawler_url = main_state
            .lock()
            .map_err(|e| e.to_string())?
            .get_config(CFG_CHP_SERVER, String::new());

        // 获取最后一条用户消息的内容
        let content = messages
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .map(|s| s.to_string());

        // 如果找到内容，则进行抓取
        if let Some(content) = content {
            // find url in content
            let urls: Vec<String> = URL_REGEX
                .find_iter(&content)
                .map(|m| m.as_str().to_string())
                .collect();

            // if the content has no url, do search
            if urls.is_empty() {
                return chat_completion_with_search(
                    window, chat_state, main_state, protocol, api_url, model, api_key, chat_id,
                    messages, metadata,
                )
                .await;
            }

            match crawler_from_content(&crawler_url, urls).await {
                Ok(crawled_contents) => {
                    if !crawled_contents.is_empty() {
                        if let Some(last_message) = messages.last_mut() {
                            let new_content = t!(
                                "chat.user_message_with_references",
                                content = content,
                                crawl_content = &crawled_contents
                            )
                            .to_string();
                            dbg!(&new_content);
                            last_message["content"] = Value::String(new_content);
                        }
                    }
                    let _ = &tx
                        .try_send(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            crawled_contents,
                            MessageType::Reference,
                            metadata.clone(),
                        ))
                        .map_err(|e| e.to_string())?;
                }
                Err(err) => {
                    log::error!("Failed to crawl content: {}", err);
                }
            }
        }
    }

    setup_chat_context(&main_state, &mut messages, &mut metadata);

    let callback = move |chunk: Arc<ChatResponse>| {
        let _ = tx.try_send(chunk);
    };

    // Execute the chat with the processed messages
    complete_chat_async(
        &chat_state,
        protocol,
        api_url,
        model,
        api_key,
        chat_id,
        messages,
        metadata,
        callback,
    )
    .await?;

    Ok(())
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

    serde_json::to_string(&crawled_contents).map_err(|e| e.to_string())
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
) -> Result<(), String> {
    let protocol: ChatProtocol = api_protocol
        .clone()
        .try_into()
        .map_err(|e: String| e.to_string())?;
    // Lock the chat state to access the chat interfaces.
    let chats = state.chats.lock().await;
    let chat_interface = chats.get(&protocol);

    match chat_interface {
        Some(chat_enum) => {
            chat_enum.set_stop_flag(true).await; // Set the stop flag to true.
            Ok(()) // Return success.
        }
        None => Err(t!("chat.invalid_api_provider", provider = api_protocol).to_string()), // Handle invalid API provider case.
    }
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
        let languages = get_available_lang().map_err(|e| e.to_string())?;
        let lang_code = lang_to_iso_639_1(&info.lang().code()).map_err(|e| e.to_string())?;

        if let Some(lang_name) = languages.get(lang_code) {
            Ok(json!({ "lang": lang_name.to_string(), "code": lang_code }))
        } else {
            Ok(json!({ "lang": info.lang().name().to_string(), "code": lang_code }))
        }
    } else {
        Err(t!("chat.failed_to_detect_language").to_string())
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
