use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize}; // 导入 Deserialize
use serde_json::{json, Value};

use crate::http::client::HttpClient; // 导入 HttpClient
use crate::http::types::HttpConfig; // 导入 HttpConfig

use super::search::{SearchParams, SearchPeriod, SearchProvider, SearchResult};

const GOOGLE_API_URL: &str = "https://www.googleapis.com/customsearch/v1";

#[derive(Debug, Serialize)]
struct GoogleSearchRequest<'a> {
    q: &'a str,
    key: &'a str,
    cx: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    num: Option<u32>, // max_results
    #[serde(skip_serializing_if = "Option::is_none")]
    hl: Option<String>, // language
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<String>, // time_range, e.g., "date:r:20230101:20231231"
}

impl From<GoogleSearchRequest<'_>> for String {
    fn from(request: GoogleSearchRequest) -> Self {
        // Serialization of this struct is not expected to fail.
        serde_json::to_string(&request).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)] // 添加 Deserialize
struct GoogleItem {
    title: String,
    link: String,
    snippet: String,
}

#[derive(Debug, Deserialize)] // 添加 Deserialize
struct GoogleSearchResponse {
    items: Option<Vec<GoogleItem>>,
}

pub struct GoogleSearch {
    api_key: String,
    cx: String,
    http_client: HttpClient, // 添加 HttpClient 字段
}

impl GoogleSearch {
    pub fn new(api_key: String, cx: String, proxy: Option<String>) -> Result<Self> {
        let http_client = HttpClient::new(proxy)?;
        Ok(Self {
            api_key,
            cx,
            http_client,
        })
    }
}

#[async_trait]
impl SearchProvider for GoogleSearch {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        let search_params = SearchParams::try_from(params)?;

        let sort_param = search_params.period.as_ref().map(|p| match p {
            SearchPeriod::Hour => "date:h".to_string(), // Google Custom Search API doesn't directly support "hour" for sorting, using a general date sort.
            SearchPeriod::Day => "date:d".to_string(),
            SearchPeriod::Week => "date:w".to_string(),
            SearchPeriod::Month => "date:m".to_string(),
            SearchPeriod::Year => "date:y".to_string(),
        });

        let request_body = GoogleSearchRequest {
            q: &search_params.query,
            key: &self.api_key,
            cx: &self.cx,
            num: search_params.count,
            hl: search_params.language.clone(),
            sort: sort_param,
        };

        // 构建请求配置
        let config = HttpConfig::get(GOOGLE_API_URL)
            .query(json!(&request_body))
            .header("Content-Type", "application/json");

        // 发送请求
        let response = self.http_client.send_request(config).await?;

        if !response.is_success() {
            return Err(anyhow!(
                "Google Custom Search API request failed with status {}: {}",
                response.status,
                response.error.unwrap_or_else(|| response
                    .body
                    .as_deref()
                    .unwrap_or("No error details")
                    .to_string()),
            ));
        }

        let body = response
            .body
            .context("Response body was empty, but a successful status was returned.")?;

        let google_response: GoogleSearchResponse = serde_json::from_str(&body)
            .context("Failed to deserialize Google response from JSON body")?;

        let results = google_response
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.link,
                snippet: Some(item.snippet), // Google API 的 snippet 字段是 String，这里转换为 Option<String>
                score: None,                 // Google Custom Search API 结果中没有直接的 score 字段
                content: None,               // Google Custom Search API 不直接提供完整内容
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::env;

    use super::*;

    #[tokio::test]
    async fn test_google_search() {
        let api_key = match env::var("GOOGLE_API_KEY") {
            Ok(key) => key,
            Err(_) => {
                println!(
                    "Skipping test_google_search: GOOGLE_API_KEY environment variable not set."
                );
                return;
            }
        };

        let cx = match env::var("GOOGLE_CSE_ID") {
            Ok(id) => id,
            Err(_) => {
                println!(
                    "Skipping test_google_search: GOOGLE_CSE_ID environment variable not set."
                );
                return;
            }
        };

        let client =
            GoogleSearch::new(api_key, cx, None).expect("Failed to create GoogleSearch client");

        let query = "what is rust programming language?";
        let search_result = client.search(&json!({ "query": query, "count": 5 })).await;

        assert!(
            search_result.is_ok(),
            "Search request failed: {:?}",
            search_result.err()
        );

        let response = search_result.unwrap();
        assert!(!response.is_empty(), "Search returned no results.");

        if let Some(first_result) = response.first() {
            println!("Top search result: {:?}", first_result);
        }
    }
}
