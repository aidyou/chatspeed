use reqwest::{Client, StatusCode};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt::Display;
use std::{convert::TryFrom, str::FromStr};
use url::form_urlencoded::byte_serialize;

use crate::http::{
    client::HttpClient,
    error::{HttpError, HttpResult},
    types::HttpConfig,
};

#[derive(Clone)]
pub struct Chp {
    chp_server: String,
    proxy: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrawlerData {
    pub id: i32,
    pub title: String,
    pub content: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(skip_deserializing, skip_serializing_if = "is_zero")]
    pub id: usize,
    pub title: String,
    pub url: String,
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sitename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_date: Option<String>,
    #[serde(skip_deserializing)]
    #[serde(default, skip_serializing)]
    pub score: f32,
}

fn is_zero<T: PartialEq + From<u8>>(value: &T) -> bool {
    *value == T::from(0)
}

impl Default for SearchResult {
    fn default() -> Self {
        Self {
            id: 0,
            title: String::new(),
            url: String::new(),
            summary: None,
            sitename: None,
            publish_date: None,
            score: 0.0,
        }
    }
}
#[derive(PartialEq, Clone)]
pub enum SearchProvider {
    Baidu,
    BaiduNews,
    Google,
    GoogleNews,
    Bing,
}

impl Display for SearchProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Baidu => write!(f, "baidu"),
            Self::BaiduNews => write!(f, "baidu_news"),
            Self::Google => write!(f, "google"),
            Self::GoogleNews => write!(f, "google_news"),
            Self::Bing => write!(f, "bing"),
        }
    }
}

impl From<SearchProvider> for String {
    fn from(provider: SearchProvider) -> Self {
        provider.to_string()
    }
}

impl FromStr for SearchProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl TryFrom<&str> for SearchProvider {
    type Error = String;

    fn try_from(provider: &str) -> Result<Self, Self::Error> {
        match provider {
            "baidu" => Ok(Self::Baidu),
            "baidu_news" => Ok(Self::BaiduNews),
            "google" => Ok(Self::Google),
            "google_news" => Ok(Self::GoogleNews),
            "bing" => Ok(Self::Bing),
            _ => Err(format!("Invalid search provider: {}", provider)),
        }
    }
}

impl TryFrom<String> for SearchProvider {
    type Error = String;

    fn try_from(provider: String) -> Result<Self, Self::Error> {
        Self::try_from(provider.as_str())
    }
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

impl Chp {
    pub fn new(chp_server_url: String, proxy: Option<String>) -> Self {
        Self {
            chp_server: chp_server_url,
            proxy,
        }
    }

    /// Calls CHP function with the specified parameters.
    ///
    /// # Arguments
    ///
    /// * `function_name` - The name of the remote function to call.
    /// * `method` - The HTTP method to use for the request (GET or POST).
    /// * `params` - Optional parameters to pass to the remote function. It's a JSON object.
    /// * `headers` - Optional headers to pass to the remote function. It's a JSON object.
    ///
    /// # Returns
    /// * `HttpResult<Value>` - The result of the remote function call.
    pub async fn call(
        &self,
        function_name: &str,
        method: &str,
        params: Option<Value>,
        headers: Option<std::collections::HashMap<String, String>>,
    ) -> HttpResult<Value> {
        let query_url = format!("{}/chp/{}", self.chp_server, function_name);

        let mut config = if method.to_lowercase() == "post" {
            HttpConfig::post(
                &query_url,
                params.as_ref().map(|v| v.to_string()).unwrap_or_default(),
            )
            .header("Content-Type", "application/json")
        } else {
            let query_str = if let Some(ref params) = params {
                // parse params to url query string
                let mut query_url = "".to_string();
                if let Some(p) = params.as_object() {
                    let query_string: Vec<String> = p
                        .iter()
                        .map(|(k, v)| {
                            let value_str = match v {
                                // Important: trim the '"' from the string
                                serde_json::Value::String(s) => s.clone(),
                                _ => v.to_string().trim_matches('"').to_string(),
                            };
                            format!("{}={}", Self::urlencode(k), Self::urlencode(&value_str))
                        })
                        .collect();
                    query_url.push_str(&query_string.join("&"));
                }
                query_url
            } else {
                "".to_string()
            };
            if query_str.is_empty() {
                HttpConfig::get(&query_url)
            } else {
                let sep = if query_url.contains('?') { "&" } else { "?" };
                HttpConfig::get(format!("{}{}{}", &query_url, sep, query_str))
            }
        };

        // Set proxy
        if let Some(proxy) = self.proxy.as_ref() {
            config = config.proxy(proxy.clone());
        }

        // Set headers
        if let Some(headers) = headers {
            config = config.headers(headers);
        }
        let client = HttpClient::new_with_config(&config)?;

        let response = client.send_request(config).await?;
        if let Some(err) = response.error {
            Err(HttpError::Response(err))
        } else if response.status != StatusCode::OK {
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
            if value.is_null() {
                return Err(HttpError::Response(
                    t!("http.missing_response_body").to_string(),
                ));
            }

            let code = value["code"].as_i64().unwrap_or(0);
            if code == 0 {
                return Err(HttpError::Response(
                    t!(
                        "http.invalid_response_format",
                        error = value["message"].as_str().unwrap_or_default()
                    )
                    .to_string(),
                ));
            }

            if code != 200 {
                log::error!(
                    "Http request failed, code: {}, error: {}",
                    code,
                    value["message"].as_str().unwrap_or_default()
                );
                return Err(HttpError::Response(
                    t!(
                        "http.request_failed",
                        status = code,
                        error = value["message"].as_str().unwrap_or_default()
                    )
                    .to_string(),
                ));
            }
            if value["data"].is_null() {
                return Err(HttpError::Response(
                    t!("http.missing_response_data").to_string(),
                ));
            }

            Ok(value["data"].clone())
        }
    }

    /// Crawl a single page and get the title and content
    ///
    /// # Arguments
    ///
    /// * `url_to_crawl` - The URL of the page to crawl.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or if the response is invalid.
    pub async fn web_crawler(
        &self,
        url_to_crawl: &str,
        params: Option<Value>,
    ) -> HttpResult<CrawlerData> {
        let mut params = params.unwrap_or_default();
        params["url"] = json!(url_to_crawl);

        let value = self.call("web_crawler", "get", Some(params), None).await?;
        let content = value.get_str_or_default("content");

        if content.is_empty() {
            return Err(HttpError::Response(t!("http.crawler_failed").to_string()));
        }

        Ok(CrawlerData {
            id: value.get_str_or_default("id").parse().unwrap_or(0),
            title: value.get_str_or_default("title"),
            content,
            url: url_to_crawl.to_string(),
        })
    }

    /// Search for pages containing the specified keywords
    ///
    /// # Arguments
    /// * `provider` - The search provider to use.
    /// * `keywords` - A slice of strings containing the keywords to search for.
    /// * `page` - The page number to start searching from.
    /// * `time_period` - The time period to search for, optional, value can be empty or one of `day`, `week`, `month`, `year`.
    /// * `number` - The number of pages to search.
    /// * `resolve_baidu_links` - Whether to resolve Baidu links.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or if the response is invalid.
    pub async fn web_search(
        &self,
        provider: SearchProvider,
        keywords: &[&str],
        page: Option<i64>,
        number: Option<i64>,
        time_period: Option<&str>,
        resolve_baidu_links: bool,
    ) -> HttpResult<Vec<SearchResult>> {
        let encoded_keywords: String = keywords
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        let page = page.unwrap_or(1);
        let number = number.unwrap_or(10);
        let value = self
            .call(
                "web_search",
                "get",
                Some(json!({
                    "provider": provider.to_string(),
                    "kw": encoded_keywords,
                    "page": page,
                    "number": number,
                    "time_period": time_period.unwrap_or("").to_string(),
                    "resolve_baidu_links": resolve_baidu_links
                })),
                None,
            )
            .await?;

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
                ..SearchResult::default()
            });
        }

        if resolve_baidu_links
            && (provider == SearchProvider::Baidu || provider == SearchProvider::BaiduNews)
        {
            Ok(Self::resolve_all_baidu_links(results).await)
        } else {
            Ok(results)
        }
    }

    /// Resolves Baidu short URL with retry mechanism
    ///
    /// # Arguments
    /// * `short_url` - The Baidu short URL to resolve
    ///
    /// # Returns
    /// * `Ok(String)` - The resolved original URL
    /// * `Err(Box<dyn std::error::Error>)` - Error if resolution fails after retries
    pub async fn resolve_baidu_link_with_retry(
        short_url: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .build()?;

        let mut retries = 3;
        while retries > 0 {
            let response = client.get(short_url).send().await?;
            if let Some(location) = response.headers().get("Location") {
                return Ok(location.to_str().unwrap_or_default().to_string());
            }
            retries -= 1;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Err("Failed to resolve URL after retries".into())
    }

    /// Resolves all Baidu short URLs in a list of search results
    ///
    /// # Arguments
    /// * `results` - List of search results containing Baidu short URLs
    ///
    /// # Returns
    /// * `Vec<SearchResult>` - List of search results with resolved URLs
    async fn resolve_all_baidu_links(results: Vec<SearchResult>) -> Vec<SearchResult> {
        let mut resolved_results = Vec::new();
        for mut result in results {
            if result.url.starts_with("http://www.baidu.com/link") {
                if let Ok(resolved_url) = Self::resolve_baidu_link_with_retry(&result.url).await {
                    result.url = resolved_url;
                }
            }
            resolved_results.push(result);
        }
        resolved_results
    }

    /// URL encode a string
    ///
    /// # Arguments
    /// * `s` - The string to encode
    ///
    /// # Returns
    /// * `String` - The encoded string
    fn urlencode(s: &str) -> String {
        byte_serialize(s.as_bytes()).collect::<String>()
    }
}

mod tests {

    #[tokio::test]
    async fn test_resolve_baidu_link_with_retry() {
        let start = std::time::Instant::now();
        let result = crate::http::chp::Chp::resolve_baidu_link_with_retry("http://www.baidu.com/link?url=WIQRAMUouxr_m19f9eqQJ3yL6hmq_Fe9f3sP9APaC_L2dPCtnAsxXse6zm2Dea3v").await;
        let duration = start.elapsed();

        println!("链接转换耗时: {:?}", duration);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_resolve_all_baidu_links() {
        let start = std::time::Instant::now();
        let sr = r#"[
        {
            "title": "五粮液集团",
            "url": "http://www.baidu.com/link?url=lgBH1Rcryr1Zw5bG1boR5tLc2OTGvt43sWML__A2IloIMxvte3rOudlHgCMhIRbi"
        },
        {
            "site_name": "阿里巴巴1688",
            "title": "宜宾五粮液白酒-宜宾五粮液白酒批发、促销价格、产地货源 ...",
            "url": "http://www.baidu.com/link?url=xEdCv8KMtVCnCpIg_LqQxmZV04N9e3xhE1cPvyp779RNvaVPQsbT1UayN0sx38WJhks8fN_azfBm0oMHd-H7BlGTzlHDlnbZ-2ZHxhADp8G",
            "summary": ""
        },
        {
            "site_name": "京东",
            "title": "五粮液浓香型 52度价格及图片表 - 京东",
            "url": "http://www.baidu.com/link?url=AQtx2AxOavR39513fGQc7fsoWM73N50DiH-lnYgnJpu4F-0xGFZ-UBaCWv3HSyIh1s_5_5TG87UAsXa85ORXxK",
            "summary": ""
        },
        {
            "site_name": "京东",
            "title": "五粮液浓香型 - 京东",
            "url": "http://www.baidu.com/link?url=RrhbQKBd3FqSnJJivcxBZnqA-bepcfq4UMV5soAzzvkyc0loU-_6Mjey8yC-0rvd_Erx1-SijuaG2dh1f3NXga",
            "summary": ""
        },
        {
            "site_name": "家居好物记",
            "title": "五粮液系列白酒值得买吗?盘点推荐十大优质款式!",
            "url": "http://www.baidu.com/link?url=Y2u0b0QO6Amq_Hz1Mofjy-emFc226-5LE4qdJ1CWSXCDs6qTA0rMZJNKPnc1g6Fpd6XZxgv9ZxvZTfwt0dlTeE-XfucUxELQMC0ptSbviqK",
            "summary": ""
        }]"#;
        let results: Vec<crate::http::chp::SearchResult> =
            serde_json::from_str(sr).expect("Failed to parse JSON");
        let result = crate::http::chp::Chp::resolve_all_baidu_links(results).await;
        let duration = start.elapsed();
        dbg!(&result);
        println!("链接转换耗时: {:?}", duration);
    }

    #[tokio::test]
    async fn test_web_search() {
        let start = std::time::Instant::now();
        let chp = crate::http::chp::Chp::new("http://localhost:12321".to_string(), None);
        let result = chp
            .web_search(
                crate::http::chp::SearchProvider::Google,
                &["deepseek"],
                Some(1),
                Some(10),
                None,
                false,
            )
            .await;
        let duration = start.elapsed();
        dbg!(&result);
        println!("搜索耗时: {:?}", duration);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_web_crawler() {
        let start = std::time::Instant::now();
        let chp = crate::http::chp::Chp::new("http://localhost:12321".to_string(), None);
        let result = chp
            .web_crawler("https://www.bbc.com/news/articles/c5yv5976z9po", None)
            .await;
        let duration = start.elapsed();
        dbg!(&result);
        println!("搜索耗时: {:?}", duration);
        assert!(result.is_ok());
    }
}
