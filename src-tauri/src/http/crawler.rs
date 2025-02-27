use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::form_urlencoded::byte_serialize;

use super::{
    client::HttpClient,
    error::{HttpError, HttpResult},
    types::HttpConfig,
};

pub struct Crawler {
    crawler_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlerData {
    pub title: String,
    pub content: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub summary: Option<String>,
    pub sitename: Option<String>,
    pub publish_date: Option<String>,
}

/// Extension trait for `Value` that provides helper methods for getting string values
pub trait ValueExt {
    fn get_str(&self, key: &str) -> Option<&str>;
    fn get_str_or_default(&self, key: &str) -> String;
}

impl ValueExt for Value {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }

    fn get_str_or_default(&self, key: &str) -> String {
        self.get_str(key).unwrap_or_default().to_string()
    }
}

impl Crawler {
    pub fn new(crawler_url: String) -> Self {
        Self { crawler_url }
    }

    /// Crawl a single page and get the title and content
    ///
    /// # Arguments
    ///
    /// * `url_to_crawl` - The URL of the page to crawl.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or if the response is invalid.
    pub async fn crawl_data(&self, url_to_crawl: &str) -> HttpResult<CrawlerData> {
        let full_url = format!(
            "{}/data?url={}",
            self.crawler_url,
            byte_serialize(url_to_crawl.as_bytes()).collect::<String>()
        );

        let client = HttpClient::new()?;
        let config = HttpConfig::get(&full_url);

        let response = client.send_request(config).await?;

        if let Some(err) = response.error {
            Err(HttpError::Response(err))
        } else if response.status != 200 {
            Err(HttpError::Response(
                t!("http.status_not_ok", status = response.status).to_string(),
            ))
        } else {
            let body = response.body.ok_or(HttpError::Response(
                t!("http.missing_response_body").to_string(),
            ))?;

            let value: Value = serde_json::from_str(&body).map_err(|e| {
                HttpError::Response(t!("http.json_parse_failed", error = e.to_string()).to_string())
            })?;

            Ok(CrawlerData {
                title: value.get_str_or_default("title"),
                content: value.get_str_or_default("content"),
                url: url_to_crawl.to_string(),
            })
        }
    }

    /// Search for pages containing the specified keywords
    ///
    /// # Arguments
    ///
    /// * `keywords` - A slice of strings containing the keywords to search for.
    /// * `page` - The page number to start searching from.
    /// * `number` - The number of pages to search.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or if the response is invalid.
    pub async fn search(
        &self,
        keywords: &[&str],
        page: Option<usize>,
        number: Option<usize>,
    ) -> HttpResult<Vec<SearchResult>> {
        let encoded_keywords: String = keywords
            .iter()
            .map(|k| byte_serialize(k.as_bytes()).collect::<String>())
            .collect::<Vec<_>>()
            .join("%20");

        let page = page.unwrap_or(1);
        let number = number.unwrap_or(10);
        let full_url = format!(
            "{}/search?kw={}&page={}&number={}",
            self.crawler_url, encoded_keywords, page, number
        );

        let client = HttpClient::new()?;
        let config = HttpConfig::get(&full_url);
        let response = client.send_request(config).await?;

        if let Some(err) = response.error {
            Err(HttpError::Response(err))
        } else if response.status != 200 {
            Err(HttpError::Response(
                t!("http.status_not_ok", status = response.status).to_string(),
            ))
        } else {
            let body = response.body.ok_or(HttpError::Response(
                t!("http.missing_response_body").to_string(),
            ))?;

            let value: Value = serde_json::from_str(&body).map_err(|e| {
                HttpError::Response(t!("http.json_parse_failed", error = e.to_string()).to_string())
            })?;

            let items = value.as_array().ok_or(HttpError::Response(
                t!("http.invalid_response_format", error = "Expected an array").to_string(),
            ))?;

            let mut results = Vec::new();
            for item in items {
                results.push(SearchResult {
                    title: item.get_str_or_default("title"),
                    url: item.get_str_or_default("url"),
                    summary: item.get_str("summary").map(|s| s.to_string()),
                    sitename: item.get_str("sitename").map(|s| s.to_string()),
                    publish_date: item.get_str("publish_date").map(|s| s.to_string()),
                });
            }

            Ok(results)
        }
    }
}
