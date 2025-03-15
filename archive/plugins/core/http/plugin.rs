//! HTTP Plugin Implementation
//!
//! This module provides a plugin interface for the HTTP client, allowing it to be used
//! within the plugin system. It handles the conversion between JSON configuration and
//! HTTP requests/responses.

use crate::{
    http::{client::HttpClient, types::HttpConfig},
    plugins::{
        traits::{PluginFactory, PluginInfo, PluginType},
        Plugin, PluginError,
    },
};
use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

/// HTTP Plugin that implements the Plugin trait
#[derive(Clone)]
pub struct HttpPlugin {
    plugin_info: PluginInfo,
    client: HttpClient,
    global_config: Option<HttpConfig>,
}

impl HttpPlugin {
    /// Create a new HTTP plugin instance
    ///
    /// # Arguments
    ///
    /// * `init_options` - Optional initialization options
    pub fn new(
        init_options: Option<&Value>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            plugin_info: PluginInfo {
                id: "http_client".to_string(),
                name: "http_client".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            client: HttpClient::new()?,
            global_config: init_options.and_then(|v| serde_json::from_value(v.clone()).ok()),
        })
    }
}

#[async_trait]
impl Plugin for HttpPlugin {
    fn plugin_info(&self) -> &PluginInfo {
        &self.plugin_info
    }

    fn plugin_type(&self) -> &PluginType {
        &PluginType::Native
    }

    async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Store workflow context if needed
        Ok(())
    }

    async fn destroy(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn input_schema(&self) -> Value {
        json!({
            "properties": {
                "url": {
                    "type": "string",
                    "required": true,
                    "description": t!("http.description.url")
                },
                "method": {
                    "type": "string",
                    "required": false,
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "default": "GET",
                    "description": t!("http.description.method")
                },
                "headers": {
                    "type": "object",
                    "required": false,
                    "properties": {
                        "content-type": {
                            "type": "string",
                            "required": false,
                            "description": "Content-Type header"
                        },
                        "user-agent": {
                            "type": "string",
                            "required": false,
                            "description": "User-Agent header"
                        },
                        "referer": {
                            "type": "string",
                            "required": false,
                            "description": "Referer header"
                        }
                    },
                    "description": t!("http.description.headers")
                },
                "body": {
                    "type": ["object", "string", "null"],
                    "required": false,
                    "description": t!("http.description.body")
                }
            }
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "status": {
                "type": "integer",
                "required": true,
                "description": t!("http.description.status")
            },
            "headers": {
                "type": "object",
                "required": true,
                "properties": {
                    "content-type": {
                        "type": "string",
                        "required": false,
                        "description": "Content-Type header"
                    },
                    "content-length": {
                        "type": "string",
                        "required": false,
                        "description": "Content-Length header"
                    }
                },
                "description": t!("http.description.response_headers")
            },
            "body": {
                "type": ["object", "string", "null"],
                "required": false,
                "description": t!("http.description.response_body")
            }
        })
    }

    fn validate_input(
        &self,
        input: &Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(obj) = input.as_object() {
            // 验证必需的 url 字段
            match obj.get("url") {
                Some(url) if url.is_string() => {
                    let url_str = url.as_str().unwrap();
                    if url_str.is_empty() {
                        return Err(Box::new(PluginError::InvalidInput(
                            t!("http.errors.empty_url").to_string(),
                        )));
                    }
                    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
                        return Err(Box::new(PluginError::InvalidInput(
                            t!("http.errors.invalid_url").to_string(),
                        )));
                    }
                }
                _ => {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("http.errors.url_required").to_string(),
                    )));
                }
            }

            // 验证可选的 method 字段
            if let Some(method) = obj.get("method") {
                if !method.is_string() {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("http.errors.method_type").to_string(),
                    )));
                }
                let method_str = method.as_str().unwrap().to_uppercase();
                if !["GET", "POST", "PUT", "DELETE", "PATCH"].contains(&method_str.as_str()) {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("http.errors.invalid_method").to_string(),
                    )));
                }
            }

            // 验证可选的 headers 字段
            if let Some(headers) = obj.get("headers") {
                if !headers.is_object() {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("http.errors.headers_type").to_string(),
                    )));
                }
            }
        } else {
            return Err(Box::new(PluginError::InvalidInput(
                t!("http.errors.invalid_input").to_string(),
            )));
        }

        Ok(())
    }

    fn validate_output(
        &self,
        output: &Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(obj) = output.as_object() {
            // validate status field
            match obj.get("status") {
                Some(status) if status.is_number() => {
                    let status_code = status.as_i64().unwrap();
                    if !(100..=599).contains(&status_code) {
                        return Err(Box::new(PluginError::InvalidOutput(
                            t!("http.errors.invalid_status").to_string(),
                        )));
                    }
                }
                _ => {
                    return Err(Box::new(PluginError::InvalidOutput(
                        t!("http.errors.status_required").to_string(),
                    )));
                }
            }

            // validate headers field
            match obj.get("headers") {
                Some(headers) if headers.is_object() => {}
                _ => {
                    return Err(Box::new(PluginError::InvalidOutput(
                        t!("http.errors.headers_required").to_string(),
                    )));
                }
            }

            // body is optional, but if present, it must be of the correct type
            if let Some(body) = obj.get("body") {
                if !body.is_null() && !body.is_string() && !body.is_object() {
                    return Err(Box::new(PluginError::InvalidOutput(
                        t!("http.errors.invalid_body").to_string(),
                    )));
                }
            }
        } else {
            return Err(Box::new(PluginError::InvalidOutput(
                t!("http.errors.invalid_output").to_string(),
            )));
        }

        Ok(())
    }

    /// Execute HTTP request
    ///
    /// # Arguments
    /// * `input` - Configuration for the HTTP request
    /// * `plugin_info` - Plugin info, it's None for the HTTP plugin
    ///
    /// # Returns
    /// * `Value` - JSON object with the HTTP response
    ///
    async fn execute(
        &mut self,
        input: Option<Value>,
        _plugin_info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.validate_input(input.as_ref().unwrap_or(&json!({})))?;

        let config = match (&self.global_config, input) {
            (Some(global), Some(input)) => {
                // If there's both global and input configuration, merge them
                let mut config = global.clone();
                let input_config: HttpConfig = serde_json::from_value(input).map_err(|e| {
                    Box::new(PluginError::InvalidInput(
                        t!("http.invalid_config", error = e.to_string()).to_string(),
                    )) as Box<dyn std::error::Error + Send + Sync>
                })?;

                // Override global configuration with input configuration
                config.method = input_config.method;
                if !input_config.url.is_empty() {
                    config.url = input_config.url;
                }

                if !input_config.headers.is_empty() {
                    input_config.headers.iter().for_each(|(k, v)| {
                        config.headers.insert(k.clone(), v.clone());
                    });
                }
                if let Some(body) = input_config.body {
                    config.body = Some(body);
                }
                if let Some(timeout) = input_config.timeout {
                    config.timeout = Some(timeout);
                }
                if let Some(max_redirects) = input_config.max_redirects {
                    config.max_redirects = Some(max_redirects);
                }
                config
            }
            (Some(global), None) => global.clone(),
            (None, Some(input)) => serde_json::from_value(input).map_err(|e| {
                Box::new(PluginError::InvalidInput(
                    t!("http.invalid_config", error = e.to_string()).to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?,
            (None, None) => {
                return Err(
                    PluginError::InvalidInput(t!("http.missing_config").to_string()).into(),
                );
            }
        };

        // Execute HTTP request
        let response = self.client.send_request(config)?;
        let output = serde_json::to_value(response).map_err(|e| {
            Box::new(PluginError::RuntimeError(
                t!("http.serialize_failed", error = e.to_string()).to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        self.validate_output(&output)?;

        Ok(output)
    }
}

/// Factory for creating HTTP plugin instances
pub struct HttpPluginFactory;

impl HttpPluginFactory {
    /// Create a new HTTP plugin factory
    pub fn new() -> Self {
        Self
    }
}

impl PluginFactory for HttpPluginFactory {
    fn create_instance(
        &self,
        init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(HttpPlugin::new(init_options)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_plugin_execution() {
        let mut plugin = HttpPlugin::new(None).unwrap();

        // Test with valid config
        let input = serde_json::json!({
            "url": "https://api.example.com",
            "method": "GET"
        });

        let result = plugin.execute(Some(input), None).await;
        assert!(result.is_ok());
    }
}
