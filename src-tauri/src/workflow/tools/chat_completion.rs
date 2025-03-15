use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    ai::interaction::chat_completion::{complete_chat_blocking, ChatProtocol, ChatState},
    db::AiModel,
    workflow::{
        context::Context,
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

/// A function that sends an HTTP request.
pub struct ChatCompletion {
    chat_state: Arc<ChatState>,
    models: Arc<tokio::sync::RwLock<HashMap<String, AiModel>>>,
}

impl ChatCompletion {
    /// Creates a new instance of the `ChatCompletion` function.
    pub fn new(state: Arc<ChatState>) -> Self {
        Self {
            chat_state: state,
            models: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_model(&self, name: &str, model: AiModel) {
        self.models.write().await.insert(name.to_string(), model);
    }

    pub async fn get_model(&self, name: &str) -> Option<AiModel> {
        self.models.read().await.get(name).cloned()
    }
}

#[async_trait]
impl FunctionDefinition for ChatCompletion {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "chat_completion"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::Http
    }

    /// Returns a brief description of the function.
    fn description(&self) -> &str {
        "Execute a chat completion operation"
    }

    /// Get the function calling specification
    ///
    /// Returns a JSON object containing the function calling specification
    /// Get the function calling specification
    fn function_calling_spec(&self) -> serde_json::Value {
        json!({
            "name": "chat_completion",
            "description": "Perform chat completion operation",
            "parameters": {
                "type": "object",
                "properties": {
                    "model_name": {
                        "type": "string",
                        "enum": ["reasoning", "general"],
                        "description": "Name of the model to be used. Must be a model name registered in the workflow. Options: 'reasoning' (for complex tasks like planning and analysis), 'general' (for text processing tasks like summarization)."
                    },
                    "chat_id": {
                        "type": "string",
                        "description": "Optional chat ID"
                    },
                   "messages": {
                       "type": "array",
                       "description": "List of messages. Each message should be an object containing 'role' and 'content' fields. The 'role' can be 'system', 'user', or 'assistant', and 'content' is the message text.",
                       "items": {
                           "type": "object",
                           "properties": {
                               "role": {
                                   "type": "string",
                                   "enum": ["system", "user", "assistant"],
                                   "description": "Role of the message sender. Can be 'system', 'user', or 'assistant'."
                               },
                               "content": {
                                   "type": "string",
                                   "description": "Content of the message."
                               }
                           },
                           "required": ["role", "content"]
                       }
                   }
                },
                "required": ["model_name", "messages"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The generated chat completion content."
                    }
                },
                "description": "The response containing the generated chat completion content."
            }
        })
    }

    /// Executes the function with the given parameters and context.
    ///
    /// # Arguments
    /// * `params` - The parameters of the function.
    /// * `_context` - The context of the function.
    ///
    /// # Returns
    /// Returns a `FunctionResult` containing the result of the function execution.
    async fn execute(&self, params: Value, _context: &Context) -> FunctionResult {
        let model_name = params["model_name"].as_str().ok_or_else(|| {
            WorkflowError::FunctionParamError("model_name must be a string".to_string())
        })?;
        let model = self.get_model(model_name).await.ok_or_else(|| {
            WorkflowError::FunctionParamError(format!(
                "model {} not found in workflow models",
                model_name
            ))
        })?;

        let chat_id = params["chat_id"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or(uuid::Uuid::new_v4().to_string());
        let messages = params["messages"].as_array().ok_or_else(|| {
            WorkflowError::FunctionParamError("messages must be an array".to_string())
        })?;

        let content = complete_chat_blocking(
            &self.chat_state,
            model.api_protocol.try_into()?,
            Some(model.base_url.as_str()),
            model.default_model,
            Some(model.api_key.as_str()),
            chat_id,
            messages.to_vec(),
            model.metadata,
        )
        .await?;

        Ok(json!({
            "content": content,
        }))
    }
}

mod tests {
    use std::{env, sync::Arc};

    use crate::{
        ai::interaction::chat_completion::ChatState,
        libs::window_channels::WindowChannels,
        logger,
        workflow::{
            context::Context, function_manager::FunctionDefinition,
            tools::chat_completion::ChatCompletion,
        },
    };
    use serde_json::json;

    #[tokio::test]
    async fn test_chat_completion_execute() {
        for (key, value) in std::env::vars() {
            println!("{}: {}", key, value);
        }

        let chat_state = Arc::new(ChatState::new(Arc::new(WindowChannels::new())));
        let chat_completion = ChatCompletion::new(chat_state);

        let params = json!({
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

        let result = chat_completion.execute(params, &Context::new()).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.get("content").is_some());
        dbg!(response);
    }
}
