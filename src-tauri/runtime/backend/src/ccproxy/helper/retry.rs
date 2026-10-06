use crate::ccproxy::errors::{CCProxyError, ProxyResult};
use reqwest::{RequestBuilder, Response};
use rust_i18n::t;
use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;

/// Exponential backoff retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts, 0 means no retry
    pub max_retries: u32,
    /// Initial backoff time in milliseconds
    pub initial_backoff_ms: u64,
    /// Maximum backoff time in milliseconds
    pub max_backoff_ms: u64,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 0,
            initial_backoff_ms: 1000,
            max_backoff_ms: 32000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Create retry configuration from settings
    pub fn from_settings(max_retries: u32) -> Self {
        Self {
            max_retries,
            ..Default::default()
        }
    }

    /// Calculate backoff duration for the nth retry attempt
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let backoff_ms = (self.initial_backoff_ms as f64
            * self.backoff_multiplier.powi(attempt as i32 - 1))
        .min(self.max_backoff_ms as f64) as u64;
        Duration::from_millis(backoff_ms)
    }
}

/// Send request with exponential backoff retry support for 429 status code
pub async fn send_with_retry(
    request_builder: RequestBuilder,
    retry_config: &RetryConfig,
) -> ProxyResult<Response> {
    if retry_config.max_retries == 0 {
        // No retry, send request directly
        return request_builder.send().await.map_err(|error| {
            log_backend_request_error(None, &error);
            CCProxyError::BackendRequestError(format_backend_request_error(&error))
        });
    }

    let mut last_error: Option<String> = None;

    for attempt in 0..=retry_config.max_retries {
        // Clone request builder for retry
        let request = request_builder.try_clone().ok_or_else(|| {
            CCProxyError::InternalError(t!("http.failed_to_clone_request").to_string())
        })?;

        match request.send().await {
            Ok(response) => {
                let status = response.status();

                // If not 429 status code, return result directly
                if status != 429 {
                    return Ok(response);
                }

                // If 429 and still have retry attempts
                if attempt < retry_config.max_retries {
                    let backoff = retry_config.calculate_backoff(attempt + 1);
                    log::warn!(
                        "Received 429 Too Many Requests, retrying in {:?} (attempt {}/{})",
                        backoff,
                        attempt + 1,
                        retry_config.max_retries
                    );

                    // Check if Retry-After header exists
                    if let Some(retry_after) = response.headers().get("retry-after") {
                        if let Ok(retry_after_str) = retry_after.to_str() {
                            // Try to parse as seconds
                            if let Ok(seconds) = retry_after_str.parse::<u64>() {
                                let retry_after_duration = Duration::from_secs(seconds);
                                log::info!(
                                    "Using Retry-After header value: {:?}",
                                    retry_after_duration
                                );
                                sleep(retry_after_duration).await;
                                continue;
                            }
                        }
                    }

                    sleep(backoff).await;
                } else {
                    // Retry attempts exhausted, return the last response
                    log::error!(
                        "Retry attempts exhausted for 429 response after {} attempts",
                        retry_config.max_retries
                    );
                    return Ok(response);
                }
            }
            Err(error) => {
                let error_msg = format_backend_request_error(&error);
                log_backend_request_error(Some(attempt), &error);
                last_error = Some(error_msg);

                // If not the last attempt, wait and retry
                if attempt < retry_config.max_retries {
                    let backoff = retry_config.calculate_backoff(attempt + 1);
                    sleep(backoff).await;
                }
            }
        }
    }

    // All retries failed
    Err(CCProxyError::BackendRequestError(
        last_error.unwrap_or_else(|| t!("proxy.error.retry_exceeded").to_string()),
    ))
}

fn format_backend_request_error(error: &reqwest::Error) -> String {
    format!("Request to backend failed: {}", error)
}

fn format_error_sources(error: &(dyn Error + 'static)) -> String {
    let mut messages = Vec::new();
    let mut current = error.source();

    while let Some(cause) = current {
        let message = cause.to_string();
        if !message.is_empty() && messages.last() != Some(&message) {
            messages.push(message);
        }
        current = cause.source();
    }

    messages.join(" -> ")
}

fn log_backend_request_error(attempt: Option<u32>, error: &reqwest::Error) {
    let causes = format_error_sources(error);
    let attempt = attempt
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());

    if causes.is_empty() {
        log::error!(
            "Backend request error: attempt={}, error={}",
            attempt,
            error
        );
    } else {
        log::error!(
            "Backend request error: attempt={}, error={}, causes={}",
            attempt,
            error,
            causes
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 0);
        assert_eq!(config.initial_backoff_ms, 1000);
        assert_eq!(config.max_backoff_ms, 32000);
        assert_eq!(config.backoff_multiplier, 2.0);
    }

    #[test]
    fn test_calculate_backoff() {
        let config = RetryConfig::default();

        // 1st retry: 1000ms
        assert_eq!(config.calculate_backoff(1), Duration::from_millis(1000));
        // 2nd retry: 2000ms
        assert_eq!(config.calculate_backoff(2), Duration::from_millis(2000));
        // 3rd retry: 4000ms
        assert_eq!(config.calculate_backoff(3), Duration::from_millis(4000));
        // 5th retry: 16000ms
        assert_eq!(config.calculate_backoff(5), Duration::from_millis(16000));
    }

    #[test]
    fn test_calculate_backoff_with_max_limit() {
        let config = RetryConfig {
            max_retries: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 5000,
            backoff_multiplier: 2.0,
        };

        // Should be capped at 5000ms
        assert_eq!(config.calculate_backoff(10), Duration::from_millis(5000));
    }

    #[test]
    fn test_format_error_sources_excludes_top_level_message() {
        let result: anyhow::Result<()> = Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused by test backend",
        ))
        .context("TCP connection failed")
        .context("request dispatch failed");
        let error = result.expect_err("test error chain should be present");

        assert_eq!(
            format_error_sources(error.as_ref()),
            "TCP connection failed -> connection refused by test backend"
        );
    }

    #[tokio::test]
    async fn test_backend_request_error_adds_context_once() {
        let request = reqwest::Client::new().get("http://127.0.0.1:0/test");
        let error = send_with_retry(request, &RetryConfig::default())
            .await
            .expect_err("test endpoint should reject the connection");

        let CCProxyError::BackendRequestError(message) = error else {
            panic!("connection failure should produce BackendRequestError");
        };

        assert_eq!(message.matches("Request to backend failed:").count(), 1);
        assert!(message.contains("error sending request for url"));
    }
}
