use async_trait::async_trait;
use reqwest::Url;
use rust_i18n::t;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    constants::RESTRICTED_EXTENSIONS,
    db::MainStore,
    http::{
        client::{HttpClient, HttpProxyConfig},
        types::HttpConfig,
    },
    scraper::{
        engine,
        types::{ContentOptions, ScrapeRequest},
    },
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition},
};

const DIRECT_TEXT_FETCH_EXTENSIONS: &[&str] = &[
    "bat",
    "c",
    "cc",
    "cfg",
    "conf",
    "cpp",
    "cs",
    "css",
    "csv",
    "cxx",
    "env",
    "gitignore",
    "go",
    "h",
    "hpp",
    "ini",
    "java",
    "js",
    "json",
    "jsx",
    "log",
    "mjs",
    "md",
    "php",
    "py",
    "rb",
    "rs",
    "scss",
    "sh",
    "sql",
    "svg",
    "toml",
    "ts",
    "tsx",
    "txt",
    "vue",
    "xml",
    "yaml",
    "yml",
];
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36";
const DIRECT_TEXT_FETCH_ACCEPT: &str = "text/plain,text/markdown,text/css,application/javascript,text/javascript,application/json,text/csv,text/html,application/xhtml+xml,application/xml,text/xml,*/*;q=0.8";
const HTML_CONTENT_TYPES: &[&str] = &["text/html", "application/xhtml+xml"];
const DIRECT_TEXT_CONTENT_TYPES: &[&str] = &[
    "application/javascript",
    "application/json",
    "application/octet-stream",
    "application/xml",
    "application/x-sh",
    "application/x-yaml",
    "text/css",
    "text/csv",
    "text/javascript",
    "text/markdown",
    "text/plain",
    "text/xml",
    "text/x-python",
    "text/x-rust",
    "text/yaml",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectFetchDecision {
    Browser,
    Direct,
}

#[derive(Debug, Clone, Default)]
struct WebFetchProxyConfig {
    server: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

/// A web scraper tool that uses Tauri's Webview to extract content from URLs
pub struct WebFetch {
    app_handle: AppHandle<tauri::Wry>,
}

impl WebFetch {
    pub fn new(app_handle: AppHandle<tauri::Wry>) -> Self {
        Self { app_handle }
    }

    fn is_direct_text_candidate(url: &str) -> bool {
        Url::parse(url)
            .ok()
            .and_then(|parsed| {
                parsed
                    .path_segments()
                    .and_then(|segments| segments.last())
                    .and_then(|filename| filename.rsplit('.').next().map(str::to_string))
            })
            .map(|ext| {
                let ext = ext.to_ascii_lowercase();
                DIRECT_TEXT_FETCH_EXTENSIONS.contains(&ext.as_str())
            })
            .unwrap_or(false)
    }

    fn classify_content_type(
        content_type: &str,
        is_direct_text_candidate: bool,
    ) -> Option<DirectFetchDecision> {
        let mime = content_type
            .split(';')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();

        if mime.is_empty() {
            return None;
        }

        if HTML_CONTENT_TYPES.contains(&mime.as_str()) {
            return Some(DirectFetchDecision::Browser);
        }

        if mime == "application/octet-stream" {
            return if is_direct_text_candidate {
                Some(DirectFetchDecision::Direct)
            } else {
                Some(DirectFetchDecision::Browser)
            };
        }

        if DIRECT_TEXT_CONTENT_TYPES.contains(&mime.as_str()) || mime.starts_with("text/") {
            return Some(DirectFetchDecision::Direct);
        }

        None
    }

    fn get_proxy(&self) -> Result<WebFetchProxyConfig, ToolError> {
        let main_store = self
            .app_handle
            .state::<std::sync::Arc<std::sync::RwLock<MainStore>>>()
            .inner();
        let store = main_store.read().map_err(|e| {
            ToolError::Store(t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())
        })?;

        let proxy_type = store.get_config("proxy_type", String::new());
        if proxy_type == "http" {
            let proxy_server = store.get_config("proxy_server", String::new());
            if proxy_server.is_empty() {
                Ok(WebFetchProxyConfig::default())
            } else {
                let proxy_username = store.get_config("proxy_username", String::new());
                let proxy_password = store.get_config("proxy_password", String::new());
                Ok(WebFetchProxyConfig {
                    server: Some(proxy_server),
                    username: if proxy_username.is_empty() {
                        None
                    } else {
                        Some(proxy_username)
                    },
                    password: if proxy_password.is_empty() {
                        None
                    } else {
                        Some(proxy_password)
                    },
                })
            }
        } else {
            Ok(WebFetchProxyConfig::default())
        }
    }

    async fn probe_direct_fetch_decision(
        &self,
        url: &str,
    ) -> Result<DirectFetchDecision, ToolError> {
        let is_direct_text_candidate = Self::is_direct_text_candidate(url);
        let proxy = self.get_proxy()?;
        let mut client_builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .user_agent(BROWSER_USER_AGENT);
        if let Some(proxy_server) = proxy.server.as_ref() {
            let mut reqwest_proxy = reqwest::Proxy::all(proxy_server).map_err(|e| {
                ToolError::ExecutionFailed(
                    t!(
                        "tools.web_scraper_failed",
                        url = url,
                        details = e.to_string()
                    )
                    .to_string(),
                )
            })?;
            if let (Some(username), Some(password)) =
                (proxy.username.as_ref(), proxy.password.as_ref())
            {
                reqwest_proxy = reqwest_proxy.basic_auth(username, password);
            }
            client_builder = client_builder.proxy(reqwest_proxy);
        }
        let client = client_builder.build().map_err(|e| {
            ToolError::ExecutionFailed(
                t!(
                    "tools.web_scraper_failed",
                    url = url,
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        let head_response = client
            .head(url)
            .header("Accept", DIRECT_TEXT_FETCH_ACCEPT)
            .send()
            .await;

        if let Ok(response) = head_response {
            if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if let Some(decision) =
                        Self::classify_content_type(content_type, is_direct_text_candidate)
                    {
                        return Ok(decision);
                    }
                }
            }
        }

        let get_response = client
            .get(url)
            .header("Accept", DIRECT_TEXT_FETCH_ACCEPT)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(
                    t!(
                        "tools.web_scraper_failed",
                        url = url,
                        details = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        if let Some(content_type) = get_response.headers().get(reqwest::header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                if let Some(decision) =
                    Self::classify_content_type(content_type, is_direct_text_candidate)
                {
                    return Ok(decision);
                }
            }
        }

        Ok(DirectFetchDecision::Browser)
    }

    async fn fetch_text_directly(&self, url: &str) -> Result<String, ToolError> {
        let proxy = self.get_proxy()?;
        let client = HttpClient::new_with_proxy_config(HttpProxyConfig {
            server: proxy.server,
            username: proxy.username,
            password: proxy.password,
        })
        .map_err(|e| {
            ToolError::ExecutionFailed(
                t!(
                    "tools.web_scraper_failed",
                    url = url,
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        let response = client
            .send_request(
                HttpConfig::get(url)
                    .header("User-Agent", BROWSER_USER_AGENT)
                    .header("Accept", DIRECT_TEXT_FETCH_ACCEPT),
            )
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(
                    t!(
                        "tools.web_scraper_failed",
                        url = url,
                        details = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        if !response.is_success() {
            return Err(ToolError::ExecutionFailed(
                t!(
                    "tools.web_scraper_failed",
                    url = url,
                    details = format!("unexpected status {}", response.status)
                )
                .to_string(),
            ));
        }

        Ok(response.body.unwrap_or_default())
    }
}

#[async_trait]
impl ToolDefinition for WebFetch {
    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Both
    }

    /// Returns the name of the function.
    fn name(&self) -> &str {
        crate::tools::TOOL_WEB_FETCH
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Extracts the full content or links from a single web page URL. Use this tool to understand the content of a specific link or discover more links on a portal/list page.

**Usage Guidelines:**
-  **For News/List/Portal pages**: Use `format: \"links\"` or set `keep_link: true` to discover the content you need.
-  **For specific articles/content**: Use `format: \"markdown\"` (default) to get the main text.
-  Prioritize content from this tool over your internal knowledge when answering questions about a specific URL.
-  When using information from this tool, cite the source URL in your answer.

**Limitations:**
-  Avoid using this tool on multimedia files (typically URLs ending in .pdf, .ppt, .docx, .xlsx, .mp3, .mp4, etc.) as they cannot be processed - focus on HTML pages and text-based content instead

**Error Handling:**
-  If a page returns empty content or fails, do NOT retry the same URL.
-  Instead, try an alternative source URL from your search results.
-  If no alternatives exist, mark the data as unavailable and proceed to the next task.
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
                        "enum": ["markdown", "text", "links"],
                        "description": "Format for the extracted content. Use 'markdown' for articles, 'text' for plain text, or 'links' for news/list/portal pages to discover more URLs. Defaults to 'markdown'."
                    },
                    "keep_link": {
                        "type": "boolean",
                        "description": "Whether to include hyperlinks in the output. Only effective for 'markdown' format. MUST be set to true for news/list/portal pages if using 'markdown' format. Defaults to false."
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
            scope: Some(self.scope()),
        }
    }

    /// Executes the web scraper tool.
    async fn call(&self, params: Value) -> NativeToolResult {
        // Get the URL from parameters
        let url = params["url"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams(t!("tools.url_must_be_string").to_string()))?;

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
        let content_format_str = params["format"].as_str().unwrap_or("markdown");

        let content_format: crate::scraper::types::StrapeContentFormat =
            content_format_str.to_string().into();

        let mut keep_link = params["keep_link"].as_bool().unwrap_or(false);

        // Force keep_link to true if format is 'links' or if we're on a portal/list page
        if content_format_str == "links" {
            keep_link = true;
        }

        let keep_image = params["keep_image"].as_bool().unwrap_or(false);
        let content = if Self::is_direct_text_candidate(url)
            && self.probe_direct_fetch_decision(url).await? == DirectFetchDecision::Direct
        {
            self.fetch_text_directly(url).await?
        } else {
            let request = ScrapeRequest::Content(ContentOptions {
                url: url.to_string(),
                content_format,
                keep_link,
                keep_image,
            });

            // Execute the scraper engine and propagate errors directly
            engine::run(self.app_handle.clone(), request)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(
                        t!(
                            "tools.web_scraper_failed",
                            url = url,
                            details = e.to_string()
                        )
                        .to_string(),
                    )
                })?
        };

        let format_webpage = |content: &str| -> String {
            format!(
                "<webpage>\n<url>{}</url>\n<content>\n{}\n</content>\n</webpage>",
                &url, content
            )
        };

        let empty_prompt = format!(
            "<webpage><url>{}</url><content><!--Not Results--></content></webpage>\n<SYSTEM_REMINDER>Failed to fetch content from this URL. Do NOT retry the same URL immediately. The page may be empty, protected, or a dynamic web application. Try a different source URL if available; otherwise treat the data as unavailable and continue.</SYSTEM_REMINDER>",
            &url
        );

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
    use crate::tools::web_fetch::DirectFetchDecision;

    use super::WebFetch;
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

    #[test]
    fn test_is_direct_text_candidate_for_known_text_extensions() {
        assert!(WebFetch::is_direct_text_candidate(
            "https://example.com/assets/app.js"
        ));
        assert!(WebFetch::is_direct_text_candidate(
            "https://example.com/docs/readme.md?raw=1"
        ));
        assert!(WebFetch::is_direct_text_candidate(
            "https://example.com/data/report.CSV"
        ));
        assert!(WebFetch::is_direct_text_candidate(
            "https://example.com/styles/site.css#hash"
        ));
    }

    #[test]
    fn test_is_not_direct_text_candidate_for_non_text_extensions() {
        assert!(!WebFetch::is_direct_text_candidate(
            "https://example.com/image.png"
        ));
        assert!(!WebFetch::is_direct_text_candidate(
            "https://example.com/archive.zip"
        ));
        assert!(!WebFetch::is_direct_text_candidate(
            "https://example.com/article"
        ));
    }

    #[test]
    fn test_classify_content_type_prefers_browser_for_html() {
        assert_eq!(
            WebFetch::classify_content_type("text/html; charset=utf-8", true),
            Some(DirectFetchDecision::Browser)
        );
        assert_eq!(
            WebFetch::classify_content_type("application/xhtml+xml", true),
            Some(DirectFetchDecision::Browser)
        );
    }

    #[test]
    fn test_classify_content_type_allows_direct_text_fetch() {
        assert_eq!(
            WebFetch::classify_content_type("text/markdown; charset=utf-8", true),
            Some(DirectFetchDecision::Direct)
        );
        assert_eq!(
            WebFetch::classify_content_type("application/json", true),
            Some(DirectFetchDecision::Direct)
        );
        assert_eq!(
            WebFetch::classify_content_type("text/plain", true),
            Some(DirectFetchDecision::Direct)
        );
    }

    #[test]
    fn test_classify_content_type_allows_octet_stream_for_text_candidates() {
        assert_eq!(
            WebFetch::classify_content_type("application/octet-stream", true),
            Some(DirectFetchDecision::Direct)
        );
        assert_eq!(
            WebFetch::classify_content_type("application/octet-stream", false),
            Some(DirectFetchDecision::Browser)
        );
    }

    #[test]
    fn test_classify_content_type_returns_none_for_unknown_types() {
        assert_eq!(
            WebFetch::classify_content_type("application/unknown", true),
            None
        );
        assert_eq!(WebFetch::classify_content_type("", true), None);
    }
}
