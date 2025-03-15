use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    http::{client::HttpClient, types::HttpConfig},
    workflow::{
        context::Context,
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

/// A function that sends an HTTP request.
pub struct Request {}

impl Request {
    /// Creates a new instance of the `Request` function.
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl FunctionDefinition for Request {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "request"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::Http
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Execute a HTTP request"
    }

    /// Returns the function calling spec.
    fn function_calling_spec(&self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to send the request to."
                    },
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST"],
                        "description": "The HTTP method to use. Supported methods: GET, POST."
                    },
                    "headers": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                        "description": "The headers to include in the request. Example: {\"User-Agent\": \"Mozilla/5.0\", \"Accept\": \"application/json\"}."
                    },
                    "body": {
                        "type": "string",
                        "description": "The body of the request. Required for POST requests."
                    }
                },
                "required": ["url", "method"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "headers": {
                        "type": "object",
                        "additionalProperties": {"type": "string"},
                        "description": "The headers of the HTTP response."
                    },
                    "body": {
                        "type": "string",
                        "description": "The body of the HTTP response."
                    }
                },
                "description": "The response containing the HTTP headers and body."
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
        // The URL of the request.
        let url = params["url"]
            .as_str()
            .ok_or_else(|| WorkflowError::FunctionParamError("url must be a string".to_string()))?;
        if url.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                "url must not be empty".to_string(),
            ));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(WorkflowError::FunctionParamError(
                "url must start with http:// or https://".to_string(),
            ));
        }
        let config = HttpConfig::get(url).async_request();

        // Parse headers
        params["headers"].as_object().map(|obj| {
            obj.iter().for_each(|(k, v)| {
                if let Some(v_str) = v.as_str() {
                    config.clone().header(k, v_str);
                }
            })
        });

        let client = HttpClient::new_with_config(&config)
            .map_err(|e| WorkflowError::Execution(e.to_string()))?;

        let response = client
            .send_request(config)
            .await
            .map_err(|e| WorkflowError::Execution(e.to_string()))?;

        if let Some(err) = response.error {
            return Err(WorkflowError::Execution(err));
        } else if response.status != 200 {
            return Err(WorkflowError::Execution(
                t!("http.status_not_ok", status = response.status).to_string(),
            ));
        }

        let body = response.body.ok_or(WorkflowError::Execution(
            t!("http.missing_response_body").to_string(),
        ))?;

        Ok(json!({
            "headers": response.headers,
            "body": body,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_fetch() {
        let search = Request::new();
        let urls = [
            "https://en.wikipedia.org/wiki/Schutzstaffel",
            "https://www.ssa.gov/myaccount/",
            "https://hznews.hangzhou.com.cn/shehui/content/2025-03/04/content_8872051.htm",
        ];
        for url in urls {
            let result = search
                .execute(
                    json!({"url": url,"headers": {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/87.0.4280.88 Safari/537.36 Edg/87.0.664.66"}}),
                    &Context::new(),
                )
                .await;
            println!("{:?}", result);
            assert!(result.is_ok());
        }
    }
}
