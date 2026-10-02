//! This module is for development and testing purposes only.
//!
//! `test_scrape` is a desktop-only WebView diagnostic. The `content`/`normal`
//! paths run the local WebView scraper, which is a Tauri-only capability, and
//! stay exactly as they were.
//!
//! The `web_fetch`/`search` paths used the runtime-owned `ToolManager`'s web
//! tools. Those tools are owned by the runtime, which never links a WebView, so
//! the paths now travel through the runtime's client WebView capability
//! contract. It reports `available` only while the runtime has a live,
//! lease-bound Tauri bridge; otherwise the command fails closed with the
//! runtime's structured `unavailable` status instead of reaching a second
//! `ToolManager` or pretending the WebView tool ran.

use std::sync::Arc;

use serde_json::Value;
use tauri::{command, AppHandle, Manager, Wry};

use crate::{
    commands::capability::runtime_capability,
    error::{AppError, Result},
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
        // The runtime owns the fetch/search web tools; the desktop only has the
        // WebView scraper above. The request goes through the runtime's client
        // capability contract: it reports `available` only while a live bridge
        // declares the capability (no second ToolManager, no local execution).
        "web_fetch" | "search" => {
            invoke_runtime_web_capability(&app_handle, request_type, &request_data).await
        }
        _ => Err(AppError::General {
            message: "Invalid request type".to_string(),
        }),
    }
}

/// Bridges a desktop web-tool request to the runtime's client capability
/// contract.
///
/// The runtime owns `web_fetch`/`web_search`; the desktop executes them only
/// through its lease-bound client bridge. The command reads the runtime's
/// capability registry, invokes only a declared capability, and forwards the
/// structured error otherwise. It never opens a local database or creates a
/// second tool owner.
async fn invoke_runtime_web_capability(
    app_handle: &AppHandle<Wry>,
    request_type: &str,
    request_data: &Value,
) -> Result<String> {
    let capability = if request_type == "web_fetch" {
        "web_fetch"
    } else {
        "web_search"
    };
    let request = capability_request(capability, request_data)?;

    let supervisor = app_handle
        .try_state::<Arc<RuntimeSupervisor>>()
        .ok_or_else(|| AppError::General {
            message: format!(
                "The `{capability}` web tool is owned by the runtime, and no runtime client is connected"
            ),
        })?;

    // Discovery gate: only a capability the runtime declares available is
    // invoked, so the desktop never calls into a bridge the runtime does not own.
    let registry = runtime_capability::client_capabilities(supervisor.inner().as_ref())
        .await
        .map_err(|error| capability_error(capability, &error))?;
    match capability_status(&registry, capability) {
        Some((status, _)) if status == "available" => {}
        Some((status, detail)) => {
            return Err(AppError::General {
                message: format!("Client capability `{capability}` is {status}: {detail}"),
            });
        }
        None => {
            return Err(AppError::General {
                message: format!("The runtime does not declare client capability `{capability}`"),
            });
        }
    }

    let result = runtime_capability::invoke_client_capability(
        supervisor.inner().as_ref(),
        capability,
        &request,
    )
    .await
    .map_err(|error| capability_error(capability, &error))?;
    serde_json::to_string_pretty(&result).map_err(|error| AppError::General {
        message: error.to_string(),
    })
}

/// Maps a runtime capability failure to the desktop's general error wire.
fn capability_error(
    capability: &str,
    error: &crate::capability::error::CapabilityError,
) -> AppError {
    AppError::General {
        message: format!(
            "Client capability `{capability}` failed: {}: {}",
            error.code(),
            error.redacted_message()
        ),
    }
}

/// Reads one capability's `(status, detail)` from the runtime registry.
fn capability_status(registry: &Value, capability: &str) -> Option<(String, String)> {
    let entries = registry.get("capabilities")?.as_array()?;
    for entry in entries {
        if entry.get("name").and_then(Value::as_str) == Some(capability) {
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let detail = entry
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Some((status, detail));
        }
    }
    None
}

/// Builds the typed invocation request for one client web capability.
///
/// Only the capability's declared arguments are forwarded, so the desktop can
/// never turn the bridge into a generic passthrough of arbitrary `request_data`.
fn capability_request(capability: &str, request_data: &Value) -> Result<Value> {
    if capability == "web_fetch" {
        let url = request_data["url"]
            .as_str()
            .ok_or_else(|| AppError::General {
                message: "Missing 'url' for web_fetch request".to_string(),
            })?;
        let mut request = serde_json::Map::new();
        request.insert("url".to_string(), Value::String(url.to_string()));
        if let Some(format) = request_data["format"].as_str() {
            request.insert("format".to_string(), Value::String(format.to_string()));
        }
        if let Some(keep_link) = request_data["keep_link"].as_bool() {
            request.insert("keep_link".to_string(), Value::Bool(keep_link));
        }
        if let Some(keep_image) = request_data["keep_image"].as_bool() {
            request.insert("keep_image".to_string(), Value::Bool(keep_image));
        }
        return Ok(Value::Object(request));
    }

    let query = request_data["query"]
        .as_str()
        .ok_or_else(|| AppError::General {
            message: "Missing 'query' for web_search request".to_string(),
        })?;
    let mut request = serde_json::Map::new();
    request.insert("query".to_string(), Value::String(query.to_string()));
    // The desktop wire names the result count `number`; the runtime capability
    // schema names it `limit`.
    if let Some(limit) = request_data["number"].as_u64() {
        request.insert("limit".to_string(), Value::Number(limit.into()));
    }
    Ok(Value::Object(request))
}
