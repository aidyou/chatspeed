use super::stream::{StreamFormat, StreamParser};
use super::{types::*, StreamChunk};
use async_trait::async_trait;
use bytes::Bytes;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client, Response,
};
use rust_i18n::t;
use serde_json::Value;
use tauri::http::HeaderName;

#[async_trait]
pub trait ApiClient: Send + Sync {
    /// Creates a new HTTP client based on configuration
    ///
    /// # Arguments
    /// * `proxy_type` - The proxy configuration to use
    ///
    /// # Returns
    /// A Result containing either the configured Client or an error message
    async fn create_client(&self, proxy_type: &ProxyType) -> Result<Client, String>;

    /// Sends a POST request with the given configuration, supporting both regular and streaming responses
    ///
    /// # Arguments
    /// * `config` - The API configuration including URL, authentication, and proxy settings
    /// * `endpoint` - The API endpoint to send the request to
    /// * `body` - The request body as JSON
    /// * `stream` - Whether to handle the response as a stream
    ///
    /// # Returns
    /// A Result containing either the ApiResponse or an error message
    ///
    /// # Example
    /// ```
    /// let client = DefaultApiClient::new();
    /// let config = ApiConfig::new(
    ///     Some("https://api.example.com"),
    ///     Some("your-api-key"),
    ///     ProxyType::None
    /// );
    ///
    /// // For streaming response
    /// let response = client.post_request(
    ///     &config,
    ///     "chat/completions",
    ///     json!({
    ///         "messages": [],
    ///         "stream": true
    ///     }),
    ///     true
    /// ).await?;
    ///
    /// if let Some(raw_response) = response.raw_response {
    ///     while let Some(chunk) = raw_response.chunk().await? {
    ///         // Process streaming chunk
    ///     }
    /// }
    /// ```
    async fn post_request(
        &self,
        config: &ApiConfig,
        endpoint: &str,
        body: Value,
        stream: bool,
    ) -> Result<ApiResponse, String>;

    /// Process a streaming response chunk with specified format
    ///
    /// # Arguments
    /// * `chunk` - The raw bytes of the response chunk
    /// * `format` - The format of the stream data
    ///
    /// # Returns
    /// A Result containing either the processed content or an error message
    ///
    /// # Example
    /// ```
    /// // Using custom format
    /// let custom_parser = Box::new(|chunk: Bytes| -> Result<Option<String>, String> {
    ///     // Custom parsing logic
    ///     Ok(Some("Hello".to_string()))
    /// });
    ///
    /// let format = StreamFormat::Custom(custom_parser);
    /// let result = client.process_stream_chunk(chunk, &format).await?;
    /// ```
    async fn process_stream_chunk(
        &self,
        chunk: Bytes,
        format: &StreamFormat,
    ) -> Result<Vec<StreamChunk>, String> {
        StreamParser::parse_chunk(chunk, format).await
    }
}

#[derive(Clone)]
pub struct DefaultApiClient {
    error_format: ErrorFormat,
}

impl DefaultApiClient {
    /// Creates a new instance of DefaultApiClient
    pub fn new(error_format: ErrorFormat) -> Self {
        Self { error_format }
    }

    /// Builds the request headers from the configuration
    ///
    /// # Arguments
    /// * `config` - The API configuration containing header information
    ///
    /// # Returns
    /// A Result containing either the HeaderMap or an error message
    fn build_headers(&self, config: &ApiConfig) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();

        // Add API key if present and not empty
        if let Some(api_key) =
            config
                .api_key
                .as_ref()
                .and_then(|k| if k.is_empty() { None } else { Some(k) })
        {
            headers.insert(
                HeaderName::from_bytes(b"Authorization")
                    .map_err(|e| t!("network.header_error", error = e.to_string()).to_string())?,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| t!("network.header_error", error = e.to_string()).to_string())?,
            );
        }

        // Add content-type by default
        headers.insert(
            HeaderName::from_bytes(b"Content-Type")
                .map_err(|e| t!("network.header_error", error = e.to_string()).to_string())?,
            HeaderValue::from_static("application/json"),
        );

        // Add custom headers if present
        if let Some(custom_headers) = &config.headers {
            if let Some(obj) = custom_headers.as_object() {
                for (key, value) in obj {
                    if let Some(value_str) = value.as_str() {
                        // Convert the key to a static string to avoid lifetime issues
                        let header_name = HeaderName::from_bytes(key.as_bytes()).map_err(|e| {
                            t!("network.header_error", error = e.to_string()).to_string()
                        })?;

                        headers.insert(
                            header_name,
                            HeaderValue::from_str(value_str).map_err(|e| {
                                t!("network.header_error", error = e.to_string()).to_string()
                            })?,
                        );
                    }
                }
            }
        }

        // Site URL for rankings on openrouter.ai
        headers.insert(
            "HTTP-Referer",
            HeaderValue::from_str("https://github.com/aidyou/chatspeed")
                .map_err(|e| t!("network.header_error", error = e.to_string()).to_string())?,
        );
        // Site title for rankings on openrouter.ai
        headers.insert(
            "X-Title",
            HeaderValue::from_str("Chatspeed")
                .map_err(|e| t!("network.header_error", error = e.to_string()).to_string())?,
        );

        Ok(headers)
    }

    /// Processes the response and handles any errors
    ///
    /// # Arguments
    /// * `response` - The HTTP response to process
    /// * `stream` - Whether to handle the response as a stream
    ///
    /// # Returns
    /// A Result containing either the ApiResponse or an error message
    async fn process_response(
        &self,
        response: Response,
        stream: bool,
    ) -> Result<ApiResponse, String> {
        let status = response.status();

        if !status.is_success() {
            return self.process_error_response(response).await;
        }

        if stream {
            Ok(ApiResponse::success_stream(response))
        } else {
            let content = response.text().await.map_err(|e| {
                t!("network.response_read_error", error = e.to_string()).to_string()
            })?;

            Ok(ApiResponse::success(content))
        }
    }

    async fn process_error_response(&self, response: Response) -> Result<ApiResponse, String> {
        let status_code = response.status().as_u16();
        let inner_type = response
            .status()
            .canonical_reason()
            .unwrap_or("Unknown")
            .to_owned();
        let error_text = response
            .text()
            .await
            .map_err(|e| t!("network.response_read_error", error = e.to_string()).to_string())?;

        let error_message =
            if let Some((mut error_type, message)) = self.error_format.parse_error(&error_text) {
                if error_type.is_empty() {
                    error_type = inner_type;
                }
                log::warn!(
                    "Error response - Status: {}, Type: {}, Message: {}",
                    status_code,
                    error_type,
                    message
                );

                t!(
                    "network.request_failed_with_type",
                    status = status_code.to_string(),
                    error_type = error_type,
                    message = message
                )
                .to_string()
            } else {
                t!(
                    "network.request_failed_with_status",
                    status = status_code.to_string(),
                    message = error_text
                )
                .to_string()
            };

        Ok(ApiResponse::error(error_message))
    }
}

#[async_trait]
impl ApiClient for DefaultApiClient {
    async fn create_client(&self, proxy_type: &ProxyType) -> Result<Client, String> {
        let mut client_builder = Client::builder();

        match proxy_type {
            ProxyType::None => {
                client_builder = client_builder.no_proxy();
            }
            ProxyType::System => {
                // Use system proxy settings (default behavior)
            }
            ProxyType::Http(proxy_url, proxy_username, proxy_password) => {
                let mut proxy = reqwest::Proxy::all(proxy_url)
                    .map_err(|e| t!("network.proxy_error", error = e.to_string()).to_string())?;
                let username = proxy_username.as_deref().unwrap_or_default();
                let password = proxy_password.as_deref().unwrap_or_default();
                if !username.is_empty() && !password.is_empty() {
                    proxy = proxy.basic_auth(username, password);
                }
                client_builder = client_builder.proxy(proxy);
            }
        }

        client_builder
            .build()
            .map_err(|e| t!("network.client_build_error", error = e.to_string()).to_string())
    }

    async fn post_request(
        &self,
        config: &ApiConfig,
        endpoint: &str,
        body: Value,
        stream: bool,
    ) -> Result<ApiResponse, String> {
        let client = self.create_client(&config.proxy_type).await?;
        let headers = self.build_headers(config)?;

        let url = if endpoint.is_empty() {
            config.api_url.as_deref().unwrap_or_default().to_string()
        } else {
            let base_url = config
                .api_url
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('/');
            if !endpoint.starts_with('/') {
                format!("{}/{}", base_url, endpoint)
            } else {
                format!("{}{}", base_url, endpoint)
            }
        };

        #[cfg(debug_assertions)]
        log::debug!("Request URL: {}", url);

        let response = client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| t!("network.request_failed", error = e.to_string()).to_string())?;

        if !response.status().is_success() {
            return self.process_error_response(response).await;
        }

        self.process_response(response, stream).await
    }
}
