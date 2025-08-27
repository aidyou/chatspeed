use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{client::HttpClient, types::HttpConfig};

use super::search::{SearchParams, SearchPeriod, SearchProvider, SearchResult};

const TAVILY_API_URL: &str = "https://api.tavily.com/search";

/// https://app.tavily.com/playground
#[derive(Debug, Serialize)]
struct TavilySearchRequest<'a> {
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_answer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_raw_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

impl<'a> From<TavilySearchRequest<'a>> for String {
    fn from(request: TavilySearchRequest<'a>) -> Self {
        // Serialization of this struct is not expected to fail.
        serde_json::to_string(&request).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
    score: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct TavilySearchResponse {
    results: Vec<TavilyResult>,
    #[serde(rename = "response_time")]
    _response_time: Option<f64>,
}

pub struct TavilySearch {
    api_key: String,
    http_client: HttpClient,
}

impl TavilySearch {
    pub fn new(api_key: String, proxy: Option<String>) -> Result<Self> {
        let http_client = HttpClient::new(proxy)?;
        Ok(Self {
            api_key,
            http_client,
        })
    }
}

#[async_trait]
impl SearchProvider for TavilySearch {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        let search_params = SearchParams::try_from(params)?;

        // 转换时间范围参数
        let time_range = search_params.period.as_ref().map(|p| match p {
            SearchPeriod::Hour => "hour".to_string(),
            SearchPeriod::Day => "day".to_string(),
            SearchPeriod::Week => "week".to_string(),
            SearchPeriod::Month => "month".to_string(),
            SearchPeriod::Year => "year".to_string(),
        });

        let request_body = TavilySearchRequest {
            query: &search_params.query,
            max_results: search_params.count.map(|c| c as usize),
            topic: Some("general".to_string()), // 默认使用通用搜索
            search_depth: Some("basic".to_string()),
            include_answer: Some(false),
            include_raw_content: Some(true), // 添加此行以获取完整内容
            time_range,
            country: None, // Tavily API 文档中 country 字段是可选的，这里暂时不处理
            language: search_params.language.clone(),
        };

        let config = HttpConfig::post(TAVILY_API_URL, request_body)
            .authorization(&self.api_key)
            .json_request();

        // Send the request using the http client.
        let response = self.http_client.send_request(config).await?;

        if !response.is_success() {
            let status = response.status;
            let error_body = response.error.unwrap_or_default();
            return Err(anyhow!(
                "Tavily API request failed with status: {}, body: {}",
                status,
                error_body
            ));
        }

        let body = response
            .body
            .context("Response body was empty, but a successful status was returned.")?;

        let serper_response: TavilySearchResponse = serde_json::from_str(&body)
            .context("Failed to deserialize Serper response from JSON body")?;

        let results = serper_response
            .results
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.url,
                content: Some(item.content),
                score: item.score,
                ..Default::default()
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
    async fn test_tavily_search() {
        let api_key = match env::var("TAVILY_API_KEY") {
            Ok(key) => key,
            Err(_) => {
                println!(
                    "Skipping test_tavily_search: TAVILY_API_KEY environment variable not set."
                );
                return;
            }
        };

        let client = TavilySearch::new(api_key, None).unwrap();

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
            println!("Top search result: {}", first_result.to_string());
        }
    }
}
