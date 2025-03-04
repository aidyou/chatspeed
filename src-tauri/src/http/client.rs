// HTTP client module for making API requests
//!
//! This module provides a simple HTTP client for making API requests.
//! It supports both synchronous and asynchronous requests, as well as file uploads and downloads.
//!
//! # Examples
//!
//! ```
//! use chatspeed::http::{HttpClient, HttpConfig};
//!
//! // Basic GET request
//! let client = HttpClient::new()?;
//! let config = HttpConfig::get("https://httpbin.org/get")
//!     .header("Authorization", "Bearer token");
//! let response = client.send_request(config)?;
//!
//! // POST request with JSON body
//! let config = HttpConfig::post("https://httpbin.org/post", "test body")
//!     .header("Content-Type", "application/json")
//!     .async_request();
//! let response = client.send_request(config)?;
//!
//! // File download
//! let config = HttpConfig::get("https://httpbin.org/image/jpeg")
//!                 .download_to("test_download.jpg");
//! let response = client.send_request(config)?;
//! ```

#![allow(dead_code)]

use futures::TryStreamExt;
use reqwest::redirect::Policy;
use rust_i18n::t;
use std::{collections::HashMap, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt};

use super::{
    error::{HttpError, HttpResult},
    types::{HttpConfig, HttpRequest, HttpResponse, Progress, ProgressState, RetryPolicy},
};

/// HTTP client implementation with support for async requests and file operations
pub struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    /// Creates a new HTTP client instance
    pub fn new() -> HttpResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self { client })
    }

    /// Creates a new HTTP client instance with custom configuration
    pub fn new_with_config(config: &HttpConfig) -> HttpResult<Self> {
        let client = reqwest::Client::builder()
            .redirect(
                config
                    .follow_redirects
                    .map_or(Policy::none(), |max| Policy::limited(max as usize)),
            )
            .timeout(Duration::from_secs(config.timeout.unwrap_or(30) as u64))
            .build()?;

        Ok(Self { client })
    }

    /// Helper function to handle common request errors
    fn handle_request_error(e: reqwest::Error) -> HttpError {
        if e.is_timeout() {
            HttpError::Request(t!("http.request_timeout").to_string())
        } else if e.is_connect() {
            HttpError::Request(t!("http.connection_failed", error = e.to_string()).to_string())
        } else {
            HttpError::Request(t!("http.request_failed", error = e.to_string()).to_string())
        }
    }

    /// Handles file upload requests
    async fn handle_upload(&self, request: &HttpRequest, path: &str) -> HttpResult<HttpResponse> {
        use reqwest::multipart::{Form, Part};
        use tokio::fs::File;
        use tokio::io::AsyncReadExt;

        // open file
        let mut file = File::open(path).await.map_err(|e| {
            HttpError::Request(
                t!(
                    "http.failed_to_open_file",
                    path = path,
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // read the file content
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await.map_err(|e| {
            HttpError::Request(
                t!(
                    "http.failed_to_read_file",
                    path = path,
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // get file name
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        // create multipart form
        let part = Part::bytes(contents)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| {
                HttpError::Request(t!("http.mime_parse_failed", error = e.to_string()).to_string())
            })?;

        let form = Form::new().part("file", part);

        // request builder
        let mut builder = request.build_request(&self.client).map_err(|e| {
            HttpError::Request(t!("http.client_build_failed", error = e.to_string()).to_string())
        })?;

        // add multipart form
        builder = builder.multipart(form);

        // execute request
        let response = self.execute_with_retry(request, builder).await?;
        let status = response.status().as_u16();
        let headers = Self::extract_headers(response.headers());

        let body = response.text().await.map_err(|e| {
            HttpError::Response(t!("http.read_response_failed", error = e.to_string()).to_string())
        })?;

        Ok(HttpResponse {
            status,
            headers,
            body: Some(body),
            error: None,
            progress: Some(100),
        })
    }

    /// Handles file download requests with progress tracking
    async fn handle_download(
        &self,
        request: &HttpRequest,
        save_path: &str,
    ) -> HttpResult<HttpResponse> {
        let builder = request.build_request(&self.client).map_err(|e| {
            HttpError::Request(t!("http.client_build_failed", error = e.to_string()).to_string())
        })?;

        let response = self.execute_with_retry(request, builder).await?;
        let total_size = response.content_length().unwrap_or(0);
        let status = response.status().as_u16();
        let headers = Self::extract_headers(response.headers());

        let mut downloaded: u64 = 0;
        let mut file = File::create(save_path)
            .await
            .map_err(|e| HttpError::Io(e))?;

        let mut stream = response.bytes_stream();
        let mut last_progress: Option<u8> = None;
        let mut last_update = std::time::Instant::now();
        let start_time = std::time::Instant::now();
        let update_interval = Duration::from_millis(100);

        while let Some(chunk) = stream.try_next().await.map_err(|e| {
            HttpError::Response(t!("http.download_failed", error = e.to_string()).to_string())
        })? {
            file.write_all(&chunk).await.map_err(|e| HttpError::Io(e))?;

            downloaded += chunk.len() as u64;
            let current_progress = if total_size > 0 {
                Some(((downloaded as f64 / total_size as f64) * 100.0) as u8)
            } else {
                None
            };

            let now = std::time::Instant::now();
            if (current_progress != last_progress
                || now.duration_since(last_update) >= update_interval)
                && request.config.progress_callback.is_some()
            {
                let elapsed = now.duration_since(start_time).as_secs_f64();
                let speed = if elapsed > 0.0 {
                    Some(downloaded as f64 / elapsed)
                } else {
                    None
                };

                if let Some(callback) = request.config.progress_callback.as_ref() {
                    callback(Progress {
                        state: match current_progress {
                            Some(p) => ProgressState::InProgress(p),
                            None => ProgressState::Unknown,
                        },
                        bytes_processed: downloaded,
                        total_bytes: Some(total_size),
                        speed,
                    });
                }

                last_progress = current_progress;
                last_update = now;
            }
        }

        file.flush().await.map_err(|e| HttpError::Io(e))?;
        drop(file);

        if let Some(callback) = request.config.progress_callback.as_ref() {
            callback(Progress {
                state: ProgressState::Complete,
                bytes_processed: downloaded,
                total_bytes: Some(total_size),
                speed: None,
            });
        }

        Ok(HttpResponse {
            status,
            headers,
            body: None,
            error: None,
            progress: Some(100),
        })
    }

    /// Handles standard HTTP requests
    async fn handle_request(&self, request: &HttpRequest) -> HttpResult<HttpResponse> {
        let builder = request.build_request(&self.client).map_err(|e| {
            HttpError::Request(t!("http.client_build_failed", error = e.to_string()).to_string())
        })?;

        let response = self.execute_with_retry(request, builder).await?;
        let status = response.status().as_u16();
        let headers = Self::extract_headers(response.headers());

        if let Some(stream_callback) = &request.config.stream_callback {
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();

            while let Some(chunk) = stream.try_next().await.map_err(|e| {
                HttpError::Response(
                    t!("http.read_response_failed", error = e.to_string()).to_string(),
                )
            })? {
                stream_callback(&chunk);
                body.extend_from_slice(&chunk);
            }

            Ok(HttpResponse {
                status,
                headers,
                body: Some(String::from_utf8_lossy(&body).to_string()),
                error: None,
                progress: Some(100),
            })
        } else {
            let body = response.text().await.map_err(|e| {
                HttpError::Response(
                    t!("http.read_response_failed", error = e.to_string()).to_string(),
                )
            })?;

            Ok(HttpResponse {
                status,
                headers,
                body: Some(body),
                error: None,
                progress: Some(100),
            })
        }
    }

    /// Executes an HTTP request with retry logic
    async fn execute_with_retry(
        &self,
        request: &HttpRequest,
        builder: reqwest::RequestBuilder,
    ) -> HttpResult<reqwest::Response> {
        let retry_policy = RetryPolicy::default();
        let mut attempts = 0;
        let retry_policy = request
            .config
            .retry_policy
            .as_ref()
            .unwrap_or(&retry_policy);
        let mut delay = retry_policy.initial_delay;

        loop {
            attempts += 1;
            let mut cloned_builder = builder.try_clone().ok_or_else(|| {
                HttpError::Request(t!("http.failed_to_clone_request").to_string())
            })?;

            log::debug!("Attempting HTTP request (attempt {})", attempts);

            // 设置总超时
            if let Some(timeout) = request.config.timeout {
                if timeout > 0 {
                    cloned_builder =
                        cloned_builder.timeout(std::time::Duration::from_secs(timeout as u64));
                }
            }

            match cloned_builder.send().await {
                Ok(response) => {
                    log::debug!("HTTP request completed: {:?}", response);
                    if let Some(max_size) = request.config.max_body_size {
                        if let Some(content_length) = response.content_length() {
                            if content_length > max_size as u64 {
                                return Err(HttpError::Response(
                                    t!(
                                        "http.response_too_large",
                                        content_length = content_length,
                                        max_size = max_size
                                    )
                                    .to_string(),
                                ));
                            }
                        }
                    }
                    return Ok(response);
                }
                Err(e) if Self::should_retry(&e) && attempts < retry_policy.max_retries => {
                    log::warn!("Request failed, retrying: {:?}", e);
                    delay = (delay * retry_policy.backoff_factor).min(retry_policy.max_delay);
                    tokio::time::sleep(Duration::from_secs_f32(delay)).await;
                    continue;
                }
                Err(e) => return Err(Self::handle_request_error(e)),
            }
        }
    }

    /// Extracts headers from reqwest::HeaderMap into a HashMap
    fn extract_headers(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
        headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect()
    }

    /// Determines if a request should be retried based on the error
    fn should_retry(error: &reqwest::Error) -> bool {
        error.is_timeout()
            || error.is_connect()
            || error.is_request()
            || matches!(error.status(), Some(status) if status.is_server_error())
    }

    /// send async request
    ///
    /// # Parameters
    /// - `config`: The configuration for the request
    ///
    /// # Returns
    /// - `HttpResult<HttpResponse>`: The response from the server
    pub async fn send_request(&self, config: HttpConfig) -> HttpResult<HttpResponse> {
        let request = HttpRequest::new(config);
        self.send_request_async_impl(&request).await
    }

    /// Handles the actual request and retries if needed
    ///
    /// # Parameters
    /// - `request`: The request configuration
    ///
    /// # Returns
    /// - `HttpResult<HttpResponse>`: The response from the server
    async fn send_request_async_impl(&self, request: &HttpRequest) -> HttpResult<HttpResponse> {
        if let Some(path) = &request.config.upload_file {
            self.handle_upload(request, path).await
        } else if let Some(path) = &request.config.download_path {
            self.handle_download(request, path).await
        } else {
            self.handle_request(request).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_basic_get() {
        let client = HttpClient::new().unwrap();
        let config = HttpConfig::get("http://127.0.0.1:12321/data?url=https://ezool.net");

        let result = client.send_request(config).await;
        dbg!(&result);
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.is_some());
    }

    #[tokio::test]
    async fn test_post_with_body() {
        let client = HttpClient::new().unwrap();
        let config = HttpConfig::post("https://httpbin.org/post", "test body");

        let result = client.send_request(config).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.is_some());
    }

    #[tokio::test]
    async fn test_download() {
        let client = HttpClient::new().unwrap();
        let config =
            HttpConfig::get("https://httpbin.org/image/jpeg").download_to("test_download.jpg");

        let result = client.send_request(config).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status, 200);
        assert!(Path::new("test_download.jpg").exists());
        std::fs::remove_file("test_download.jpg").unwrap();
    }

    #[tokio::test]
    async fn test_async_request() {
        let client = HttpClient::new().unwrap();
        let config = HttpConfig::get("https://httpbin.org/get").async_request();

        let result = client.send_request(config).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.is_some());
    }
}
