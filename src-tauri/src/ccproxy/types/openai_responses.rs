use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::openai::{OpenAIResponseFormat, OpenAITool};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<OpenAIResponsesInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<OpenAIResponsesInstructions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAIResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<OpenAIResponsesTextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenAIResponsesReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum OpenAIResponsesInstructions {
    Text(String),
    Items(Vec<OpenAIResponsesInputItem>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum OpenAIResponsesInput {
    Text(String),
    Items(Vec<OpenAIResponsesInputItem>),
}

impl Default for OpenAIResponsesInput {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesInputItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAIResponsesContent>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum OpenAIResponsesContent {
    Text(String),
    Parts(Vec<OpenAIResponsesContentPart>),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum OpenAIResponsesTool {
    Function(OpenAIResponsesFunctionTool),
    Chat(OpenAITool),
    Other(Value),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponsesFunctionTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesTextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<OpenAIResponseFormat>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponsesResponse {
    pub id: String,
    pub object: String,
    pub created_at: u64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<Value>,
    pub model: String,
    pub output: Vec<OpenAIResponsesOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAIResponsesUsage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponsesOutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<OpenAIResponsesOutputContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<OpenAIResponsesReasoningSummary>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponsesOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponsesReasoningSummary {
    #[serde(rename = "type")]
    pub summary_type: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<OpenAIResponsesInputTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OpenAIResponsesOutputTokensDetails>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesInputTokensDetails {
    pub cached_tokens: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIResponsesOutputTokensDetails {
    pub reasoning_tokens: u64,
}
