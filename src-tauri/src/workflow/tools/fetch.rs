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
    fn name(&self) -> &str {
        "Fetch"
    }

    fn function_type(&self) -> FunctionType {
        FunctionType::CHP
    }

    fn description(&self) -> &str {
        "Crawl content from a url"
    }

    async fn execute(&self, params: Value, _context: &Context) -> FunctionResult {
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

        let crawler = Crawler::new(self.chatspeedbot_server.clone());
        let results = crawler
            .fetch(url)
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
