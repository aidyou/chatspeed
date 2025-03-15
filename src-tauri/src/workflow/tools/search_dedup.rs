use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    http::crawler::SearchResult,
    libs::dedup::dedup_and_rank_results,
    workflow::{
        context::Context,
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

pub struct SearchDedupTool;

#[async_trait]
impl FunctionDefinition for SearchDedupTool {
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
            "name": "search_dedup",
            "description": "Deduplicate and rank search results",
            "parameters": {
                "type": "object",
                "properties": {
                    "results": {
                        "type": "array",
                        "description": "List of search results to deduplicate"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query string"
                    }
                },
                "required": ["results", "query"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "results": {
                        "type": "array",
                        "description": "Deduplicated and ranked search results"
                    }
                }
            }
        })
    }

    /// Executes the search deduplication and ranking operation.
    ///
    /// # Arguments
    /// * `params` - The parameters for the operation, containing:
    ///   - `results`: A list of search results to deduplicate.
    ///   - `query`: The search query string.
    /// * `_context` - The context of the operation.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the deduplicated and ranked search results.
    async fn execute(&self, params: Value, _context: &Context) -> FunctionResult {
        let results: Vec<SearchResult> = serde_json::from_value(params["results"].clone())?;
        let query = params["query"]
            .as_str()
            .ok_or_else(|| WorkflowError::FunctionParamError("query must be a string".to_string()))?
            .to_string();
        if query.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                "query must not be empty".to_string(),
            ));
        }

        let deduped_results = dedup_and_rank_results(results, &query);
        Ok(json!(deduped_results))
    }
}
#[cfg(test)]
mod tests {
    use crate::logger::setup_test_logger;

    use super::*;
    use serde_json::json;

    #[small_ctor::ctor]
    unsafe fn init() {
        let _ = setup_test_logger();
    }

    #[tokio::test]
    async fn test_search_dedup_tool() {
        let tool = SearchDedupTool;
        let context = Context::new();

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

        let result = tool.execute(params, &context).await;
        log::debug!("Deduplicated results: {:#?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 2);

        // 测试空查询
        let params = json!({
            "results": results,
            "query": ""
        });

        let result = tool.execute(params, &context).await;
        assert!(result.is_err());

        // 测试空结果
        let params = json!({
            "results": [],
            "query": "Rust"
        });

        let result = tool.execute(params, &context).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 0);

        // 测试无效参数
        let params = json!({
            "results": "invalid",
            "query": "Rust"
        });

        let result = tool.execute(params, &context).await;
        assert!(result.is_err());
    }
}
