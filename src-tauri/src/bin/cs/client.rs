//! HTTP client for the control plane.
//!
//! The CLI is a pure client: it reads the discovery document, sends bearer
//! tokens via the `Authorization` header only, and never touches the database
//! or workflow runtime.

use crate::discovery::ControlPlaneDiscovery;
use crate::error::CliError;
use serde_json::Value;
use std::time::Duration;

/// HTTP client bound to one control-plane instance.
pub struct ControlPlaneClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl ControlPlaneClient {
    pub fn new(discovery: &ControlPlaneDiscovery) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| CliError::transport(format!("Failed to build HTTP client: {}", e)))?;
        Ok(Self {
            base_url: format!("http://{}:{}", discovery.host, discovery.port),
            token: discovery.token.clone(),
            http,
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Performs a GET request and decodes the JSON response.
    pub async fn get(&self, path: &str) -> Result<Value, CliError> {
        let response = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| CliError::transport(format!("Request failed: {}", e)))?;
        self.decode(response).await
    }

    /// Performs a POST mutation. When `idempotency_key` is provided, a single
    /// transport-level retry reuses the same key so the server can deduplicate.
    pub async fn post(
        &self,
        path: &str,
        body: Value,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let mut attempt = 0;
        loop {
            let request = self
                .http
                .post(&url)
                .header("Authorization", self.auth_header())
                .json(&body);
            let request = match idempotency_key {
                Some(key) => request.header("Idempotency-Key", key),
                None => request,
            };
            match request.send().await {
                Ok(response) => return self.decode(response).await,
                Err(error) => {
                    attempt += 1;
                    if idempotency_key.is_some() && attempt == 1 {
                        // One retry with the same idempotency key.
                        continue;
                    }
                    return Err(CliError::transport(format!("Request failed: {}", error)));
                }
            }
        }
    }

    /// Opens an SSE stream; the caller consumes raw bytes incrementally.
    pub async fn stream(
        &self,
        path: &str,
        last_event_id: Option<&str>,
    ) -> Result<reqwest::Response, CliError> {
        let mut request = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .header("Authorization", self.auth_header());
        if let Some(cursor) = last_event_id {
            request = request.header("Last-Event-ID", cursor);
        }
        let response = request
            .send()
            .await
            .map_err(|e| CliError::transport(format!("Stream request failed: {}", e)))?;
        if !response.status().is_success() {
            // decode() always returns Err for non-2xx responses.
            return match self.decode(response).await {
                Err(error) => Err(error),
                Ok(_) => Err(CliError::transport("Unexpected stream response")),
            };
        }
        Ok(response)
    }

    async fn decode(&self, response: reqwest::Response) -> Result<Value, CliError> {
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| CliError::transport(format!("Failed to read response: {}", e)))?;

        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(value);
        }

        // Stable error envelope: {"error": {"code", "message"}}.
        let code = value["error"]["code"]
            .as_str()
            .unwrap_or("unknown_error")
            .to_string();
        let message = value["error"]["message"]
            .as_str()
            .unwrap_or(&body)
            .to_string();

        match status.as_u16() {
            401 => Err(CliError::auth(format!(
                "Authentication failed ({}): {}",
                code, message
            ))),
            _ => Err(CliError::Server {
                status: status.as_u16(),
                code,
                message,
            }),
        }
    }
}
