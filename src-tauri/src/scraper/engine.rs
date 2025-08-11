//! The main engine for the scraper module.

use anyhow::{Context, Result};
use tauri::{AppHandle, Wry};
use url::Url;

use super::config_loader::ConfigLoader;
use super::webview_wrapper::WebviewScraper;

/// Represents the type of scraping request.
pub enum ScrapeRequest {
    /// A request to scrape search engine results.
    Search {
        query: String,
        provider: String,
    },
    /// A request to scrape content from a specific URL.
    Content {
        url: String,
    },
}

/// The primary entry point for the scraper module.
///
/// This function orchestrates the entire scraping process:
/// 1. Loads the appropriate configuration based on the request type.
/// 2. Constructs the target URL (for search requests).
/// 3. Initializes and runs the `WebviewScraper`.
/// 4. Returns the scraped data as a string (typically JSON).
pub async fn run(app_handle: AppHandle<Wry>, request: ScrapeRequest) -> Result<String> {
    let config_loader = ConfigLoader::new(&app_handle)?;
    let scraper = WebviewScraper::new(app_handle);

    match request {
        ScrapeRequest::Search { query, provider } => {
            let config = config_loader.load_search_config(&provider)?;
            
            // URL-encode the query to ensure it's safe for the template.
            let encoded_query = urlencoding::encode(&query);
            let url = config.config.url_template.replace("{query}", &encoded_query);

            scraper.scrape(&url, Some(config)).await
        }
        ScrapeRequest::Content { url } => {
            let url_obj = Url::parse(&url).context("Failed to parse content URL")?;
            let config = config_loader.load_content_config(&url_obj)?;
            
            scraper.scrape(&url, config).await
        }
    }
}