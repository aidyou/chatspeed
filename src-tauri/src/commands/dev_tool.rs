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

use std::sync::Arc;

use serde_json::{Map, Value};
use tauri::{command, AppHandle, Manager, Wry};

use crate::{
    capability::mcp_service::WEB_MCP_VIRTUAL_ID,
    error::{AppError, Result},
    runtime_capability,
    runtime_client::RuntimeSupervisor,
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
        // The desktop WebView hosts these capabilities only as the fixed MCP
        // provider. The diagnostic adapter therefore uses the runtime's normal
        // MCP call route with the in-memory provider identity; it never invokes
        // WebView code directly and never creates a second tool owner.
        "web_fetch" | "search" => {
            let tool_name = if request_type == "web_fetch" {
                "web_fetch"
            } else {
                "web_search"
            };
            let arguments = web_mcp_arguments(tool_name, &request_data)?;
            let supervisor = app_handle
                .try_state::<Arc<RuntimeSupervisor>>()
                .ok_or_else(|| AppError::General {
                    message: "The runtime MCP provider is unavailable".to_string(),
                })?;
            runtime_capability::mcp_call(
                supervisor.inner().as_ref(),
                WEB_MCP_VIRTUAL_ID,
                tool_name,
                &arguments,
            )
            .await
            .map(|result| serde_json::to_string_pretty(&result).unwrap_or_default())
            .map_err(|error| AppError::General {
                message: error.redacted_message(),
            })
        }
        _ => Err(AppError::General {
            message: "Invalid request type".to_string(),
        }),
    }
}

/// Converts the legacy scraper-test payload to the fixed provider MCP schema.
fn web_mcp_arguments(tool_name: &str, request_data: &Value) -> Result<Value> {
    let mut arguments = Map::new();
    if tool_name == "web_fetch" {
        let url = request_data["url"]
            .as_str()
            .ok_or_else(|| AppError::General {
                message: "Missing 'url' for web_fetch request".to_string(),
            })?;
        arguments.insert("url".to_string(), Value::String(url.to_string()));
        for key in ["format", "keep_link", "keep_image"] {
            if let Some(value) = request_data.get(key) {
                arguments.insert(key.to_string(), value.clone());
            }
        }
    } else {
        let query = request_data["query"]
            .as_str()
            .ok_or_else(|| AppError::General {
                message: "Missing 'query' for web_search request".to_string(),
            })?;
        arguments.insert("query".to_string(), Value::String(query.to_string()));
        for key in ["provider", "page", "number", "time_period"] {
            if let Some(value) = request_data.get(key) {
                arguments.insert(key.to_string(), value.clone());
            }
        }
    }
    Ok(Value::Object(arguments))
}
