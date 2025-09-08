use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    scraper::{
        engine,
        types::{ContentOptions, ScrapeRequest},
    },
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolDefinition},
};

/// A web scraper tool that uses Tauri's Webview to extract content from URLs
pub struct WebFetch {
    app_handle: AppHandle<tauri::Wry>,
}

impl WebFetch {
    pub fn new(app_handle: AppHandle<tauri::Wry>) -> Self {
        Self { app_handle }
    }
}

#[async_trait]
impl ToolDefinition for WebFetch {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "WebFetch"
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Extracts and returns the content of a web page from a URL. This tool can process JavaScript-rendered pages, making it suitable for modern websites. Use it to get real-time information, summarize articles, or answer questions based on the content of a specific link provided by the user or found through a search."
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
                    "format": {
                        "type": "string",
                        "enum": ["markdown", "text"],
                        "description": "Format for the extracted content. Use 'markdown' to preserve structure, or 'text' for plain text. Defaults to 'markdown'."
                    },
                    "keep_link": {
                        "type": "boolean",
                        "description": "Whether to include hyperlinks in the output. Only effective for 'markdown' format. Consider setting to false if you only need the text for summarization. Defaults to true."
                    },
                    "keep_image": {
                        "type": "boolean",
                        "description": "Whether to include images in the output. Only effective when format is 'markdown'. Only set this to true if you have image-understanding capabilities and the user's query requires analyzing images. Defaults to false."
                    }
                },
                "required": ["url"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    /// Executes the web scraper tool.
    async fn call(&self, params: Value) -> NativeToolResult {
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
        let content_format = params["format"]
            .as_str()
            .unwrap_or("markdown")
            .to_string()
            .into();
        let keep_link = params["keep_link"].as_bool().unwrap_or(true);
        let keep_image = params["keep_image"].as_bool().unwrap_or(false);
        let request = ScrapeRequest::Content(ContentOptions {
            url: url.to_string(),
            content_format,
            keep_link,
            keep_image,
        });
        // Create scraper instance
        let content = engine::run(self.app_handle.clone(), request)
            .await
            .map_err(|e| {
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
        Ok(ToolCallResult::success(
            Some(content.clone()),
            Some(json!({
                "url": url,
                "content": content,
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    // Note: These tests require a Tauri app handle and are integration tests
    // They should be run with proper test setup
    #[tokio::test]
    async fn test_web_scraper_params() {
        // This test just validates parameter handling without actual scraping
        let params = json!({
            "url": "https://www.aidyou.ai",
            "selector": "container"
        });

        let url = params["url"].as_str().unwrap();
        let selector = params["selector"].as_str();

        assert_eq!(url, "https://www.aidyou.ai");
        assert_eq!(selector.unwrap(), "container");
    }

    #[tokio::test]
    async fn test_web_scraper_missing_url() {
        let params = json!({});
        let url = params["url"].as_str();
        assert!(url.is_none());
    }
}
