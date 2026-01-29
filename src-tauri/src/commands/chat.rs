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

use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{
    list_models_async, start_new_chat_interaction, ChatState,
};
use crate::ai::interaction::constants::{SYSTEM_PROMPT, TOOL_USAGE_GUIDANCE};
use crate::ai::traits::chat::{MCPToolDeclaration, ModelDetails};
use crate::ccproxy::ChatProtocol;
use crate::constants::{
    CFG_INTERFACE_LANGUAGE, DEFAULT_WEB_FETCH_TOOL, DEFAULT_WEB_SEARCH_TOOL,
};
use crate::db::MainStore;
use crate::error::{AppError, Result};
use crate::libs::lang::{get_available_lang, lang_to_iso_639_1};
use crate::sensitive::manager::{FilterManager, SensitiveConfig};
use crate::tools::MCP_TOOL_NAME_SPLIT;

use chrono::{DateTime, Local};
use rust_i18n::t;
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use tauri::State;
use whatlang::detect;

/// Generates environment information for the AI assistant
fn generate_environment_info() -> String {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let family = env::consts::FAMILY;

    // Get current time in both UTC and local timezone
    // let utc_now: DateTime<Utc> = Utc::now();
    let local_now: DateTime<Local> = Local::now();

    // Get shell information
    let shell = env::var("SHELL")
        .or_else(|_| env::var("COMSPEC"))
        .unwrap_or_else(|_| "unknown".to_string());

    // Get additional useful environment variables
    // let user = env::var("USER")
    //     .or_else(|_| env::var("USERNAME"))
    //     .unwrap_or_else(|_| "unknown".to_string());

    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| "unknown".to_string());

    format!(
        r#"<environment>
Operating System: {} ({})
Architecture: {}
Current Local Time: {}
Shell: {}
Home Directory: {}
</environment>"#,
        os,
        family,
        arch,
        local_now.format("%Y-%m-%d %H:%M:%S %Z"),
        shell,
        home
    )
}

/// Prepares messages with system prompts and environment information
fn prepare_messages_with_system_context(mut messages: Vec<Value>, has_tools: bool) -> Vec<Value> {
    let mut system_content = SYSTEM_PROMPT.to_string();

    // Add tool usage guidance if tools are available
    if has_tools {
        system_content.push_str(TOOL_USAGE_GUIDANCE);
    }

    // Find existing system message from user
    let mut user_system_content = String::new();
    let mut user_system_index = None;

    for (i, message) in messages.iter().enumerate() {
        if message.get("role").and_then(|r| r.as_str()) == Some("system") {
            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                user_system_content = content.to_string();
                user_system_index = Some(i);
                break;
            }
        }
    }

    // Remove user's system message if found
    if let Some(index) = user_system_index {
        messages.remove(index);
    }

    // Combine system prompts: our system prompt + tool guidance + user's system prompt
    if !user_system_content.is_empty() {
        system_content.push_str("\n\n");
        system_content.push_str(&user_system_content);
    }

    // Add environment information
    system_content.push_str("\n\n");
    system_content.push_str(&generate_environment_info());

    // Create the final system message
    let system_message = json!({
        "role": "system",
        "content": system_content
    });

    // Insert at the beginning
    messages.insert(0, system_message);

    messages
}

/// Helper function to filter a single piece of text for sensitive information
fn filter_single_text(
    text: &str,
    filter_manager: &FilterManager,
    sensitive_config: &SensitiveConfig,
    interface_lang: &String,
) -> String {
    let lang_info = detect(text);
    let detected_code = if let Some(ref info) = lang_info {
        lang_to_iso_639_1(&info.lang().code()).unwrap_or("en")
    } else {
        "en"
    };

    let langs = vec![detected_code, interface_lang.as_str()];

    #[cfg(debug_assertions)]
    log::debug!(
        "Detect language: {:?} -> {}. Interface lang: {}",
        lang_info,
        detected_code,
        interface_lang
    );

    let sanitized = filter_manager.filter_text(text, &langs, sensitive_config);

    #[cfg(debug_assertions)]
    if text != sanitized {
        log::debug!(
            "Filtered content. Original len: {}, Sanitized len: {}",
            text.len(),
            sanitized.len()
        );
    } else {
        log::debug!("No sensitive data found in message.");
    }

    sanitized
}

pub fn setup_chat_proxy(
    main_state: Arc<std::sync::RwLock<MainStore>>,
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
            let ps = md.get("proxyServer").and_then(Value::as_str).unwrap_or("");
            if ps.starts_with("http://") || ps.starts_with("https://") {
                return Ok(());
            }
        }

        // If proxy type is "bySetting", get it from config
        // if proxy_type is "bySetting", get proxy type from config
        if proxy_type == "bySetting" {
            let config_store = main_state.read()?;
            proxy_type = config_store.get_config("proxy_type", "none".to_string());
            if let Some(md_obj) = md.as_object_mut() {
                md_obj.insert("proxyType".to_string(), json!(proxy_type));
            }
        }
        if proxy_type == "http" {
            let config_store = main_state.read()?;
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
    main_state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    api_protocol: String,
    api_url: Option<&str>,
    api_key: Option<&str>,
    metadata: Option<Value>,
) -> Result<Vec<ModelDetails>> {
    let mut metadata_val = metadata.unwrap_or_else(|| json!({}));
    if metadata_val["proxyType"].is_null() {
        metadata_val["proxyType"] = json!("bySetting");
    }
    let mut metadata = Some(metadata_val);

    setup_chat_proxy(main_state.inner().clone(), &mut metadata)?;

    list_models_async(
        main_state.inner().clone(),
        api_protocol,
        api_url,
        api_key,
        metadata,
    )
    .await
}

/// Tauri command to interact with the AI chat system.
/// This command handles sending messages to the AI and receiving responses.
///
/// # Arguments
/// - `window` - The Tauri window instance, automatically injected by Tauri
/// - `chat_state` - The state of the chat system, automatically injected by Tauri
/// - `main_state` - The main application state, automatically injected by Tauri
/// - `filter_manager` - The sensitive data filter manager, automatically injected by Tauri
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
    filter_manager: State<'_, FilterManager>,
    provider_id: i64,
    model: String,
    chat_id: String,
    messages: Vec<Value>,
    network_enabled: Option<bool>,
    mcp_enabled: Option<bool>,
    metadata: Option<Value>, // This comes from frontend, contains model params & UI flags
) -> Result<()> {
    if provider_id < 1 {
        return Err(AppError::Ai(AiError::InitFailed(
            t!("chat.empty_provider_id").to_string(),
        )));
    }
    if model.is_empty() {
        return Err(AppError::Ai(AiError::InitFailed(
            t!("chat.empty_model").to_string(),
        )));
    }
    if chat_id.is_empty() {
        return Err(AppError::Ai(AiError::InitFailed(
            t!("chat.empty_chat_id").to_string(),
        )));
    }
    if messages.is_empty() {
        return Err(AppError::Ai(AiError::InitFailed(
            t!("chat.empty_messages").to_string(),
        )));
    }

    // Ensure the channel for this window is created/retrieved.
    // This is crucial for global_message_processor_loop to send UI updates.
    //
    // !!! DO NOT remove the following line !!!
    let _ = chat_state
        .channels
        .get_or_create_channel(window.clone())
        .await
        .map_err(|e| AiError::FailedToGetOrCreateWindowChannel(e.to_string()))?;

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

    // Sensitive Data Filtering
    let (sensitive_config, interface_lang): (SensitiveConfig, String) = {
        let store = chat_state.main_store.read().map_err(|e| {
            AppError::Db(crate::db::StoreError::IoError(e.to_string()))
        })?;
        (
            store.get_config("sensitive_config", SensitiveConfig::default()),
            store.get_config(CFG_INTERFACE_LANGUAGE, "en".to_string()),
        )
    };

    let mut filtered_messages = messages;
    if sensitive_config.enabled {
        #[cfg(debug_assertions)]
        log::debug!(
            "Sensitive data filtering enabled. Config: {:?}",
            sensitive_config
        );

        for message in filtered_messages.iter_mut() {
            if message.get("role").and_then(|r| r.as_str()) == Some("user") {
                if let Some(content_val) = message.get_mut("content") {
                    if let Some(content_str) = content_val.as_str() {
                        if !content_str.is_empty() {
                            let sanitized = filter_single_text(content_str, &filter_manager, &sensitive_config, &interface_lang);
                            *content_val = json!(sanitized);
                        }
                    } else if let Some(content_array) = content_val.as_array_mut() {
                        for block in content_array {
                            if let Some(block_obj) = block.as_object_mut() {
                                if block_obj.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(text_val) = block_obj.get_mut("text") {
                                        if let Some(text_str) = text_val.as_str() {
                                            if !text_str.is_empty() {
                                                let sanitized = filter_single_text(text_str, &filter_manager, &sensitive_config, &interface_lang);
                                                *text_val = json!(sanitized);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        #[cfg(debug_assertions)]
        log::debug!("Sensitive data filtering disabled.");
    }

    let tools_enabled_in_metadata = final_metadata
        .as_ref()
        .and_then(|md_val| md_val.get("toolsEnabled").and_then(Value::as_bool))
        .unwrap_or(true);

    let tools: Option<Vec<MCPToolDeclaration>> = if tools_enabled_in_metadata {
        let mut available_tools = chat_state.tool_manager.get_tool_calling_spec(None).await?;
        if !network_enabled.unwrap_or(false) {
            available_tools.retain(|tool| {
                tool.name != DEFAULT_WEB_SEARCH_TOOL && tool.name != DEFAULT_WEB_FETCH_TOOL
            });
        }
        if !mcp_enabled.unwrap_or(false) {
            available_tools.retain(|tool| !tool.name.contains(MCP_TOOL_NAME_SPLIT));
        }
        Some(available_tools)
    } else {
        None
    };

    // Prepare messages with system context
    let has_tools = tools.as_ref().map_or(false, |t| !t.is_empty());
    let prepared_messages = prepare_messages_with_system_context(filtered_messages, has_tools);

    #[cfg(debug_assertions)]
    log::debug!("Processed messages count: {}", prepared_messages.len());

    start_new_chat_interaction(
        chat_state.inner().clone(),
        provider_id,
        model,
        chat_id,
        prepared_messages,
        tools,
        final_metadata,
        None,
    )
    .await
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
) -> Result<()> {
    let protocol: ChatProtocol = api_protocol.clone().try_into().map_err(|_| {
        AiError::InitFailed(
            t!("chat.invalid_api_protocol", protocll = api_protocol.clone()).to_string(),
        )
    })?;
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
    Err(AppError::Ai(AiError::InitFailed(
        t!("chat.chat_not_found").to_string(),
    )))
}

/// Detects the language of a given text and returns the corresponding language code.
///
/// # Arguments
/// - `text` - The text to detect the language of.
///
/// # Returns
/// A `Result` containing the language code or an error message.
#[tauri::command]
pub fn detect_language(text: &str) -> Result<Value> {
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
// #[tauri::command]
// pub async fn deep_search(
//     window: tauri::Window,
//     chat_state: State<'_, Arc<ChatState>>,
//     main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
//     chat_id: &str,
//     question: &str,
//     metadata: Option<Value>,
// ) -> Result<(), String> {
//     // Get crawler URL, it use to fetch web page contents
//     let crawler_url = main_store
//         .read()
//         .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
//         .get_config("chatspeedCrawlerUrl", String::new());

//     // Get search engine providers from config
//     // e.g. ["google", "bing", "duckduckgo"]
//     let search_providers = main_store
//         .read()
//         .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
//         .get_config(CFG_SEARCH_ENGINE, vec![])
//         .into_iter()
//         .filter_map(|s: String| s.trim().parse::<SearchProviderName>().ok())
//         .collect::<Vec<SearchProviderName>>();

//     let tx = chat_state.channels.get_or_create_channel(window).await?;
//     let tx_clone = tx.clone();
//     let callback = move |chunk: Arc<ChatResponse>| {
//         let _ = tx_clone.try_send(chunk);
//     };

//     let ds = DeepSearch::new(
//         chat_state.inner().clone(),
//         None,
//         crawler_url,
//         search_providers,
//         Arc::new(callback),
//     );
//     let stop_flag = ds.get_stop_flag();
//     chat_state
//         .active_searches
//         .lock()
//         .await
//         .insert(chat_id.to_string(), stop_flag);

//     // Get the model configured for the deep search
//     let reasoning_model =
//         ToolManager::get_model(main_store.inner().clone(), ModelName::Reasoning.as_ref()).map_err(
//             |e| {
//                 t!(
//                     "tools.failed_to_get_model",
//                     model_type = ModelName::Reasoning.as_ref(),
//                     error = e.to_string()
//                 )
//                 .to_string()
//             },
//         )?;
//     ds.add_model(ModelName::Reasoning, reasoning_model).await;
//     let general_model =
//         ToolManager::get_model(main_store.inner().clone(), ModelName::General.as_ref()).map_err(
//             |e| {
//                 t!(
//                     "tools.failed_to_get_model",
//                     model_type = ModelName::General.as_ref(),
//                     error = e.to_string()
//                 )
//                 .to_string()
//             },
//         )?;
//     ds.add_model(ModelName::General, general_model).await;

//     let _ = ds
//         .execute_deep_search(chat_id, question, metadata, 20)
//         .await?;

//     Ok(())
// }

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
// #[tauri::command]
// pub async fn stop_deep_search(
//     state: State<'_, Arc<ChatState>>,
//     chat_id: &str,
// ) -> Result<(), String> {
//     let mut active_searches = state.active_searches.lock().await;
//     if let Some(flag) = active_searches.get(chat_id) {
//         flag.store(true, Ordering::Release);
//         active_searches.remove(chat_id);
//         Ok(())
//     } else {
//         Err(t!("chat.search_not_found").to_string())
//     }
// }

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
