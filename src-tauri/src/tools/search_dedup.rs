use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    http::chp::SearchResult,
    libs::dedup::dedup_and_rank_results,
    tools::{error::ToolError, ToolDefinition, ToolResult},
};

pub struct SearchDedup;

impl SearchDedup {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolDefinition for SearchDedup {
    fn name(&self) -> &str {
        "search_dedup"
    }

    fn description(&self) -> &str {
        "Deduplicate and rank search results based on URL and semantic similarity"
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
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
            }),
            output_schema: None,
            disabled: false,
        }
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
    async fn call(&self, params: Value) -> ToolResult {
        let results: Vec<SearchResult> = serde_json::from_value(params["results"].clone())
            .map_err(|e| ToolError::Serialization(e.to_string()))?;
        let query = params["query"]
            .as_str()
            .ok_or_else(|| {
                ToolError::FunctionParamError(t!("tools.query_must_not_be_empty").to_string())
            })?
            .to_string();
        if query.is_empty() {
            return Err(ToolError::FunctionParamError(
                t!("tools.query_must_not_be_empty").to_string(),
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

        let result = tool.call(params).await;
        log::debug!("Deduplicated results: {:#?}", result);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 2);

        // 测试空查询
        let params = json!({
            "results": results,
            "query": ""
        });

        let result = tool.call(params).await;
        assert!(result.is_err());

        // 测试空结果
        let params = json!({
            "results": [],
            "query": "Rust"
        });

        let result = tool.call(params).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_array().unwrap().len(), 0);

        // 测试无效参数
        let params = json!({
            "results": "invalid",
            "query": "Rust"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
    }
}
