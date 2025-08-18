use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::search::{SearchParams, SearchPeriod, SearchProvider, SearchResult};

const TAVILY_API_URL: &str = "https://api.tavily.com/search";

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
    include_raw_content: Option<bool>, // 添加此行
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
    raw_content: Option<String>,
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
}

impl TavilySearch {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl SearchProvider for TavilySearch {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        let search_params = SearchParams::try_from(params)?;
        let client = reqwest::Client::new();

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

        let response = client
            .post(TAVILY_API_URL)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await?;
            return Err(anyhow!(
                "Tavily API request failed with status: {}, body: {}",
                status,
                error_body
            ));
        }

        let tavily_response: TavilySearchResponse = response.json().await?;

        let results = tavily_response
            .results
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.url,
                snippet: if !item.raw_content.as_deref().unwrap_or("").is_empty() {
                    None
                } else {
                    Some(item.content)
                },
                content: item.raw_content,
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

        let client = TavilySearch::new(api_key);

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
