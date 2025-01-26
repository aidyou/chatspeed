//! HTTP Types and Configurations
//!
//! Defines the core types used by the HTTP client, including request
//! configurations, responses, and utility functions for building HTTP requests.

use rust_i18n::t;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::error::HttpError;

/// HTTP request methods
#[derive(Debug, Clone, Serialize)]
pub enum HttpMethod {
    /// GET request method
    Get,
    /// POST request method
    Post,
    /// PUT request method
    Put,
    /// DELETE request method
    Delete,
    /// HEAD request method
    Head,
    /// OPTIONS request method
    Options,
    /// PATCH request method
    Patch,
}

impl Default for HttpMethod {
    /// Returns the default HTTP request method (GET)
    fn default() -> Self {
        Self::Get
    }
}

/// Deserializes HTTP method from string
fn deserialize_http_method<'de, D>(deserializer: D) -> Result<HttpMethod, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.to_uppercase().as_str() {
        "GET" => Ok(HttpMethod::Get),
        "POST" => Ok(HttpMethod::Post),
        "PUT" => Ok(HttpMethod::Put),
        "DELETE" => Ok(HttpMethod::Delete),
        "HEAD" => Ok(HttpMethod::Head),
        "OPTIONS" => Ok(HttpMethod::Options),
        "PATCH" => Ok(HttpMethod::Patch),
        _ => Err(serde::de::Error::custom(format!(
            "Invalid HTTP method: {}",
            s
        ))),
    }
}

impl<'de> Deserialize<'de> for HttpMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_http_method(deserializer)
    }
}

/// Progress tracking states for file operations
#[derive(Debug, Clone, Copy)]
pub enum ProgressState {
    /// Progress cannot be determined
    Unknown,
    /// Operation in progress with percentage
    InProgress(u8),
    /// Operation completed successfully
    Complete,
    /// Operation failed
    Failed,
}

/// Progress information for file operations
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    /// Current progress state
    pub state: ProgressState,
    /// Number of bytes processed
    pub bytes_processed: u64,
    /// Total number of bytes (if known)
    pub total_bytes: Option<u64>,
    /// Transfer speed in bytes per second
    pub speed: Option<f64>,
}

/// Retry policy configuration for failed requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial retry delay in seconds
    pub initial_delay: f32,
    /// Maximum retry delay in seconds
    pub max_delay: f32,
    /// Multiplier for exponential backoff
    pub backoff_factor: f32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: 1.0,
            max_delay: 30.0,
            backoff_factor: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Determines if a request should be retried based on the attempt count and error
    pub fn should_retry(&self, attempt: u32, error: &reqwest::Error) -> bool {
        if attempt >= self.max_retries {
            return false;
        }

        error.is_timeout()
            || error.is_connect()
            || error.status().map_or(false, |s| s.is_server_error())
    }

    /// Calculates the delay duration for the next retry attempt
    pub fn get_delay(&self, attempt: u32) -> Duration {
        let delay = self.initial_delay * self.backoff_factor.powi(attempt as i32);
        Duration::from_secs_f32(delay.min(self.max_delay))
    }
}

/// Default request timeout in seconds
const fn default_timeout() -> Option<u32> {
    None // No timeout by default
}

/// Default connection timeout in seconds
const fn default_connect_timeout() -> Option<u32> {
    Some(30) // 30 seconds is reasonable for connection
}

/// Default setting for following redirects
const fn default_follow_redirects() -> bool {
    true
}

/// Default maximum number of redirects to follow
const fn default_max_redirects() -> u32 {
    10
}

/// Default setting for cookie store
const fn default_cookie_store() -> bool {
    false
}

/// Default setting for compression
const fn default_compression() -> bool {
    true
}

/// HTTP request configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Request URL
    pub url: String,
    /// Request method
    #[serde(deserialize_with = "deserialize_http_method")]
    pub method: HttpMethod,
    /// Request headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Upload file path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_file: Option<String>,
    /// Download save path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_path: Option<String>,
    /// Whether to enable cookie store
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_cookie_store: Option<bool>,
    /// Progress callback
    #[serde(skip)]
    pub progress_callback: Option<Arc<dyn Fn(Progress) + Send + Sync>>,
    /// Stream callback
    #[serde(skip)]
    pub stream_callback: Option<Arc<dyn Fn(&[u8]) + Send + Sync>>,
    /// Retry policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
    /// Whether to enable compression
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<bool>,
    /// Proxy configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Request timeout in seconds, None means no timeout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Connection timeout in seconds, None means no timeout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout: Option<u32>,
    /// Whether to follow redirects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    /// Maximum number of redirects to follow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_redirects: Option<u32>,
    /// Whether to handle the request asynchronously
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_request: Option<bool>,
    /// Maximum response body size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_body_size: Option<usize>,
}

impl std::fmt::Debug for HttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpConfig")
            .field("url", &self.url)
            .field("method", &self.method)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .field("upload_file", &self.upload_file)
            .field("download_path", &self.download_path)
            .field("enable_cookie_store", &self.enable_cookie_store)
            .field("compression", &self.compression)
            .field("proxy", &self.proxy)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("follow_redirects", &self.follow_redirects)
            .field("max_redirects", &self.max_redirects)
            .field("async_request", &self.async_request)
            .field("max_body_size", &self.max_body_size)
            .field("retry_policy", &self.retry_policy)
            .field(
                "progress_callback",
                &if self.progress_callback.is_some() {
                    "Some(Fn)"
                } else {
                    "None"
                },
            )
            .field(
                "stream_callback",
                &if self.stream_callback.is_some() {
                    "Some(Fn)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            upload_file: None,
            download_path: None,
            enable_cookie_store: None,
            compression: None,
            proxy: None,
            timeout: default_timeout(),
            connect_timeout: default_connect_timeout(),
            follow_redirects: Some(default_follow_redirects()),
            max_redirects: Some(default_max_redirects()),
            async_request: Some(false),
            max_body_size: None,
            retry_policy: None,
            progress_callback: None,
            stream_callback: None,
        }
    }
}

impl HttpConfig {
    /// Creates a new GET request configuration
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: HttpMethod::Get,
            ..Default::default()
        }
    }

    /// Creates a new POST request configuration
    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: HttpMethod::Post,
            body: Some(body.into()),
            ..Default::default()
        }
    }

    /// Headers that use semicolon as separator
    const SEMICOLON_SEPARATED_HEADERS: &'static [&'static str] = &[
        "cookie",
        "set-cookie",
        "www-authenticate",
        "proxy-authenticate",
    ];

    /// Headers that use comma as separator
    const COMMA_SEPARATED_HEADERS: &'static [&'static str] = &[
        "accept",
        "accept-charset",
        "accept-encoding",
        "accept-language",
        "allow",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "access-control-expose-headers",
        "cache-control",
        "connection",
        "content-encoding",
        "content-language",
        "if-match",
        "if-none-match",
        "link",
        "vary",
        "via",
        "warning",
    ];

    /// Adds a request header
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        let key_lower = key.to_lowercase();

        if Self::COMMA_SEPARATED_HEADERS.contains(&key_lower.as_str())
            || Self::SEMICOLON_SEPARATED_HEADERS.contains(&key_lower.as_str())
        {
            let separator = if Self::SEMICOLON_SEPARATED_HEADERS.contains(&key_lower.as_str()) {
                "; "
            } else {
                ", "
            };

            if let Some(existing) = self.headers.get_mut(&key) {
                existing.push_str(separator);
                existing.push_str(&value);
            } else {
                self.headers.insert(key, value);
            }
        } else {
            self.headers.insert(key, value);
        }
        self
    }

    /// Adds a request header if it doesn't exist
    pub fn header_if_not_exists(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let key = key.into();
        if !self.headers.contains_key(&key) {
            self.headers.insert(key, value.into());
        }
        self
    }

    /// Removes a request header
    pub fn remove_header(mut self, key: impl Into<String>) -> Self {
        self.headers.remove(&key.into());
        self
    }

    /// Sets the request to be asynchronous
    pub fn async_request(mut self) -> Self {
        self.async_request = Some(true);
        self
    }

    /// Sets the file to upload
    pub fn upload(mut self, file_path: impl Into<String>) -> Self {
        self.upload_file = Some(file_path.into());
        self
    }

    /// Sets the download destination path
    pub fn download_to(mut self, save_path: impl Into<String>) -> Self {
        self.download_path = Some(save_path.into());
        self
    }

    /// Sets whether to enable cookie store
    pub fn cookie_store(mut self, enable: bool) -> Self {
        self.enable_cookie_store = Some(enable);
        self
    }

    /// Sets the progress callback for file operations
    pub fn progress_callback(
        mut self,
        callback: impl Fn(Progress) + Send + Sync + 'static,
    ) -> Self {
        self.progress_callback = Some(Arc::new(callback));
        self
    }

    /// Sets the stream callback for streaming responses
    pub fn stream_callback(mut self, callback: impl Fn(&[u8]) + Send + Sync + 'static) -> Self {
        self.stream_callback = Some(Arc::new(callback));
        self
    }
}

/// HTTP response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    /// Response status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: Option<String>,
    /// Error message if any
    pub error: Option<String>,
    /// Progress percentage (0-100)
    pub progress: Option<u8>,
}

/// HTTP request wrapper
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// Request configuration
    pub config: HttpConfig,
}

impl HttpRequest {
    /// Creates a new HTTP request
    pub fn new(config: HttpConfig) -> Self {
        Self { config }
    }

    /// Builds a reqwest request from the configuration
    pub fn build_request(
        &self,
        client: &reqwest::Client,
    ) -> Result<reqwest::RequestBuilder, HttpError> {
        // Check if URL exists
        if self.config.url.is_empty() {
            return Err(HttpError::Config(t!("http.missing_url").to_string()));
        }

        // Convert headers
        let headers = (&self.config.headers)
            .try_into()
            .map_err(|e: tauri::http::Error| {
                HttpError::Config(
                    t!("http.headers_convert_failed", error = e.to_string()).to_string(),
                )
            })?;

        // Build request with method and URL
        let mut builder = client
            .request(
                match self.config.method {
                    HttpMethod::Get => reqwest::Method::GET,
                    HttpMethod::Post => reqwest::Method::POST,
                    HttpMethod::Put => reqwest::Method::PUT,
                    HttpMethod::Delete => reqwest::Method::DELETE,
                    HttpMethod::Head => reqwest::Method::HEAD,
                    HttpMethod::Options => reqwest::Method::OPTIONS,
                    HttpMethod::Patch => reqwest::Method::PATCH,
                },
                &self.config.url,
            )
            .headers(headers);

        // Add body if present
        if let Some(body) = &self.config.body {
            builder = builder.body(body.clone());
        }

        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_config_builder() {
        // Test GET request
        let config = HttpConfig::get("https://api.example.com/users")
            .header("Authorization", "Bearer token")
            .async_request();

        assert_eq!(config.url, "https://api.example.com/users");
        assert!(matches!(config.method, HttpMethod::Get));
        assert!(config.async_request.unwrap());
        assert_eq!(config.headers.get("Authorization").unwrap(), "Bearer token");

        // Test POST request
        let config = HttpConfig::post("https://api.example.com/users", r#"{"name": "test"}"#)
            .header("Content-Type", "application/json");

        assert_eq!(config.url, "https://api.example.com/users");
        assert!(matches!(config.method, HttpMethod::Post));
        assert_eq!(config.body.unwrap(), r#"{"name": "test"}"#);
        assert_eq!(
            config.headers.get("Content-Type").unwrap(),
            "application/json"
        );

        // Test file operations
        let config = HttpConfig::get("https://example.com/file.zip").download_to("local/file.zip");

        assert_eq!(config.download_path.unwrap(), "local/file.zip");
    }

    #[test]
    fn test_header_handling() {
        let config = HttpConfig::get("https://api.example.com")
            .header("Cookie", "session=123")
            .header("Cookie", "user=456") // Should append
            .header("Content-Type", "text/plain")
            .header("Content-Type", "application/json"); // Should override

        assert_eq!(
            config.headers.get("Cookie").unwrap(),
            "session=123; user=456"
        );
        assert_eq!(
            config.headers.get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_http_method_case_insensitive() {
        let methods = vec!["get", "GET", "Get", "gEt"];
        for method in methods {
            let json = format!(
                r#"{{"method": "{}", "url": "https://example.com"}}"#,
                method
            );
            let config: HttpConfig = serde_json::from_str(&json).unwrap();
            assert!(matches!(config.method, HttpMethod::Get));
        }

        // Test invalid method
        let json = r#"{"method": "INVALID"}"#;
        let result: Result<HttpConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
