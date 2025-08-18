use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    http::chp::Chp,
    search::SearchProviderName,
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolDefinition},
};

pub struct Search {
    // chatspeed bot server url
    chp_server: String,
}

impl Search {
    pub fn new(chp_server: String) -> Self {
        Self { chp_server }
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
impl ToolDefinition for Search {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "web_search"
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Perform a search query"
    }

    /// Returns the function calling spec.
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                    "type": "object",
                    "properties": {
                        "provider": {
                            "type": "string",
                            "enum": ["google", "google_news", "baidu", "baidu_news", "bing"],
                            "description": "Search provider"
                        },
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
                        },
                        "resolve_baidu_links": {
                            "type": "boolean",
                            "default": true,
                            "description": "Resolve Baidu links"
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
        let provider: SearchProviderName = params["provider"]
            .as_str()
            .ok_or_else(|| {
                ToolError::FunctionParamError(t!("tools.provider_must_be_string").to_string())
            })?
            .try_into()
            .map_err(|e_str| {
                ToolError::FunctionParamError(
                    t!(
                        "tools.invalid_search_provider",
                        provider_name = params["provider"].as_str().unwrap_or("unknown"),
                        details = e_str
                    )
                    .to_string(),
                )
            })?;

        let kw = Self::extract_keywords(&params)?;

        if kw.is_empty() {
            return Err(ToolError::FunctionParamError(
                t!("tools.keyword_must_be_non_empty").to_string(),
            ));
        }

        let number = params["number"].as_i64().unwrap_or(10);
        let page = params["page"].as_i64().unwrap_or(1);
        let resolve_baidu_links = params["resolve_baidu_links"].as_bool().unwrap_or(true);
        let time_period = params["time_period"].as_str().unwrap_or("");

        let crawler = Chp::new(self.chp_server.clone(), None);
        let results = crawler
            .web_search(
                provider.clone(),
                &[&kw],
                Some(page),
                Some(number),
                Some(time_period),
                resolve_baidu_links,
            )
            .await
            .map_err(|e_str| {
                ToolError::Execution(
                    t!(
                        "tools.web_search_failed",
                        provider = provider.to_string(),
                        keyword = kw,
                        details = e_str
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
            Some(json!(results)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_search() {
        let search = Search::new("http://127.0.0.1:12321".to_string());
        let provider = ["google", "google_news", "bing", "baidu", "baidu_news"];
        for p in provider {
            let params = json!({
                    "provider": p,
                    "kw": "deepseek",
                    "number": 10,
                    "page": 1,
                    "time_period": "week"
            });
            let result = search.call(params).await;
            println!("{:?}", result);
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_search_error() {
        let search = Search::new("http://127.0.0.1:12321".to_string());
        let params = json!({
            "provider": "duckduckgo",
            "kw": "rust lifetime",
            "number": 10,
            "page": 1
        });
        let result = search.call(params).await;
        println!("{:?}", result);
        assert!(!result.is_ok());
    }
}
