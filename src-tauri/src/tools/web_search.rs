use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{str::FromStr, sync::Arc};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    db::MainStore,
    search::{
        GoogleSearch, SearchFactory, SearchPeriod, SearchProvider, SearchProviderName,
        SerperSearch, TavilySearch,
    },
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolDefinition},
};

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
    // chatspeed bot server url
    searcher: SearchFactory,
}

impl WebSearch {
    pub fn new(
        search_engine: String,
        main_store: Arc<std::sync::RwLock<MainStore>>,
    ) -> Result<Arc<Self>, ToolError> {
        let store = main_store.read().map_err(|e| {
            ToolError::Store(t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())
        })?;

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
        let auth = Self::get_and_check_auth(&search_engine, main_store.clone())?;

        let provider_name = SearchProviderName::from_str(&search_engine)
            .map_err(|e| ToolError::Initialization(e))?;
        let searcher = match provider_name {
            SearchProviderName::Google => SearchFactory::Google(
                GoogleSearch::new(auth.api_key, auth.cx.unwrap_or_default(), proxy)
                    .map_err(|e| ToolError::Initialization(e.to_string()))?,
            ),
            SearchProviderName::Tavily => SearchFactory::Tavily(
                TavilySearch::new(auth.api_key, proxy)
                    .map_err(|e| ToolError::Initialization(e.to_string()))?,
            ),
            SearchProviderName::Serper => SearchFactory::Serper(
                SerperSearch::new(auth.api_key, proxy)
                    .map_err(|e| ToolError::Initialization(e.to_string()))?,
            ),
        };

        Ok(Arc::new(Self { searcher }))
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
            } // _ => {
              //     return Err(ToolError::Initialization(
              //         t!(
              //             "tools.search.provider_no_support",
              //             name = provider_name.to_string()
              //         )
              //         .to_string(),
              //     ))
              // }
        };
        Ok(auth)
    }

    /// Extract keywords from params
    ///
    /// # Arguments
    /// * `params` - The parameters for the function, which must contain a "kw" field
    ///             and it must be a string or array of strings.
    ///             When the kw is array, it will be joined by space with double quotes.
    ///             `["foo", "bar"]` -> `"foo" "bar"`
    ///
    /// # Returns
    /// * `Result<String, ToolError>` - The encoded keywords
    pub fn extract_keywords(params: &Value) -> Result<String, ToolError> {
        match &params["kw"] {
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
}

#[async_trait]
impl ToolDefinition for WebSearch {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "webSearch"
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "A powerful web search tool that performs a web search query"
    }

    /// Returns the function calling spec.
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                    "type": "object",
                    "properties": {
                        "kw": {
                            "type": "string",
                            "description": "Search keyword"
                        },
                        "number": {
                            "type": "integer",
                            "default": 10,
                            "description": "Results count"
                        },
                        "page": {
                            "type": "integer",
                            "default": 1,
                            "description": "Page number"
                        },
                        "time_period": {
                            "type": "string",
                            "enum": ["day", "week", "month", "year"],
                            "description": "Time range filter"
                        }
                    },
                    "required": ["provider", "kw"]
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
        let kw = Self::extract_keywords(&params)?;
        if kw.is_empty() {
            return Err(ToolError::FunctionParamError(
                t!("tools.keyword_must_be_non_empty").to_string(),
            ));
        }

        let number = params["number"].as_i64().unwrap_or(10);
        let page = params["page"].as_i64().unwrap_or(1);
        // let resolve_baidu_links = params["resolve_baidu_links"].as_bool().unwrap_or(true);
        let time_period = params["time_period"].as_str().unwrap_or("");

        // Convert time_period string to SearchPeriod enum
        let period = match time_period {
            "day" => Some(SearchPeriod::Day),
            "week" => Some(SearchPeriod::Week),
            "month" => Some(SearchPeriod::Month),
            "year" => Some(SearchPeriod::Year),
            _ => None,
        };

        // Create search parameters
        let search_params = json!({
            "query": kw,
            "count": number,
            "period": period,
            "page": page,
        });

        // Execute search using the factory
        let results = self.searcher.search(&search_params).await.map_err(|e| {
            ToolError::Execution(
                t!(
                    "tools.search.web_search_failed",
                    provider = self.searcher.to_string(),
                    keyword = kw,
                    details = e.to_string()
                )
                .to_string(),
            )
        })?;

        Ok(ToolCallResult::success(
            Some(
                results
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join("\n===============\n"),
            ),
            Some(json!({
                "query": &search_params,
                "results": results
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::db;

    use super::*;
    use serde_json::json;

    fn get_store() -> MainStore {
        let db_path = {
            let dev_dir = &*crate::STORE_DIR.read();
            dev_dir.join("chatspeed.db")
        };

        // Setup language
        MainStore::new(db_path)
            .map_err(|e| db::StoreError::IoError(e.to_string()))
            .expect("Failed to create main store")
    }

    #[tokio::test]
    async fn test_search() {
        let store = Arc::new(std::sync::RwLock::new(get_store()));
        for provider in vec!["google", "tavily", "serper"] {
            let search = WebSearch::new(provider.to_string(), store.clone()).unwrap();
            let params = json!({
                    "kw": "deepseek",
                    "number": 10,
                    "page": 1,
                    "time_period": "week"
            });
            let result = search.call(params).await.unwrap();
            assert!(result.content.is_some());
            println!("{}", result.content.as_ref().unwrap());
        }
    }

    #[tokio::test]
    async fn test_search_error() {
        let store = Arc::new(std::sync::RwLock::new(get_store()));
        let search = WebSearch::new("google".to_string(), store.clone()).unwrap();
        let params = json!({
            "kw": "rust lifetime",
            "number": 10,
            "page": 1
        });
        let result = search.call(params).await.unwrap();
        assert!(result.content.is_some());
        println!("{}", result.content.as_ref().unwrap());
    }
}
