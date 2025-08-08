use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    scraper::webview_wrapper::WebviewScraper,
    tools::{error::ToolError, ToolDefinition, ToolResult},
};

/// A web scraper tool that uses Tauri's Webview to extract content from URLs
pub struct WebScraperTool {
    app_handle: AppHandle<tauri::Wry>,
}

impl WebScraperTool {
    pub fn new(app_handle: AppHandle<tauri::Wry>) -> Self {
        Self { app_handle }
    }
}

#[async_trait]
impl ToolDefinition for WebScraperTool {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "web_scraper"
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Extract content from web pages using a built-in webview scraper"
    }

    /// Returns the function calling specification in JSON format.
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to scrape content from"
                    },
                    "selector": {
                        "type": "string",
                        "description": "CSS selector to target specific content (optional, defaults to entire page)"
                    }
                },
                "required": ["url"]
            }),
            disabled: false,
        }
    }

    /// Executes the web scraper tool.
    async fn call(&self, params: Value) -> ToolResult {
        // Get the URL from parameters
        let url = params["url"].as_str().ok_or_else(|| {
            ToolError::FunctionParamError(t!("tools.url_must_be_string").to_string())
        })?;

        // Check if URL is empty
        if url.is_empty() {
            return Err(ToolError::FunctionParamError(
                t!("tools.url_must_not_be_empty").to_string(),
            ));
        }

        // Validate URL format
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::FunctionParamError(
                t!("tools.url_invalid", url = url).to_string(),
            ));
        }

        // Get optional selector
        let selector = params["selector"].as_str();

        // Create scraper instance
        let scraper = WebviewScraper::new(self.app_handle.clone());

        // Perform scraping
        let content = scraper.scrape_url(url, selector).await.map_err(|e| {
            ToolError::Execution(
                t!(
                    "tools.web_scraper_failed",
                    url = url,
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Return the scraped content as JSON
        Ok(json!({
            "url": url,
            "content": content,
            "selector": selector.unwrap_or("body"),
            "format": "markdown"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Note: These tests require a Tauri app handle and are integration tests
    // They should be run with proper test setup
    #[tokio::test]
    async fn test_web_scraper_params() {
        // This test just validates parameter handling without actual scraping
        let params = json!({
            "url": "https://www.iqx.me",
            "selector": "container"
        });

        let url = params["url"].as_str().unwrap();
        let selector = params["selector"].as_str();

        assert_eq!(url, "https://www.iqx.me");
        assert_eq!(selector.unwrap(), "container");
    }

    #[tokio::test]
    async fn test_web_scraper_missing_url() {
        let params = json!({});
        let url = params["url"].as_str();
        assert!(url.is_none());
    }
}
