use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    http::crawler::Crawler,
    workflow::{
        context::Context,
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

/// A function implementation for fetching data from a remote URL.
///
/// This function supports HTTP and HTTPS protocols, and can handle
/// various types of requests (GET, POST, etc.) with custom headers and body.
pub struct Fetch {
    // chatspeed bot server url
    chatspeedbot_server: String,
}

impl Fetch {
    pub fn new(chatspeedbot_server: String) -> Self {
        Self {
            chatspeedbot_server,
        }
    }
}

#[async_trait]
impl FunctionDefinition for Fetch {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "fetch"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::CHP
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Fetches data from a remote URL using HTTP or HTTPS."
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
                    "url": {"type": "string", "description": "The URL to fetch data from"},
                },
                "required": ["url"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "title": {
                    "type": "string",
                    "description": "The title of the fetched content."
                    },
                    "url": {
                    "type": "string",
                    "description": "The URL of the fetched content."
                    },
                    "content": {
                    "type": "string",
                    "description": "The main content of the fetched page."
                    }
                },
                "description": "The response containing the fetched content."
            }
        })
    }

    /// Executes the fetch function.
    ///
    /// # Arguments
    /// * `params` - The parameters to pass to the function, including the URL.
    /// * `context` - The context in which the function is executed.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    async fn execute(&self, params: Value, _context: &Context) -> FunctionResult {
        // Get the URL from the parameters
        let url = params["url"]
            .as_str()
            .ok_or_else(|| WorkflowError::FunctionParamError("url must be a string".to_string()))?;

        // Check if the URL is empty
        if url.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                "url must not be empty".to_string(),
            ));
        }

        // Check if the URL starts with http:// or https://
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(WorkflowError::FunctionParamError(
                "url must start with http:// or https://".to_string(),
            ));
        }

        // Create a new crawler instance
        let crawler = Crawler::new(self.chatspeedbot_server.clone());

        // Fetch the data from the URL
        let results = crawler
            .fetch(url)
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
        let search = Fetch::new("http://127.0.0.1:12321".to_string());
        let urls = [
            "https://en.wikipedia.org/wiki/Schutzstaffel",
            "https://www.ssa.gov/myaccount/",
            "https://hznews.hangzhou.com.cn/shehui/content/2025-03/04/content_8872051.htm",
        ];
        for url in urls {
            let result = search.execute(json!({"url": url}), &Context::new()).await;
            println!("{:?}", result);
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_fetch_invalid_url() {
        let search = Fetch::new("http://127.0.0.1:12321".to_string());
        let params = json!({
            "url": "google.com",
        });
        let result = search.execute(params, &Context::new()).await;
        assert!(!result.is_ok());
    }
}
