use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

use super::ModelName;
use crate::{
    ai::{interaction::chat_completion::complete_chat_blocking, traits::chat::MCPToolDeclaration},
    db::AiModel,
    tools::{error::ToolError, NativeToolResult, ToolCallResult, ToolDefinition},
};

/// A function that sends an HTTP request.
pub struct ChatCompletion {
    models: Arc<tokio::sync::RwLock<HashMap<ModelName, AiModel>>>,
}

impl ChatCompletion {
    /// Creates a new instance of the `ChatCompletion` function.
    pub fn new() -> Self {
        Self {
            models: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_model(&self, name: ModelName, model: AiModel) {
        self.models.write().await.insert(name, model);
    }

    pub async fn get_model(&self, name: ModelName) -> Option<AiModel> {
        self.models.read().await.get(&name).cloned()
    }
}

#[async_trait]
impl ToolDefinition for ChatCompletion {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "chat_completion"
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Generate text using a language model"
    }

    /// Get the function calling specification
    ///
    /// Returns a JSON object containing the function calling specification
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "model_name": {
                        "type": "string",
                        "enum": ["reasoning", "general"],
                        "description": "Model type: 'reasoning' (planning/analysis) or 'general' (text processing)"
                    },
                    "chat_id": {
                        "type": "string",
                        "description": "Optional chat ID"
                    },
                    "messages": {
                        "type": "array",
                        "description": "Message list with role and content",
                        "items": {
                            "type": "object",
                            "properties": {
                                "role": {
                                    "type": "string",
                                    "enum": ["system", "user", "assistant", "function"],
                                    "description": "Message sender role"
                                },
                                "content": {
                                    "type": "string",
                                    "description": "Message text"
                                }
                            },
                            "required": ["role", "content"]
                        }
                    },
                    "max_tokens": {
                        "type": "integer",
                        "description": "Maximum tokens to generate"
                    },
                    "temperature": {
                        "type": "number",
                        "description": "Sampling temperature"
                    },
                    "top_p": {
                        "type": "number",
                        "description": "Top-p sampling value"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Top-k sampling value"
                    }
                },
                "required": ["model_name", "messages"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    /// Executes the function with the given parameters.
    ///
    /// # Arguments
    /// * `params` - The parameters of the function.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the result of the function execution.
    async fn call(&self, params: Value) -> NativeToolResult {
        let model_param = params["model_name"].as_str().ok_or_else(|| {
            ToolError::FunctionParamError(t!("tools.model_name_must_be_string").to_string())
        })?;
        let model_name = ModelName::try_from(model_param)
            .map_err(|e| ToolError::FunctionParamError(e.to_string()))?;
        let mut model = self.get_model(model_name).await.ok_or_else(|| {
            ToolError::FunctionParamError(
                t!("tools.model_not_found", model_name = model_param).to_string(),
            )
        })?;

        if let Some(max_tokens) = params["max_tokens"].as_i64() {
            if max_tokens > 0 {
                model.max_tokens = max_tokens as i32;
            }
        }
        if let Some(temperature) = params["temperature"].as_f64() {
            if temperature >= 0.0 {
                model.temperature = temperature as f32;
            }
        }
        if let Some(top_p) = params["top_p"].as_f64() {
            if top_p > 0.0 {
                model.top_p = top_p as f32;
            }
        }
        if let Some(top_k) = params["top_k"].as_i64() {
            if top_k > 0 {
                model.top_k = top_k as i32;
            }
        }

        let chat_id = params["chat_id"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or(uuid::Uuid::new_v4().to_string());
        let messages = params["messages"].as_array().ok_or_else(|| {
            ToolError::FunctionParamError(t!("tools.messages_must_be_array").to_string())
        })?;

        let tools = params
            .get("tools")
            .and_then(|v| serde_json::from_value::<Vec<MCPToolDeclaration>>(v.clone()).ok());

        let result = complete_chat_blocking(
            model
                .api_protocol
                .try_into()
                .map_err(|e| ToolError::Initialization(e))?,
            Some(model.base_url.as_str()),
            model.default_model,
            Some(model.api_key.as_str()),
            chat_id,
            messages.to_vec(),
            tools,
            model.metadata,
        )
        .await
        .map_err(|e_str| {
            ToolError::Execution(t!("tools.chat_completion_failed", details = e_str).to_string())
        })?;

        if result.content.is_empty() {
            return Err(ToolError::Execution(
                t!("tools.chat_completion_no_content").to_string(),
            ));
        }
        log::debug!(
            "Chat completion result: {:#?}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );

        Ok(ToolCallResult::success(
            Some(result.content.clone()),
            Some(json!(&result)),
        ))
    }
}

mod tests {
    #[tokio::test]
    async fn test_chat_completion_execute() {
        for (key, value) in std::env::vars() {
            println!("{}: {}", key, value);
        }

        let chat_completion = crate::tools::chat_completion::ChatCompletion::new();

        let params = serde_json::json!({
            "chat_protocol": "openai",
            "api_url":std::env::var("TEST_AI_URL").expect("Env value TEST_AI_URL not set"),
            "api_key": std::env::var("TEST_AI_API_KEY").expect("Env value TEST_AI_API_KEY not set"),
            "model": std::env::var("TEST_AI_MODEL").expect("Env value TEST_AI_MODEL not set"),
            "metadata": {
                "maxTokens": 1000,
                "temperature": 0.5,
            },
            "messages": [
                {"role": "user", "content": "Hello, who are you?"},
            ]
        });

        let result = crate::tools::ToolDefinition::call(&chat_completion, params).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.content.is_some());
        dbg!(response);
    }
}
