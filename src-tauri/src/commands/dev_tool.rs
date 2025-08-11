//! This module is for development and testing purposes only.

use crate::scraper::engine::{run as run_scraper, ScrapeRequest};
use tauri::{command, AppHandle, Wry};

#[command]
pub async fn test_scrape(
    app_handle: AppHandle<Wry>,
    request_data: serde_json::Value, // Changed to accept a JSON object
) -> Result<String, String> {
    let request_type = request_data["type"]
        .as_str()
        .ok_or("Missing 'type' in request data")?;

    let request = match request_type {
        "content" => {
            let url = request_data["url"]
                .as_str()
                .ok_or("Missing 'url' for content request")?
                .to_string();
            ScrapeRequest::Content { url }
        }
        "search" => {
            let provider = request_data["provider"]
                .as_str()
                .ok_or("Missing 'provider' for search request")?
                .to_string();
            let query = request_data["query"]
                .as_str()
                .ok_or("Missing 'query' for search request")?
                .to_string();
            ScrapeRequest::Search { query, provider }
        }
        _ => return Err("Invalid request type".to_string()),
    };

    run_scraper(app_handle, request)
        .await
        .map_err(|e| e.to_string())
}
