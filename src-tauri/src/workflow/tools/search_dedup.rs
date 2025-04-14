use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    http::chp::SearchResult,
    libs::dedup::dedup_and_rank_results,
    workflow::{
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

pub struct SearchDedup;

impl SearchDedup {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FunctionDefinition for SearchDedup {
    fn name(&self) -> &str {
        "search_dedup"
    }

    fn function_type(&self) -> FunctionType {
        FunctionType::Native
    }

    fn description(&self) -> &str {
        "Deduplicate and rank search results based on URL and semantic similarity"
    }

    fn function_calling_spec(&self) -> serde_json::Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "parameters": {
                "type": "object",
                "properties": {
                    "results": {
                        "type": "array",
                        "description": "Search results to deduplicate"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["results", "query"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "results": {
                        "type": "array",
                        "description": "Deduplicated results"
                    }
                },
                "description": "Processed search results"
            }
        })
    }

    /// Executes the search deduplication and ranking operation.
    ///
    /// # Arguments
    /// * `params` - The parameters for the operation, containing:
    ///   - `results`: A list of search results to deduplicate.
    ///   - `query`: The search query string.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the deduplicated and ranked search results.
    async fn execute(&self, params: Value) -> FunctionResult {
        let results: Vec<SearchResult> = serde_json::from_value(params["results"].clone())?;
        let query = params["query"]
            .as_str()
            .ok_or_else(|| {
                WorkflowError::FunctionParamError(
                    t!("workflow.query_must_not_be_empty").to_string(),
                )
            })?
            .to_string();
        if query.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                t!("workflow.query_must_not_be_empty").to_string(),
            ));
        }

        let deduped_results = dedup_and_rank_results(results, &query);
        Ok(json!(deduped_results))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_search_dedup_tool() {
        let tool = SearchDedup;

        // 测试正常情况
        let results = json!([
            {
                "title": "Rust Programming",
                "url": "https://rust-lang.org",
                "snippet": "A systems programming language"
            },
            {
                "title": "Rust Lang",
                "url": "https://rust-lang.org",
                "snippet": "The Rust programming language"
            },
            {
                "title": "Rust Book",
                "url": "https://doc.rust-lang.org/book/",
                "snippet": "The Rust programming language book"
            }
        ]);

        let params = json!({
            "results": results,
            "query": "Rust programming language"
        });

        let result = tool.execute(params).await;
        log::debug!("Deduplicated results: {:#?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 2);

        // 测试空查询
        let params = json!({
            "results": results,
            "query": ""
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());

        // 测试空结果
        let params = json!({
            "results": [],
            "query": "Rust"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 0);

        // 测试无效参数
        let params = json!({
            "results": "invalid",
            "query": "Rust"
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());
    }
}
