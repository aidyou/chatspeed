//! This module is for development and testing purposes only.
//!
//! `test_scrape` is a desktop-only WebView diagnostic. The `content`/`normal`
//! paths run the local WebView scraper, which is a Tauri-only capability, and
//! stay exactly as they were.
//!
//! The `web_fetch`/`search` request types are no longer a production web
//! execution path. Runtime workflow web calls use the loopback rmcp provider;
//! this legacy diagnostic command fails closed rather than invoking the removed
//! client-pull bridge or creating a second executor.

use serde_json::Value;
use tauri::{command, AppHandle, Wry};

use crate::{
    error::{AppError, Result},
    scraper::{
        engine::run as run_scraper,
        types::{ContentOptions, ScrapeRequest},
    },
};

#[command]
pub async fn test_scrape(
    app_handle: AppHandle<Wry>,
    request_data: serde_json::Value, // Changed to accept a JSON object
) -> Result<String> {
    let request_type = request_data["type"].as_str().ok_or(AppError::General {
        message: "Missing 'type' in request data".to_string(),
    })?;

    match request_type {
        "content" | "normal" => {
            log::debug!(
                "request: {}",
                serde_json::to_string_pretty(&request_data).unwrap_or_default()
            );

            let url = request_data["url"]
                .as_str()
                .ok_or(AppError::General {
                    message: "Missing 'url' for content request".to_string(),
                })?
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
                .map_err(|e| AppError::General {
                    message: e.to_string(),
                })
        }
        // Runtime workflow web calls use the fixed loopback MCP provider. This
        // legacy diagnostic command has no direct tool invocation path and fails
        // closed rather than reaching the retired client-pull bridge.
        "web_fetch" | "search" => Err(AppError::General {
            message: "Web tools are available only through the runtime MCP provider".to_string(),
        }),
        _ => Err(AppError::General {
            message: "Invalid request type".to_string(),
        }),
    }
}
