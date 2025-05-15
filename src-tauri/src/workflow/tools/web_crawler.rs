use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    http::chp::Chp,
    workflow::{
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

/// A function implementation for fetching data from a remote URL.
///
/// This function supports HTTP and HTTPS protocols, and can handle
/// various types of requests (GET, POST, etc.) with custom headers and body.
pub struct Crawler {
    // chatspeed bot server url
    chp_server: String,
}

impl Crawler {
    pub fn new(chp_server: String) -> Self {
        Self { chp_server }
    }
}

#[async_trait]
impl FunctionDefinition for Crawler {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "web_crawler"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::MCP
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Crawl data from a remote URL using HTTP or HTTPS."
    }

    /// Returns the function calling specification in JSON format.
    ///
    /// This method provides detailed information about the function
    /// in a format compatible with function calling APIs.
    ///
    /// # Returns
    /// * `Value` - The function specification in JSON format.
    fn function_calling_spec(&self) -> Value {
        json!({
           "name": self.name(),
           "description": self.description(),
           "parameters": {
               "type": "object",
               "properties": {
                   "url": {"type": "string", "description": "URL to fetch"},
               },
               "required": ["url"]
           },
           "responses": {
               "type": "object",
               "properties": {
                   "title": {
                       "type": "string",
                       "description": "Page title"
                   },
                   "url": {
                       "type": "string",
                       "description": "Source URL"
                   },
                   "content": {
                       "type": "string",
                       "description": "Page content"
                   }
               },
               "description": "Fetched web content"
           }
        })
    }

    /// Executes the fetch function.
    ///
    /// # Arguments
    /// * `params` - The parameters to pass to the function, including the URL.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    async fn execute(&self, params: Value) -> FunctionResult {
        // Get the URL from the parameters
        let url = params["url"]
            .as_str()
            .ok_or_else(|| WorkflowError::FunctionParamError("url must be a string".to_string()))?;

        // Check if the URL is empty
        if url.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                t!("workflow.url_must_not_be_empty").to_string(),
            ));
        }

        // Check if the URL starts with http:// or https://
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(WorkflowError::FunctionParamError(
                t!("workflow.url_invalid", url = url).to_string(),
            ));
        }

        let format = params["format"].as_str().unwrap_or("html").to_string();

        // Create a new crawler instance
        let crawler = Chp::new(self.chp_server.clone(), None);

        // Fetch the data from the URL
        let results = crawler
            .web_crawler(url, Some(json!({"format": format})))
            .await
            .map_err(|e| WorkflowError::Execution(e.to_string()))?;

        // Return the results as JSON
        Ok(json!(results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_fetch() {
        let search = Crawler::new("http://127.0.0.1:12321".to_string());
        let urls = [
            "https://en.wikipedia.org/wiki/Schutzstaffel",
            "https://www.ssa.gov/myaccount/",
            "https://hznews.hangzhou.com.cn/shehui/content/2025-03/04/content_8872051.htm",
        ];
        for url in urls {
            let result = search.execute(json!({"url": url})).await;
            println!("{:?}", result);
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_fetch_invalid_url() {
        let search = Crawler::new("http://127.0.0.1:12321".to_string());
        let params = json!({
            "url": "google.com",
        });
        let result = search.execute(params).await;
        assert!(!result.is_ok());
    }
}
