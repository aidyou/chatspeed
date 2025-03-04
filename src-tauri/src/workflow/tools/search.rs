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
    fn name(&self) -> &str {
        "Search"
    }

    fn function_type(&self) -> FunctionType {
        FunctionType::CHP
    }

    fn description(&self) -> &str {
        "Get Search Results from a Provider"
    }

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

        let crawler = Crawler::new(self.chatspeedbot_server.clone());
        let results = crawler
            .search(provider, &[&kw], Some(page), Some(number))
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
