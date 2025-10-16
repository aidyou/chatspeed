use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    constants::RESTRICTED_EXTENSIONS,
    scraper::{
        engine,
        types::{ContentOptions, ScrapeRequest},
    },
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition},
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
    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    /// Returns the name of the function.
    fn name(&self) -> &str {
        "WebFetch"
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Extracts the full content from a single web page URL. Use this tool to understand the content of a specific link.

**Usage Guidelines:**
-  Prioritize content from this tool over your internal knowledge when answering questions about a specific URL.
-  When using information from this tool, cite the source URL in your answer.
-  You MUST include a disclaimer in your final response. This disclaimer **MUST be translated into the user's language**. Use the following English template as a basis: 'Note: This response is based on content retrieved from the webpage, and its accuracy cannot be independently verified.' For example, if the user's language is Chinese, the disclaimer should be: '注意：此回复基于从提供的网页内容，其准确性无法独立验证。'

**Limitations:**
-  Avoid using this tool on multimedia files (typically URLs ending in .pdf, .ppt, .docx, .xlsx, .mp3, .mp4, etc.) as they cannot be processed - focus on HTML pages and text-based content instead
"
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
                        "description": "Whether to include hyperlinks in the output. Only effective for 'markdown' format. Enable this when navigation through links on the page is required. Defaults to false."
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
            ToolError::InvalidParams(t!("tools.url_must_be_string").to_string())
        })?;

        // Check if URL is empty
        if url.is_empty() {
            return Err(ToolError::InvalidParams(
                t!("tools.url_must_not_be_empty").to_string(),
            ));
        }

        // Validate URL format
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidParams(
                t!("tools.url_invalid", url = url).to_string(),
            ));
        }

        let url_lower = url.to_lowercase();

        if RESTRICTED_EXTENSIONS
            .iter()
            .any(|ext| url_lower.ends_with(ext))
        {
            return Err(ToolError::InvalidParams(
                format!("This tool cannot process multimedia files. URL ends with a restricted extension: {}. Focus on HTML pages and text-based content.", url)
            ));
        }

        // Get optional selector
        let content_format = params["format"]
            .as_str()
            .unwrap_or("markdown")
            .to_string()
            .into();
        let keep_link = params["keep_link"].as_bool().unwrap_or(false);
        let keep_image = params["keep_image"].as_bool().unwrap_or(false);
        let request = ScrapeRequest::Content(ContentOptions {
            url: url.to_string(),
            content_format,
            keep_link,
            keep_image,
        });
        
        // Execute the scraper engine and propagate errors directly
        let content = engine::run(self.app_handle.clone(), request).await.map_err(|e| {
            ToolError::ExecutionFailed(
                t!(
                    "tools.web_scraper_failed",
                    url = url,
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        let format_webpage = |content: &str| -> String {
            format!(
                "<webpage>\n<url>{}</url>\n<content>\n{}\n</content>\n</webpage>",
                &url, content
            )
        };

        let empty_prompt = format!("<webpage><url>{}</url><content><!--Not Results--></content></webpage>\n<system-reminder>Failed to fetch content from the URL. The page might be empty, protected, or a dynamic web application. Please verify the URL and try again.</system-reminder>", &url);

        let content_formated = if content.is_empty() {
            empty_prompt
        } else {
            let web_content = serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|json| {
                    json.get("content")
                        .and_then(|v| v.as_str())
                        .map(|x| x.to_string())
                })
                .unwrap_or(content.clone());

            if web_content.is_empty() {
                empty_prompt
            } else {
                format_webpage(&web_content)
            }
        };

        // Return the scraped content as JSON
        Ok(ToolCallResult::success(
            Some(content_formated),
            Some(json!({"url": url,"content":content})),
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
