use super::stoppable::Stoppable;
use async_trait::async_trait;
use serde_json::Value;
use std::error::Error;

#[async_trait]
pub trait AiChatTrait: Send + Sync + Stoppable {
    /// Sends a chat request to the AI API and processes the response.
    ///
    /// # Parameters
    /// - `api_url`: The URL of the AI API endpoint.
    /// - `model`: The model to be used for the chat.
    /// - `api_key`: The API key for authentication.
    /// - `messages`: A vector of messages in the expected format for the AI API.
    ///   Each message is represented as a JSON object containing:
    ///   - `role`: A string indicating the role of the message sender.
    ///     Supported roles include:
    ///     - `user`: Represents the input message from the user.
    ///     - `assistant`: Represents the response message from the AI assistant.
    ///     - `system`: Used for system-level instructions or context setting.
    ///   - `content`: A string containing the actual message content.
    /// - `extra_params`: An optional extra parameters for the request.
    ///     - `max_tokens`: An optional maximum number of tokens to generate in the response.
    ///     - `stream`: A boolean indicating whether to stream the response. This parameter depends on whether the AI interface supports streaming responses.
    ///     - `temperature`: A float number indicating the temperature of the response. This parameter depends on the API support.
    ///     - `top_p`: A float number indicating the top_p of the response. This parameter depends on the API support.
    ///     - `top_k`: An integer indicating the top_k of the response. This parameter depends on the API support.
    ///     - `proxy_type`: A string indicating the proxy type. valid values are `system`, `http`, `none`.
    ///     - `proxy_server`: A string indicating the proxy server. like `http://127.0.0.1:7890`.
    ///     - Additional parameters that may be included in the callback function, like label.
    /// - `callback`: A callback function to handle streaming responses.
    ///     - `content`: `String`: The content of the message.
    ///     - `is_error`: `bool`: A boolean indicating whether the message is a error message.
    ///     - `is_finished`: `bool`: A boolean indicating whether the message is finished.
    ///     - `is_reasoning`: `bool`: A boolean indicating whether the message is a reasoning message.
    ///     - `Value`: Additional parameters that may be included in the callback function, like label.
    ///
    /// # Returns
    /// - A `Result` containing the full response as a `String` or an error if the request fails.
    async fn chat(
        &self,
        api_url: Option<&str>,
        model: &str,
        api_key: Option<&str>,
        messages: Vec<Value>,
        extra_params: Option<Value>,
        callback: impl Fn(String, bool, bool, bool, Option<Value>) + Send + 'static,
    ) -> Result<String, Box<dyn Error + Send + Sync>>;
}
