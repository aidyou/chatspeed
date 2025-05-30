use phf::phf_set;
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use super::chat::setup_chat_proxy;
use crate::ai::interaction::chat_completion::{
    complete_chat_async, complete_chat_blocking, ChatProtocol, ChatState,
};
use crate::ai::traits::chat::{ChatCompletionResult, MCPToolDeclaration, MessageType};
use crate::constants::{CFG_CHP_SERVER, CFG_SEARCH_ENGINE};
use crate::http::chp::{Chp, SearchProvider, SearchResult};
use crate::{ai::traits::chat::ChatResponse, db::MainStore};

const MAX_SEARCH_RESULT: i32 = 3;

// When the search results include video or image websites, they are filtered out
static VIDEO_AND_IMAGE_DOMAINS: phf::Set<&'static str> = phf_set! {
    // video websites
    "v.qq.com", "iqiyi.com", "youku.com", "imgo.tv", "bilibili.com", "xigua.com",
    "douyin.com", "kuaishou.com", "yspapp.cn", "youtube.com", "vimeo.com",
    "dailymotion.com", "netflix.com", "primevideo.com", "hulu.com", "disneyplus.com",
    "tiktok.com", "twitch.tv",
    // image websites
    "vcg.com", "dfic.cn", "tuchong.com", "zcool.com.cn",
    "ui.cn", "gettyimages.com", "shutterstock.com", "istockphoto.com", "stock.adobe.com",
    "alamy.com", "unsplash.com", "pexels.com", "pixabay.com", "instagram.com",
    "pinterest.com", "flickr.com", "dribbble.com", "behance.net",
};

pub async fn chat_completion_with_search(
    window: tauri::Window,
    chat_state: State<'_, Arc<ChatState>>,
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    mut messages: Vec<Value>,
    metadata: Option<Value>,
) -> Result<(), String> {
    let content = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if content.is_empty() {
        return Err(t!("chat.failed_to_find_user_message_content").to_string());
    }

    let tx = chat_state
        .channels
        .get_or_create_channel(window.clone())
        .await?;
    // User confirmed db.failed_to_lock_main_store
    let crawler_url = main_state
        .lock()
        .map_err(|e| e.to_string())?
        .get_config(CFG_CHP_SERVER, String::new());

    let search_engines = main_state
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?
        .get_config(CFG_SEARCH_ENGINE, vec![])
        .into_iter()
        .filter_map(|s: String| {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<String>>();

    let mut content_clone = content.clone();
    let chat_state_clone = chat_state.inner().clone();

    // 只在用户设置了搜索配置时才进行搜索
    // Only perform the search if the user has set up the search configuration
    if !crawler_url.is_empty() && !search_engines.is_empty() {
        // 设置搜索模型的配置
        let search_config = setup_model(
            &main_state,
            api_protocol.clone(),
            api_url,
            model.clone(),
            api_key,
            metadata.clone(),
        )?;

        let (protocol, url, key, metadata, model) = search_config;

        dbg!(&metadata);

        // 步骤1：生成搜索主题和关键词
        let search_queries = if content.len() > 150 || messages.len() > 1 {
            generate_search_queries(
                protocol.clone(),
                url.as_deref(),
                model.clone(),
                key.as_deref(),
                chat_id.clone(),
                &content,
                messages.iter().rev().take(3).rev().cloned().collect(),
                metadata.clone(),
            )
            .await?
        } else {
            vec![content.clone()]
        };

        send_step_message(
            &tx,
            chat_id.clone(),
            t!("chat.searching").to_string(),
            metadata.clone(),
        )
        .await?;

        // 步骤2：并发执行多引擎搜索
        let search_results =
            execute_multi_search(crawler_url.clone(), &search_engines, &search_queries).await?;

        send_step_message(
            &tx,
            chat_id.clone(),
            t!("chat.search_results", count = search_results.len()).to_string(),
            metadata.clone(),
        )
        .await?;

        if !search_results.is_empty() {
            // 步骤3：结果去重和排序
            send_step_message(
                &tx,
                chat_id.clone(),
                t!("chat.search_analysing").to_string(),
                metadata.clone(),
            )
            .await?;

            // 先过滤掉视频和图片网站，他们对文本对话没任何意义
            let filtered_results = search_results
                .iter()
                .filter(|result| {
                    !VIDEO_AND_IMAGE_DOMAINS
                        .iter()
                        .any(|domain| result.url.contains(domain))
                })
                .cloned()
                .collect::<Vec<_>>();

            // dedup and rank
            let mut ranked_results =
                crate::libs::dedup::dedup_and_rank_results(filtered_results, &content_clone);

            // 步骤4：获取最相关的搜索结果
            if let Ok(ai_ranked_results) = filter_and_rank_search_results(
                protocol,
                url.as_deref(),
                model,
                key.as_deref(),
                chat_id.clone(),
                &content,
                metadata.clone(),
                &ranked_results,
            )
            .await
            {
                dbg!(&ai_ranked_results);
                ranked_results = ai_ranked_results;
            }

            send_step_message(
                &tx,
                chat_id.clone(),
                t!("chat.search_fetching").to_string(),
                metadata.clone(),
            )
            .await?;

            // 步骤5：内容提取和分析
            let analysis_content = crawl_data(crawler_url, &ranked_results).await;

            if !analysis_content.is_empty() {
                // 先修改消息内容
                if let Some(last_message) = messages.last_mut() {
                    content_clone = t!(
                        "chat.user_message_with_references",
                        crawl_content = analysis_content,
                        content = content.clone()
                    )
                    .to_string();

                    if let Ok(content_value) = serde_json::from_str::<Value>(&content_clone) {
                        dbg!(&content_clone);
                        last_message["content"] = content_value;
                    } else {
                        last_message["content"] = Value::String(content_clone);
                    }
                }

                // 提取前 5 篇搜索结果
                let mut references = Vec::new();
                for (i, result) in ranked_results
                    .iter_mut()
                    .enumerate()
                    .take(MAX_SEARCH_RESULT as usize)
                {
                    result.id = i + 1;
                    references.push(result.clone());
                }

                // 将引用信息转换为JSON字符串
                match serde_json::to_string(&references) {
                    Ok(references_json) => {
                        // 发送引用信息到前端
                        if let Err(e) = tx.try_send(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            references_json,
                            MessageType::Reference,
                            metadata.clone(),
                            None,
                        )) {
                            log::warn!("Failed to send reference information: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to serialize reference information: {}", e);
                    }
                }
            }
        }
    }

    generate_final_answer(
        chat_state_clone.clone(),
        api_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        messages,
        metadata,
        tx,
    )
    .await?;

    Ok(())
}

/// Setup the model for search
///
/// # Arguments
/// * `main_state` - The main application state
/// * `api_protocol` - The API protocol to use
/// * `api_url` - The API URL to use
/// * `model` - The model to use
/// * `api_key` - The API key to use
/// * `metadata` - The metadata to use
///
/// # Returns
/// * `Result<(String, Option<String>, Option<String>, Option<Value>, String), String>` - The setup model result
fn setup_model(
    main_state: &State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    metadata: Option<Value>,
) -> Result<
    (
        ChatProtocol,
        Option<String>,
        Option<String>,
        Option<Value>,
        String,
    ),
    String,
> {
    // setup the model for search
    let mut search_api_protocol = api_protocol.clone();
    let mut search_api_url = api_url.map(|s| s.to_string());
    let mut search_api_key = api_key.map(|s| s.to_string());
    let mut search_metadata = metadata.clone();
    let mut search_api_model = model.clone();
    let mut change_proxy_type = false;

    // Get the search model configuration
    let search_model = main_state
        .lock()
        .map_err(|e| e.to_string())?
        .get_config("websearch_model", Value::Null);

    // If there is a search model configuration, use the model same as chat
    if !search_model.is_null() {
        // Get model ID
        let model_id = search_model["id"].as_i64().unwrap_or_default();

        let ai_model = main_state
            .lock()
            .map_err(|e| e.to_string())?
            .config
            .get_ai_model_by_id(model_id)
            .map_err(|e| format!("Failed to get AI model: {}", e));

        // Get model details from configuration
        if let Ok(smodel) = ai_model {
            // Update search API configuration
            search_api_protocol = smodel
                .api_protocol
                .try_into()
                .map_err(|e: String| e.to_string())?;
            search_api_url = Some(smodel.base_url.clone());
            search_api_key = Some(smodel.api_key.clone());

            // Only update proxyType if it is different from the model's proxyType
            let model_proxy_type = if let Some(md) = smodel.metadata {
                md.get("proxyType").map(|v| v.clone())
            } else {
                None
            };
            if let Some(md) = search_metadata.as_mut() {
                if let Some(obj) = md.as_object_mut() {
                    // Check if current metadata already has proxyType
                    let current_proxy_type = obj.get("proxyType").map(|v| v.clone());

                    // If current proxyType is missing or different from model proxyType, update
                    if current_proxy_type.is_none() || current_proxy_type != model_proxy_type {
                        obj.insert(
                            "proxyType".to_string(),
                            model_proxy_type.unwrap_or(Value::Null).clone(),
                        );
                        change_proxy_type = true;
                    }
                }
            }

            // Get model name from configuration
            let model_name = search_model["model"]
                .as_str()
                .unwrap_or_default()
                .to_string();

            // Validate model name
            if !smodel.models.iter().any(|m| m.id == model_name) {
                return Err(t!("chat.invalid_search_model", model_name = model_name).to_string());
            }

            // Update search model
            search_api_model = model_name;
        }
    }

    if change_proxy_type {
        setup_chat_proxy(&main_state, &mut search_metadata)?;
    }

    Ok((
        search_api_protocol,
        search_api_url,
        search_api_key,
        search_metadata,
        search_api_model,
    ))
}

/// Send a step message to the chat interface
///
/// # Arguments
/// * `tx` - The sender for the chat interface
/// * `chat_id` - The ID of the chat
/// * `msg` - The message to send
/// * `metadata` - Optional metadata to include with the message
///
/// # Returns
/// * `Result<(), String>` - The result of the operation
async fn send_step_message(
    tx: &tokio::sync::mpsc::Sender<Arc<ChatResponse>>,
    chat_id: String,
    msg: String,
    metadata: Option<Value>,
) -> Result<(), String> {
    let chunk = ChatResponse::new_with_arc(chat_id, msg, MessageType::Step, metadata, None);
    if let Err(e) = tx.try_send(chunk) {
        return Err(e.to_string());
    }
    Ok(())
}

/// Generate search queries for a given question
///
/// # Arguments
/// * `state` - The chat state
/// * `main_state` - The main application state
/// * `api_protocol` - The API protocol to use
/// * `api_url` - The API URL to use
/// * `model` - The model to use
/// * `api_key` - The API key to use
/// * `chat_id` - The ID of the chat
/// * `question` - The question to generate search queries for
/// * `metadata` - Optional metadata to include with the message
///
/// # Returns
/// * `Result<Vec<String>, String>` - The result of the operation
async fn generate_search_queries(
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    question: &str,
    mut messages: Vec<Value>,
    metadata: Option<Value>,
) -> Result<Vec<String>, String> {
    let prompt = format!(
        "[Rules]\n1. ​Language Detection \n   - Respond with keywords in the same language as the user's input  \n   - Handle mixed-language queries (e.g. 苹果发布会 → \"Apple event\")\n\n2. Core Extraction  \n   - Entities (nouns): [PRODUCT][LOCATION][EVENT]  \n   - Actions (verbs): [how-to][compare][find]  \n   - Modifiers: [CurrentYear][latest][free]\n\n3. Output Format  \n   - Generate 1 concise search query per query.  \n   - No additional explanation or text, only search queries.  \n   - Each query should be precise, with relevant and specific keywords.\n   - Remove any numbering or additional commentary in the output.\n\n[Examples]\nUser Input: \"Best budget 5G phones under $300\"\nYour Output: \"5G phones\" \"budget\" under $300\n\n[Response Template]\n[Current Time]{} \nGenerate search queries for:\n\n{}",
        chrono::Local::now().to_rfc3339(),
        question
    );
    if let Some(last_message) = messages.last_mut() {
        *last_message = json!({"role": "user", "content": prompt});
    } else {
        messages.push(json!({"role": "user", "content": prompt}));
    }

    let chat_reslut = chat_with_retry(
        chat_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        messages,
        None,
        metadata,
    )
    .await?;

    // Parse result into query list
    let queries: Vec<String> = chat_reslut
        .content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .take(3)
        .collect();

    Ok(queries)
}

async fn filter_and_rank_search_results(
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    question: &str,
    metadata: Option<Value>,
    search_result: &[SearchResult],
) -> Result<Vec<SearchResult>, String> {
    if search_result.is_empty() {
        return Ok(vec![]);
    }

    let prompt = format!(
        "[Task]\nExtract the top {} most relevant search results for the given topic, excluding video and image websites, and return them in JSON format.\n\n[Topic]\n{}\n\n[Search Results]\n{}",
        MAX_SEARCH_RESULT,
        question,
        serde_json::to_string(&search_result).unwrap_or("".to_string())
    );

    let chat_result = chat_with_retry(
        api_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        vec![json!({"role": "user", "content": prompt})],
        None,
        metadata,
    )
    .await?;

    // Parse result into query list
    let queries = crate::libs::util::format_json_str(&chat_result.content);

    let result: Vec<SearchResult> = serde_json::from_str(&queries).map_err(|e| e.to_string())?;

    Ok(result)
}

/// Chat with retry 3 times
///
/// # Arguments
/// * `state` - A reference to the chat state
/// * `api_protocol` - The API protocol to use
/// * `api_url` - The API URL to use
/// * `model` - The model to use
/// * `api_key` - The API key to use
/// * `chat_id` - The chat ID to use
/// * `messages` - The messages to send
///
/// # Returns
/// * `Ok(ChatCompletionResult)` - The chat completion result
/// * `Err(String)` - The error message
async fn chat_with_retry(
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    metadata: Option<Value>,
) -> Result<ChatCompletionResult, String> {
    for i in 0..3 {
        match complete_chat_blocking(
            api_protocol.clone(),
            api_url,
            model.clone(),
            api_key,
            chat_id.clone(),
            messages.clone(),
            tools.clone(),
            metadata.clone(),
        )
        .await
        {
            Ok(chat_result) => return Ok(chat_result),
            Err(e) => {
                if i == 2 {
                    return Err(e.to_string());
                }
                // Wait for 1 second before retrying
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        };
    }
    Err(t!("chat.chat_completion_failed_after_retries", retries = 3).to_string())
}

// Multi-engine concurrent search
async fn execute_multi_search(
    crawler_url: String,
    search_engines: &[String],
    queries: &[String],
) -> Result<Vec<SearchResult>, String> {
    if crawler_url.is_empty() {
        return Err(t!("chat.crawler_url_not_found").to_string());
    }

    // Prepare keywords - convert to owned strings to avoid lifetime issues
    let keywords_owned: Vec<String> = queries
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect();

    // Create crawler instance
    let crawler = Chp::new(crawler_url.clone(), None);

    // Concurrently execute different search engines
    let mut handles = Vec::new();

    for engine_str in search_engines {
        // Try to convert to SearchProvider
        match SearchProvider::try_from(engine_str.as_str()) {
            Ok(provider) => {
                let crawler_clone = crawler.clone();
                let keywords_clone = keywords_owned.clone();

                // Spawn async task
                let handle = tokio::spawn(async move {
                    // Get string slices from owned strings
                    let keywords_refs: Vec<&str> =
                        keywords_clone.iter().map(|s| s.as_str()).collect();
                    crawler_clone
                        .web_search(provider, &keywords_refs, None, Some(20), None, true)
                        .await
                });

                handles.push(handle);
            }
            Err(e) => {
                eprintln!("Invalid search engine: {}, error: {}", engine_str, e);
            }
        }
    }

    // If no valid search engines, return error
    if handles.is_empty() {
        return Err(t!("chat.no_valid_search_engine_config").to_string());
    }

    // Wait for all searches to complete and merge results
    let mut all_results = Vec::new();
    let results = futures::future::join_all(handles).await;

    for result in results {
        match result {
            Ok(Ok(items)) => all_results.extend(items),
            Ok(Err(e)) => eprintln!("搜索引擎错误: {}", e),
            Err(e) => eprintln!("任务执行错误: {}", e),
        }
    }

    Ok(all_results)
}

// Prepare analysis content
async fn crawl_data(crawler_url: String, results: &[SearchResult]) -> String {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // At most crawl 2 URLs
    let valid_results: Vec<&SearchResult> = results
        .iter()
        .filter(|r| r.url.starts_with("http://") || r.url.starts_with("https://"))
        .take(MAX_SEARCH_RESULT as usize)
        .collect();

    if valid_results.is_empty() {
        return "".to_string();
    }

    let crawler = Arc::new(Chp::new(crawler_url, None));
    let crawl_results = Arc::new(Mutex::new(Vec::new()));

    // Spawn multiple concurrent tasks
    let tasks = valid_results.iter().map(|result| {
        let url = result.url.clone();
        let crawler_clone = Arc::clone(&crawler);
        let results_clone = Arc::clone(&crawl_results);

        async move {
            if let Ok(crawl_result) = crawler_clone
                .web_crawler(&url, Some(json!({"format":"markdown"})))
                .await
            {
                if !crawl_result.content.is_empty() {
                    let mut results = results_clone.lock().await;
                    results.push(crawl_result);
                }
            }
        }
    });

    // Wait for all tasks to complete
    join_all(tasks).await;

    // Get results
    let mut final_results = crawl_results.lock().await;

    // Reset id, starting from 1
    for (index, result) in final_results.iter_mut().enumerate() {
        result.id = (index + 1) as i32;
    }

    if !final_results.is_empty() {
        serde_json::to_string(&*final_results).unwrap_or_else(|_| "".to_string())
    } else {
        "".to_string()
    }
}

async fn generate_final_answer(
    chat_state: Arc<ChatState>,
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    metadata: Option<Value>,
    tx: tokio::sync::mpsc::Sender<Arc<ChatResponse>>,
) -> Result<(), String> {
    let callback = move |chunk: Arc<ChatResponse>| {
        let _ = tx.try_send(chunk);
    };

    complete_chat_async(
        chat_state,
        api_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        messages,
        None,
        metadata,
        callback,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
