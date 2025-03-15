use phf::phf_set;
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use super::chat::setup_chat_proxy;
use crate::ai::interaction::chat_completion::{
    complete_chat_async, complete_chat_blocking, ChatProtocol, ChatState,
};
use crate::ai::traits::chat::MessageType;
use crate::constants::{CFG_SEARCH_ENGINE, CHATSPEED_CRAWLER};
use crate::http::crawler::{Crawler, SearchProvider, SearchResult};
use crate::{ai::traits::chat::ChatResponse, db::MainStore};

const MAX_SEARCH_RESULT: i32 = 2;

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
    window: tauri::Window, // The Tauri window instance, automatically injected by Tauri
    chat_state: State<'_, Arc<ChatState>>, // The state of the chat system, automatically injected by Tauri
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    mut messages: Vec<Value>,
    mut metadata: Option<Value>,
) -> Result<(), String> {
    let content = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if content.is_empty() {
        return Err("Failed to find user message content".to_string());
    }

    let tx = chat_state
        .channels
        .get_or_create_channel(window.clone())
        .await?;

    let crawler_url = main_state
        .lock()
        .map_err(|e| e.to_string())?
        .get_config(CHATSPEED_CRAWLER, String::new());

    let search_engines = main_state
        .lock()
        .map_err(|e| e.to_string())?
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
        let search_queries = if content.len() > 150 {
            generate_search_queries(
                &chat_state,
                protocol.clone(),
                url.as_deref(),
                model.clone(),
                key.as_deref(),
                chat_id.clone(),
                &content,
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
                &chat_state,
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
            let analysis_content = fetch_data(crawler_url, &ranked_results).await;

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

    super::chat::setup_chat_context(&main_state, &mut messages, &mut metadata);

    generate_final_answer(
        &chat_state,
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

    // 从配置中获取搜索模型的设置
    let search_model = main_state
        .lock()
        .map_err(|e| e.to_string())?
        .get_config("websearch_model", Value::Null);

    // 如果有搜索模型的配置，则使用该配置
    if !search_model.is_null() {
        // 获取模型ID
        let model_id = search_model["id"].as_i64().unwrap_or_default();

        let ai_model = main_state
            .lock()
            .map_err(|e| e.to_string())?
            .config
            .get_ai_model_by_id(model_id)
            .map_err(|e| format!("Failed to get AI model: {}", e));

        // 从配置中获取模型详细信息
        if let Ok(smodel) = ai_model {
            // 更新搜索API的配置
            search_api_protocol = smodel
                .api_protocol
                .try_into()
                .map_err(|e: String| e.to_string())?;
            search_api_url = Some(smodel.base_url.clone());
            search_api_key = Some(smodel.api_key.clone());

            // 只在模型的代理类型与传入 metadata 的代理类型不同时才进行修改
            let model_proxy_type = if let Some(md) = smodel.metadata {
                md.get("proxyType").map(|v| v.clone())
            } else {
                None
            };
            if let Some(md) = search_metadata.as_mut() {
                if let Some(obj) = md.as_object_mut() {
                    // 检查当前 metadata 中是否已有 proxyType
                    let current_proxy_type = obj.get("proxyType").map(|v| v.clone());

                    // 如果当前没有 proxyType 或者与模型的 proxyType 不同，才进行更新
                    if current_proxy_type.is_none() || current_proxy_type != model_proxy_type {
                        obj.insert(
                            "proxyType".to_string(),
                            model_proxy_type.unwrap_or(Value::Null).clone(),
                        );
                        change_proxy_type = true;
                    }
                }
            }

            // 从配置中获取模型名称
            let model_name = search_model["model"]
                .as_str()
                .unwrap_or_default()
                .to_string();

            // 验证模型名称是否有效
            if !smodel.models.contains(&model_name) {
                return Err(format!("Invalid search model: {}", model_name));
            }

            // 更新搜索模型
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
    let chunk = ChatResponse::new_with_arc(chat_id, msg, MessageType::Step, metadata);
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
    state: &ChatState,
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    question: &str,
    metadata: Option<Value>,
) -> Result<Vec<String>, String> {
    let prompt = format!(
        "[Rules]\n1. ​Language Detection \n   - Respond with keywords in the same language as the user's input  \n   - Handle mixed-language queries (e.g. 苹果发布会 → \"Apple event\")\n\n2. Core Extraction  \n   - Entities (nouns): [PRODUCT][LOCATION][EVENT]  \n   - Actions (verbs): [how-to][compare][find]  \n   - Modifiers: [CurrentYear][latest][free]\n\n3. Output Format  \n   - Generate 1 concise search query per query.  \n   - No additional explanation or text, only search queries.  \n   - Each query should be precise, with relevant and specific keywords.\n   - Remove any numbering or additional commentary in the output.\n\n[Examples]\nUser Input: \"Best budget 5G phones under $300\"\nYour Output: \"5G phones\" \"budget\" under $300\n\n[Response Template]\nGenerate search queries for:\n\n{}",
        question
    );

    let chat_reslut = complete_chat_blocking(
        state,
        chat_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        vec![json!({"role": "user", "content": prompt})],
        metadata,
    )
    .await?;

    // 解析结果为查询词列表
    let queries: Vec<String> = chat_reslut
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .take(3)
        .collect();

    Ok(queries)
}

async fn filter_and_rank_search_results(
    state: &ChatState,
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

    let chat_result = complete_chat_blocking(
        state,
        api_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        vec![json!({"role": "user", "content": prompt})],
        metadata,
    )
    .await?;

    // 解析结果为查询词列表
    let queries = chat_result
        .lines()
        .filter(|line| {
            let l = line.trim();
            !l.is_empty() && !l.starts_with("```json") && !l.ends_with("```")
        })
        .map(|line| line.trim().to_string())
        .collect::<Vec<String>>()
        .join("\n");

    let result: Vec<SearchResult> = serde_json::from_str(&queries).map_err(|e| e.to_string())?;

    Ok(result)
}

// 多引擎并发搜索
async fn execute_multi_search(
    crawler_url: String,
    search_engines: &[String],
    queries: &[String],
) -> Result<Vec<SearchResult>, String> {
    if crawler_url.is_empty() {
        return Err("Crawler URL not found".to_string());
    }

    // 准备关键词 - 转换为拥有所有权的字符串以避免生命周期问题
    let keywords_owned: Vec<String> = queries
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect();

    // 创建爬虫实例
    let crawler = Crawler::new(crawler_url.clone());

    // 并发执行不同搜索引擎
    let mut handles = Vec::new();

    for engine_str in search_engines {
        // 尝试转换为 SearchProvider
        match SearchProvider::try_from(engine_str.as_str()) {
            Ok(provider) => {
                let crawler_clone = crawler.clone();
                let keywords_clone = keywords_owned.clone();

                // 创建异步任务
                let handle = tokio::spawn(async move {
                    // 从拥有所有权的字符串中获取字符串切片
                    let keywords_refs: Vec<&str> =
                        keywords_clone.iter().map(|s| s.as_str()).collect();
                    crawler_clone
                        .search(provider, &keywords_refs, None, Some(20), true)
                        .await
                });

                handles.push(handle);
            }
            Err(e) => {
                eprintln!("无效的搜索引擎: {}, 错误: {}", engine_str, e);
                // 继续处理其他搜索引擎
            }
        }
    }

    // 如果没有有效的搜索引擎，返回错误
    if handles.is_empty() {
        return Err("没有有效的搜索引擎配置".to_string());
    }

    // 等待所有搜索完成并合并结果
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

// 准备分析内容
async fn fetch_data(crawler_url: String, results: &[SearchResult]) -> String {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // 最多爬取2个URL
    let valid_results: Vec<&SearchResult> = results
        .iter()
        .filter(|r| r.url.starts_with("http://") || r.url.starts_with("https://"))
        .take(MAX_SEARCH_RESULT as usize)
        .collect();

    if valid_results.is_empty() {
        return "".to_string();
    }

    let crawler = Arc::new(Crawler::new(crawler_url));
    let crawl_results = Arc::new(Mutex::new(Vec::new()));

    // 创建多个并发任务
    let tasks = valid_results.iter().map(|result| {
        let url = result.url.clone();
        let crawler_clone = Arc::clone(&crawler);
        let results_clone = Arc::clone(&crawl_results);

        async move {
            if let Ok(crawl_result) = crawler_clone.fetch(&url).await {
                if !crawl_result.content.is_empty() {
                    let mut results = results_clone.lock().await;
                    results.push(crawl_result);
                }
            }
        }
    });

    // 等待所有任务完成
    join_all(tasks).await;

    // 获取结果
    let mut final_results = crawl_results.lock().await;

    // 重新设置id，从1开始累加
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
    chat_state: &ChatState,
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
        metadata,
        callback,
    )
    .await?;

    Ok(())
}
