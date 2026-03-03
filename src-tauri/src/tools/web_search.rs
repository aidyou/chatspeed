use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{str::FromStr as _, sync::Arc};
use url::Url;

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    constants::{CFG_SEARCH_ENGINE, RESTRICTED_EXTENSIONS, VIDEO_AND_IMAGE_DOMAINS},
    db::MainStore,
    scraper::url_helper::{decode_bing_url, get_meta_refresh_url},
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

    fn create_searcher(
        &self,
        provider: Option<String>,
    ) -> Result<(SearchFactory, SearchProviderName), ToolError> {
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

        let searcher = match provider_name.clone() {
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
        Ok((searcher, provider_name))
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
                    return Err(ToolError::InvalidParams(
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
                    return Err(ToolError::InvalidParams(
                        t!("tools.keyword_must_be_non_empty").to_string(),
                    ));
                }
                let joined_keywords = keywords.join(" "); // join with space
                Ok(joined_keywords)
            }
            _ => Err(ToolError::InvalidParams(
                t!("tools.keyword_must_be_non_empty").to_string(),
            )),
        }
    }
}

#[async_trait]
impl ToolDefinition for WebSearch {
    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Both
    }

    /// Returns the name of the function.
    fn name(&self) -> &str {
        crate::tools::TOOL_WEB_SEARCH
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Search the web for up-to-date information, current events, or data beyond your knowledge cutoff. \
        Returns a list of search results including titles, snippets, and source URLs.\n\n\
        **Best Practices:**\n\
        - Use specific, targeted queries. Avoid vague or overly broad searches.\n\
        - For Chinese topics, prefer searching in Chinese for better results.\n\
        - ALWAYS analyze search results before deciding on next action.\n\
        - After reviewing results, use web_fetch on the 1-3 most relevant URLs.\n\
        - If results are insufficient, try completely different keywords before searching again.\n\
        - Do NOT call web_search repeatedly with similar queries."
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
                            "default": 5,
                            "minimum": 1,
                            "maximum": 30,
                            "description": "Number of results to return, between 1 and 30. For more results, use the 'page' parameter."
                        },
                        "time_period": {
                            "type": "string",
                            "enum": ["day", "week", "month", "year"],
                            "description": "Filters search results to a specific time range. Use this to find recent or timely information. If omitted, no time filter is applied."
                        },
                        "response_format": {
                            "type": "string",
                            "enum": ["json", "xml"],
                            "default": "json",
                            "description": "The format of the response data. Defaults to 'json'."
                        }
                    },
                    "required": ["query"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
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
        // 1. Extract parameters
        let provider_param = params
            .get("provider")
            .and_then(|p| p.as_str())
            .map(String::from);
        let query = Self::extract_keywords(&params)?;
        let desired_count = params["number"].as_u64().unwrap_or(5).min(30).max(1) as usize;
        let time_period = params["time_period"].as_str().unwrap_or("");
        let response_format = params["response_format"].as_str().unwrap_or("json");

        let period = match time_period {
            "day" => Some(crate::search::SearchPeriod::Day),
            "week" => Some(crate::search::SearchPeriod::Week),
            "month" => Some(crate::search::SearchPeriod::Month),
            "year" => Some(crate::search::SearchPeriod::Year),
            _ => None,
        };

        // 2. Setup for pagination loop
        let (searcher, search_provider_name) = self.create_searcher(provider_param)?;
        let mut final_results: Vec<crate::search::SearchResult> = Vec::with_capacity(desired_count);
        let mut current_page = 1;
        const MAX_PAGES_TO_FETCH: u64 = 3; // Limit to prevent excessive requests
        const PAGE_SIZE: u64 = 10; // Most engines default to 10

        // 3. Pagination-Pipeline Loop
        while final_results.len() < desired_count && current_page <= MAX_PAGES_TO_FETCH {
            let search_params = json!({
                "query": query.clone(),
                "count": PAGE_SIZE,
                "period": period,
                "page": current_page,
            });

            let raw_results = searcher.search(&search_params).await.map_err(|e| {
                log::error!("Failed to fetch search page {}: {}", current_page, e);
                ToolError::ExecutionFailed(e.to_string())
            })?;

            if raw_results.is_empty() {
                break; // No more results from the search engine
            }

            for mut result in raw_results {
                // a. Resolve URL
                let resolved_url = if search_provider_name == SearchProviderName::Bing {
                    decode_bing_url(&result.url).unwrap_or(result.url.clone())
                } else if search_provider_name == SearchProviderName::So
                    || search_provider_name == SearchProviderName::Sogou
                {
                    get_meta_refresh_url(&result.url)
                        .await
                        .unwrap_or(result.url.clone())
                } else {
                    result.url.clone()
                };
                result.url = resolved_url;

                // b. Filter
                let Ok(parsed_url) = Url::parse(&result.url) else {
                    continue;
                };

                // Domain filter
                let host = parsed_url.host_str().unwrap_or_default();
                let parts: Vec<&str> = host.split('.').collect();
                if (0..parts.len().saturating_sub(1)).any(|i| {
                    let domain_to_check = parts[i..].join(".");
                    VIDEO_AND_IMAGE_DOMAINS.contains(domain_to_check.as_str())
                }) {
                    continue;
                }

                // Extension filter
                let path = parsed_url.path();
                if RESTRICTED_EXTENSIONS
                    .iter()
                    .any(|ext| path.to_lowercase().ends_with(ext))
                {
                    continue;
                }

                // c. Add to final list
                final_results.push(result);
                if final_results.len() >= desired_count {
                    break; // We have enough results
                }
            }
            current_page += 1;
        }

        // 4. Final processing
        let results_with_id = final_results
            .iter()
            .enumerate()
            .map(|(index, r)| {
                let mut sr = r.clone();
                sr.id = index + 1;
                sr
            })
            .collect::<Vec<_>>();

        let result_string = if results_with_id.is_empty() {
            match response_format {
                "xml" => "<search-results></search-results>".to_string(),
                _ => "[]".to_string(), // default to JSON
            }
        } else {
            match response_format {
                "xml" => {
                    let xml_items: Vec<String> =
                        results_with_id.iter().map(|r| r.to_string()).collect();
                    format!(
                        "<search-results>\n{}\n</search-results>",
                        xml_items.join("\n")
                    )
                }
                _ => serde_json::to_string(&results_with_id).unwrap_or_default(), // default to JSON
            }
        };

        Ok(ToolCallResult::success(
            Some(result_string),
            Some(json!(results_with_id)),
        ))
    }
}
