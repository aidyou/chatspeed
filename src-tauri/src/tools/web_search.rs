use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{str::FromStr, sync::Arc};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    constants::CFG_SEARCH_ENGINE,
    db::MainStore,
    search::{
        BuiltInSearch, GoogleSearch, SearchFactory, SearchProvider, SearchProviderName,
        SerperSearch, TavilySearch,
    },
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition},
};
use tauri::{AppHandle, Manager, Wry};

pub struct Auth {
    pub api_key: String,
    pub cx: Option<String>, // google search id
}
impl Auth {
    pub fn new(api_key: String) -> Self {
        Auth { api_key, cx: None }
    }
    pub fn new_with_cx(api_key: String, cx: Option<String>) -> Self {
        Auth { api_key, cx }
    }
}

pub struct WebSearch {
    app_handle: AppHandle<Wry>,
}

impl WebSearch {
    pub fn new(app_handle: AppHandle<Wry>) -> Arc<Self> {
        Arc::new(Self { app_handle })
    }

    fn create_searcher(&self, provider: Option<String>) -> Result<SearchFactory, ToolError> {
        let main_store = self
            .app_handle
            .state::<Arc<std::sync::RwLock<MainStore>>>()
            .inner();
        let store = main_store.read().map_err(|e| {
            ToolError::Store(t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())
        })?;

        let search_engine =
            provider.unwrap_or_else(|| store.get_config(CFG_SEARCH_ENGINE, "bing".to_string()));

        let proxy_type = store.get_config("proxy_type", "".to_string());
        let proxy = if proxy_type == "http" {
            let s = store.get_config("proxy_server", "".to_string());
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        } else {
            None
        };

        let provider_name = SearchProviderName::from_str(&search_engine)
            .map_err(|e| ToolError::Initialization(e))?;

        let searcher = match provider_name {
            SearchProviderName::Google => {
                let auth = Self::get_and_check_auth(&search_engine, main_store.clone())?;
                SearchFactory::Google(
                    GoogleSearch::new(auth.api_key, auth.cx.unwrap_or_default(), proxy)
                        .map_err(|e| ToolError::Initialization(e.to_string()))?,
                )
            }
            SearchProviderName::Tavily => {
                let auth = Self::get_and_check_auth(&search_engine, main_store.clone())?;
                SearchFactory::Tavily(
                    TavilySearch::new(auth.api_key, proxy)
                        .map_err(|e| ToolError::Initialization(e.to_string()))?,
                )
            }
            SearchProviderName::Serper => {
                let auth = Self::get_and_check_auth(&search_engine, main_store.clone())?;
                SearchFactory::Serper(
                    SerperSearch::new(auth.api_key, proxy)
                        .map_err(|e| ToolError::Initialization(e.to_string()))?,
                )
            }
            p @ (SearchProviderName::Bing
            | SearchProviderName::Brave
            | SearchProviderName::DuckDuckGo
            | SearchProviderName::So
            | SearchProviderName::Sogou) => SearchFactory::Builtin(BuiltInSearch {
                app_handle: self.app_handle.clone(),
                provider: p,
            }),
        };
        Ok(searcher)
    }

    /// Get and check authentication for the search engine.
    pub fn get_and_check_auth(
        search_engine: &str,
        main_store: Arc<std::sync::RwLock<MainStore>>,
    ) -> Result<Auth, ToolError> {
        let store = main_store.read().map_err(|e| {
            ToolError::Store(t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())
        })?;
        let provider_name = SearchProviderName::from_str(search_engine)
            .map_err(|e| ToolError::Initialization(e))?;
        let auth = match provider_name {
            SearchProviderName::Google => {
                let api_key = store.get_config("google_api_key", "".to_string());
                let cx = store.get_config("google_search_id", "".to_string());
                if api_key.is_empty() {
                    return Err(ToolError::Config(
                        t!("tools.search.google_api_key_empty").to_string(),
                    ));
                }
                if cx.is_empty() {
                    return Err(ToolError::Config(
                        t!("tools.search.google_cx_empty").to_string(),
                    ));
                }
                Auth::new_with_cx(api_key, Some(cx))
            }
            SearchProviderName::Tavily => {
                let api_key = store.get_config("tavily_api_key", "".to_string());
                if api_key.is_empty() {
                    return Err(ToolError::Config(
                        t!("tools.search.tavily_api_key_empty").to_string(),
                    ));
                }
                Auth::new(api_key)
            }
            SearchProviderName::Serper => {
                let api_key = store.get_config("serper_api_key", "".to_string());
                if api_key.is_empty() {
                    return Err(ToolError::Config(
                        t!("tools.search.serper_api_key_empty").to_string(),
                    ));
                }
                Auth::new(api_key)
            }
            SearchProviderName::Bing
            | SearchProviderName::Brave
            | SearchProviderName::DuckDuckGo
            | SearchProviderName::So
            | SearchProviderName::Sogou => {
                // Built-in providers don't need auth.
                return Ok(Auth::new("".to_string()));
            }
        };
        Ok(auth)
    }

    /// Extract keywords from params
    ///
    /// # Arguments
    /// * `params` - The parameters for the function, which must contain a "query" field
    ///             and it must be a string or array of strings.
    ///             When the query is array, it will be joined by space with double quotes.
    ///             `["foo", "bar"]` -> `"foo" "bar"`
    ///
    /// # Returns
    /// * `Result<String, ToolError>` - The encoded keywords
    pub fn extract_keywords(params: &Value) -> Result<String, ToolError> {
        match &params["query"] {
            Value::String(s) => {
                let s = s.trim();
                if s.is_empty() {
                    return Err(ToolError::FunctionParamError(
                        t!("tools.keyword_must_be_non_empty").to_string(),
                    ));
                }
                Ok(s.to_string())
            }
            Value::Array(arr) => {
                let keywords: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", s))) // warp double quotes to each kw
                    .collect();
                if keywords.is_empty() {
                    return Err(ToolError::FunctionParamError(
                        t!("tools.keyword_must_be_non_empty").to_string(),
                    ));
                }
                let joined_keywords = keywords.join(" "); // join with space
                Ok(joined_keywords)
            }
            _ => Err(ToolError::FunctionParamError(
                t!("tools.keyword_must_be_non_empty").to_string(),
            )),
        }
    }

    // pub async fn scrape_content(
    //     &self,
    //     mut results: Vec<SearchResult>,
    // ) -> Result<Vec<SearchResult>, ToolError> {
    //     if results.is_empty() {
    //         return Ok(results);
    //     }

    //     let main_store = self
    //         .app_handle
    //         .state::<Arc<std::sync::RwLock<MainStore>>>()
    //         .inner();
    //     let concurrency = {
    //         let store = main_store
    //             .read()
    //             .map_err(|e| ToolError::Store(e.to_string()))?;
    //         store.get_config(crate::constants::CFG_SCRAPER_CONCURRENCY_COUNT, 3_u64) as usize
    //     };
    //     let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));

    //     let mut scrape_tasks = Vec::new();

    //     for (index, result) in results.iter().enumerate() {
    //         if result.content.as_deref().unwrap_or("").is_empty() && !result.url.is_empty() {
    //             let app_handle = self.app_handle.clone();
    //             let url = result.url.clone();
    //             let permit = semaphore
    //                 .clone()
    //                 .acquire_owned()
    //                 .await
    //                 .map_err(|e| ToolError::Execution(e.to_string()))?;

    //             let task = tokio::spawn(async move {
    //                 let _permit = permit;

    //                 // 1. Wrap the core scraping logic in an async block
    //                 let scrape_future = async {
    //                     let request = crate::scraper::types::ScrapeRequest::Content(
    //                         crate::scraper::types::ContentOptions {
    //                             url: url.clone(), // clone url for logging on timeout
    //                             content_format: StrapeContentFormat::Markdown,
    //                             keep_link: false,
    //                             keep_image: false,
    //                         },
    //                     );

    //                     if let Ok(scraped_json_str) =
    //                         crate::scraper::engine::run(app_handle, request).await
    //                     {
    //                         match serde_json::from_str::<Value>(&scraped_json_str) {
    //                             Ok(scraped_data) => {
    //                                 return (
    //                                     index,
    //                                     scraped_data["content"]
    //                                         .as_str()
    //                                         .map(|s| s.trim().to_string()),
    //                                     scraped_data["url"].as_str().map(String::from),
    //                                 );
    //                             }
    //                             Err(e) => {
    //                                 log::error!(
    //                                     "Failed to parse scraped JSON: {}, rawdata: {}",
    //                                     e,
    //                                     scraped_json_str
    //                                 );
    //                             }
    //                         }
    //                     }
    //                     (index, None, None)
    //                 };

    //                 // 2. Wrap the future with tokio::time::timeout
    //                 let timeout_duration = std::time::Duration::from_secs(10);
    //                 if let Ok(result) = tokio::time::timeout(timeout_duration, scrape_future).await
    //                 {
    //                     // If completed within 10 seconds, return its result
    //                     result
    //                 } else {
    //                     // If timeout occurs
    //                     log::warn!("Scraping task for url {} timed out after 10 seconds.", &url);
    //                     // Return a tuple indicating failure
    //                     (index, None, None)
    //                 }
    //             });
    //             scrape_tasks.push(task);
    //         }
    //     }

    //     let scraped_contents = join_all(scrape_tasks).await;

    //     for task_result in scraped_contents {
    //         if let Ok((index, opt_content, opt_url)) = task_result {
    //             if let Some(result) = results.get_mut(index) {
    //                 if opt_content.as_deref().map_or(false, |s| !s.is_empty()) {
    //                     result.content = opt_content;
    //                     result.snippet = None; // If content is not empty, remove snippet
    //                 }

    //                 if let Some(url) = opt_url {
    //                     result.url = url;
    //                 }
    //             }
    //         }
    //     }

    //     #[cfg(debug_assertions)]
    //     log::debug!("Current results: {}", results.len());

    //     // Filter out video and image domains
    //     results.retain(|result| {
    //         !VIDEO_AND_IMAGE_DOMAINS
    //             .iter()
    //             .any(|domain| result.url.contains(domain))
    //     });

    //     #[cfg(debug_assertions)]
    //     log::debug!("Filtered results: {}", results.len());

    //     Ok(results)
    // }
}

#[async_trait]
impl ToolDefinition for WebSearch {
    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    /// Returns the name of the function.
    fn name(&self) -> &str {
        "WebSearch"
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Performs a web search to find information. This is the primary tool for answering questions about current events or topics beyond your knowledge cutoff.

**Workflow:**
1.  **Search:** Use this tool to get a list of results (with titles, URLs, and snippets).
2.  **Fetch:** Analyze the snippets to identify the most relevant URLs, then use the `WebFetch` tool to get the full page content for a deep understanding. DO NOT base answers on snippets alone.

**Usage Guidelines:**
-  **Prioritize Sources:** When analyzing search results, give preference to authoritative sources like official websites, well-known news outlets, and academic institutions.
-  For time-sensitive queries (e.g., 'recent', 'latest'), use the `time_period` parameter.

**Error & Retry Strategy:**
-  **On Timeout:** If a search request times out, you may retry the exact same query up to 2 times.
-  **On No Results:** If a search returns no results, you **must** adjust your strategy. Do not simply repeat the same query. Instead, modify your keywords (make them broader or use different terms) or adjust the `time_period`. You should make up to 3 such modification attempts before concluding the information is unavailable."
    }

    /// Returns the function calling spec.
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query or keyword. For best results, use specific and concise language. Can also be an array of strings for exact phrase matching."
                        },
                        "page": {
                            "type": "integer",
                            "default": 1,
                            "description": "The page number of the search results to return. Use this in conjunction with the 'number' parameter to paginate through more than 10 results. Defaults to 1."
                        },
                        "number": {
                            "type": "integer",
                            "default": 10,
                            "minimum": 1,
                            "maximum": 10,
                            "description": "Number of results to return, between 1 and 10. For more results, use the 'page' parameter."
                        },
                        "time_period": {
                            "type": "string",
                            "enum": ["day", "week", "month", "year"],
                            "description": "Filters search results to a specific time range. Use this to find recent or timely information. If omitted, no time filter is applied."
                        }
                    },
                    "required": ["query"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    /// Executes the function with the given parameters and context.
    ///
    /// # Arguments
    /// * `params` - The parameters of the function.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the result of the function execution.
    async fn call(&self, params: Value) -> NativeToolResult {
        // Optional provider for testing, usually not shown in `input_schema`
        let provider = params
            .get("provider")
            .map(|p| p.as_str().unwrap())
            .map(|p| p.to_string());
        let searcher = self.create_searcher(provider)?;

        let query = Self::extract_keywords(&params)?;
        if query.is_empty() {
            return Err(ToolError::FunctionParamError(
                t!("tools.keyword_must_be_non_empty").to_string(),
            ));
        }

        let number = params["number"].as_i64().unwrap_or(5).min(10).max(1);
        let page = params["page"].as_i64().unwrap_or(1).max(1);
        let time_period = params["time_period"].as_str().unwrap_or("");

        let period = match time_period {
            "day" => Some(crate::search::SearchPeriod::Day),
            "week" => Some(crate::search::SearchPeriod::Week),
            "month" => Some(crate::search::SearchPeriod::Month),
            "year" => Some(crate::search::SearchPeriod::Year),
            _ => None,
        };

        let search_params = json!({
            "query": query.clone(),
            "count": number,
            "period": period,
            "page": page,
        });

        let results = searcher.search(&search_params).await.map_err(|e| {
            ToolError::Execution(
                t!(
                    "tools.search.web_search_failed",
                    provider = searcher.to_string(),
                    keyword = query.clone(),
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        #[cfg(debug_assertions)]
        log::debug!("Scrape result count: {}", results.len());

        // The automatic scraping of content has been removed to support the agent workflow.
        // The AI should decide whether to use the WebFetch tool to get the full content.
        // results = self.scrape_content(results).await?;

        #[cfg(debug_assertions)]
        log::debug!(
            "Scrape result count after content scraped: {}",
            results.len()
        );

        let results_with_id = results
            .iter()
            .enumerate()
            .map(|(index, r)| {
                let mut sr = r.clone();
                sr.id = index + 1;
                sr
            })
            .collect::<Vec<_>>();
        let result_string = if results_with_id.is_empty() {
            "[/*Not Results*/]\n<system-reminder>No web search results. Suggestions: check your spelling, try different keywords, adjust the time range, or use more general terms.</system-reminder>".to_string()
        } else {
            serde_json::to_string(&results_with_id).unwrap_or_default()
        };

        Ok(ToolCallResult::success(
            Some(result_string),
            Some(json!(results_with_id)),
        ))
    }
}
