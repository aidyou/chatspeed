//! This module is for development and testing purposes only.

use std::sync::Arc;
use tauri::{command, AppHandle, State, Wry};

use crate::{
    ai::interaction::chat_completion::ChatState,
    constants::DEFAULT_WEB_SEARCH_TOOL,
    scraper::{
        engine::run as run_scraper,
        types::{ContentOptions, ScrapeRequest},
    },
};

#[command]
pub async fn test_scrape(
    app_handle: AppHandle<Wry>,
    chat_state: State<'_, Arc<ChatState>>,
    request_data: serde_json::Value, // Changed to accept a JSON object
) -> Result<String, String> {
    let request_type = request_data["type"]
        .as_str()
        .ok_or("Missing 'type' in request data")?;

    match request_type {
        "content" | "normal" => {
            log::debug!(
                "request: {}",
                serde_json::to_string_pretty(&request_data).unwrap_or_default()
            );

            let url = request_data["url"]
                .as_str()
                .ok_or("Missing 'url' for content request")?
                .to_string();
            let content_format = request_data["format"]
                .as_str()
                .unwrap_or("markdown")
                .to_string()
                .into();
            let keep_link = request_data["keep_link"].as_bool().unwrap_or(true);
            let keep_image = request_data["keep_image"].as_bool().unwrap_or(false);
            let request = if request_type == "content" {
                ScrapeRequest::Content(ContentOptions {
                    url,
                    content_format,
                    keep_link,
                    keep_image,
                })
            } else {
                ScrapeRequest::Normal(ContentOptions {
                    url,
                    content_format,
                    keep_link,
                    keep_image,
                })
            };
            run_scraper(app_handle, request)
                .await
                .map(|result| serde_json::to_string_pretty(&result).unwrap_or_default())
                .map_err(|e| e.to_string())
        }
        "search" => {
            log::debug!(
                "request: {}",
                serde_json::to_string_pretty(&request_data).unwrap_or_default()
            );
            let provider = request_data["provider"]
                .as_str()
                .ok_or("Missing 'provider' for search request")?
                .to_string();
            let query = request_data["query"]
                .as_str()
                .ok_or("Missing 'query' for search request")?
                .to_string();
            let page = request_data["page"].as_u64().map(|p| p as u32);
            let number = request_data["number"].as_u64().map(|n| n as u32);
            let time_period = request_data["time_period"].as_str().map(|t| t.to_string());

            chat_state
                .inner()
                .tool_manager
                .tool_call(
                    DEFAULT_WEB_SEARCH_TOOL,
                    serde_json::json!({
                        "provider": provider,
                        "kw": query,
                        "page": page,
                        "number": number,
                        "time_period": time_period,
                    }),
                )
                .await
                .map(|results| serde_json::to_string_pretty(&results).unwrap_or_default())
                .map_err(|e| e.to_string())
        }
        _ => return Err("Invalid request type".to_string()),
    }
}
