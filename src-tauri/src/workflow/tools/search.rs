use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    http::crawler::{Crawler, SearchProvider},
    workflow::{
        context::Context,
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

pub struct Search {
    // chatspeed bot server url
    chatspeedbot_server: String,
}

impl Search {
    pub fn new(chatspeedbot_server: String) -> Self {
        Self {
            chatspeedbot_server,
        }
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
    /// * `Result<String, WorkflowError>` - The encoded keywords
    fn extract_keywords(params: &Value) -> Result<String, WorkflowError> {
        match &params["kw"] {
            Value::String(s) => {
                let s = s.trim();
                if s.is_empty() {
                    return Err(WorkflowError::FunctionParamError(
                        "kw must not be empty".to_string(),
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
                    return Err(WorkflowError::FunctionParamError(
                        "kw must be a non-empty array or string".to_string(),
                    ));
                }
                let joined_keywords = keywords.join(" "); // join with space
                Ok(joined_keywords)
            }
            _ => Err(WorkflowError::FunctionParamError(
                "kw must be a string or array".to_string(),
            )),
        }
    }
}

#[async_trait]
impl FunctionDefinition for Search {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "search"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::CHP
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Perform a search query"
    }

    /// Returns the function calling spec.
    fn function_calling_spec(&self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "parameters": {
                "type": "object",
                "properties": {
                    "provider": {
                        "type": "string",
                        "enum": ["google", "google_news", "baidu", "baidu_news", "bing"],
                        "description": "The search provider to use. Supported providers: google, google_news, baidu, baidu_news, bing."
                    },
                    "kw": {
                        "type": "string",
                        "description": "The keyword to search for."
                    },
                    "number": {
                        "type": "integer",
                        "default": 10,
                        "description": "The number of results to return. Default is 10."
                    },
                    "page": {
                        "type": "integer",
                        "default": 1,
                        "description": "The page number of the results. Default is 1."
                    },
                    "resolve_baidu_links": {
                        "type": "boolean",
                        "default": true,
                        "description": "Whether to resolve Baidu links. Recommended to enable for better result filtering."
                    }
                },
                "required": ["provider", "kw"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "results": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": {
                                    "type": "string",
                                    "description": "The title of the search result."
                                },
                                "url": {
                                    "type": "string",
                                    "description": "The URL of the search result."
                                },
                                "summary": {
                                    "type": "string",
                                    "description": "A brief description of the search result.",
                                    "nullable": true
                                },
                                "sitename": {
                                    "type": "string",
                                    "description": "The site name of the search result.",
                                    "nullable": true
                                },
                                "publish_date": {
                                    "type": "string",
                                    "description": "The publish date of the search result.",
                                    "nullable": true
                                }
                            }
                        },
                        "description": "The list of search results."
                    },
                    "error": {
                        "type": "string",
                        "description": "An error message if the search failed."
                    }
                },
                "description": "The response containing search results or an error message."
            }
        })
    }

    /// Executes the function with the given parameters and context.
    ///
    /// # Arguments
    /// * `params` - The parameters of the function.
    /// * `_context` - The context of the function.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the result of the function execution.
    async fn execute(&self, params: Value, _context: &Context) -> FunctionResult {
        let provider: SearchProvider = params["provider"]
            .as_str()
            .ok_or_else(|| {
                WorkflowError::FunctionParamError("provider must be a string".to_string())
            })?
            .try_into()?;

        let kw = Self::extract_keywords(&params)?;

        if kw.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                "kw must not be empty".to_string(),
            ));
        }

        let number = params["number"].as_i64().unwrap_or(10);
        let page = params["page"].as_i64().unwrap_or(1);
        let resolve_baidu_links = params["resolve_baidu_links"].as_bool().unwrap_or(true);

        let crawler = Crawler::new(self.chatspeedbot_server.clone());
        let results = crawler
            .search(
                provider,
                &[&kw],
                Some(page),
                Some(number),
                resolve_baidu_links,
            )
            .await
            .map_err(|e| WorkflowError::Execution(e.to_string()))?;
        Ok(json!(results))
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
                    "kw": "五粮液",
                    "number": 10,
                    "page": 1
            });
            let result = search.execute(params, &Context::new()).await;
            println!("{:?}", result);
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_search_error() {
        let search = Search::new("http://127.0.0.1:12321".to_string());
        let params = json!({
            "provider": "duckduckgo",
            "kw": "rust生命周期",
            "number": 10,
            "page": 1
        });
        let result = search.execute(params, &Context::new()).await;
        assert!(!result.is_ok());
    }
}
