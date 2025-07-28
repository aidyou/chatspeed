use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// =================================================
// Gemini request structs
// =================================================
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<GeminiToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content: Option<String>, // Context cache content name
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiContent {
    pub role: String, // "user" or "model"
    // Add `serde(default)` to handle cases where the `parts` field is omitted in stream chunks,
    // such as in metadata-only chunks at the end of a stream.
    #[serde(default)]
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<GeminiInlineData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<FileData>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "functionCall")]
    pub function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "functionResponse")]
    pub function_response: Option<GeminiFunctionResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_metadata: Option<GeminiVideoMetadata>, // Video content
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionResponse {
    pub name: String,
    pub response: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiInlineData {
    pub mime_type: String,
    pub data: String, // base64 encoded string
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiTool {
    pub function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiToolConfig {
    pub function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCallingConfig {
    pub mode: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>, // Range: 0.0 to 1.0, default: 0.9
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>, // Range: 0.0 to 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>, // Sample from top K options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>, // Custom stop sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>, // "application/json" for JSON mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>, // JSON schema for structured output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<GeminiThinkingConfig>, // Extended thinking configuration
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Value, // Gemini args are typically a JSON object
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
    pub usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default, rename = "promptFeedback")]
    pub prompt_feedback: Option<GeminiPromptFeedback>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>, // Model version used for generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>, // RFC 3339 timestamp when request was sent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>, // ID that identifies each response
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GeminiCandidate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>, // Index of the candidate
    pub content: GeminiContent,
    #[serde(default, rename = "finishReason")]
    pub finish_reason: Option<String>, // "STOP", "MAX_TOKENS", "SAFETY", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>, // Safety ratings for the candidate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_metadata: Option<GeminiCitationMetadata>, // Source attribution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_metadata: Option<GeminiGroundingMetadata>, // Grounding sources metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_logprobs: Option<f64>, // Average log probability score
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_message: Option<String>, // Detailed description of stop reason
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiUsageMetadata {
    pub prompt_token_count: u64,
    pub total_token_count: u64,
    #[serde(default, deserialize_with = "deserialize_token_count")]
    pub candidates_token_count: Option<u64>, // Handle both u64 and object types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_prompt_token_count: Option<u64>, // Tokens used for tool use prompts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thoughts_token_count: Option<u64>, // Tokens used for thinking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content_token_count: Option<u64>, // Tokens from cached content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<Vec<GeminiModalityTokenCount>>, // Detailed prompt token breakdown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_tokens_details: Option<Vec<GeminiModalityTokenCount>>, // Detailed candidate token breakdown
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPromptFeedback {
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_reason_metadata: Option<Value>,
    pub safety_ratings: Vec<GeminiSafetyRating>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeminiSafetyRating {
    pub category: String,
    pub probability: String, // e.g., "NEGLIGIBLE", "LOW", "MEDIUM", "HIGH"
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String, // e.g., "BLOCK_NONE", "BLOCK_LOW_AND_ABOVE"
}

/// Video metadata for Gemini parts
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiVideoMetadata {
    pub video_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_offset_millis: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_offset_millis: Option<i64>,
}

/// Extended thinking configuration for Gemini
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiThinkingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<i32>, // Token budget for internal reasoning, 0 disables thinking
}

/// Citation metadata for generated content
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiCitationMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_sources: Option<Vec<GeminiCitationSource>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiCitationSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// Grounding metadata for content sources
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGroundingMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_queries: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_chunks: Option<Vec<GeminiGroundingChunk>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_supports: Option<Vec<GeminiGroundingSupport>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGroundingChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<GeminiWebChunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieved_context: Option<GeminiRetrievedContext>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiWebChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiRetrievedContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGroundingSupport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_chunk_indices: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_scores: Option<Vec<f64>>,
}

/// Token count information for a single modality
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeminiModalityTokenCount {
    pub modality: String, // "TEXT", "IMAGE", "VIDEO", "AUDIO"
    pub token_count: u64,
}

/// Custom deserializer for token count fields that might be either u64 or an object
fn deserialize_token_count<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum TokenCount {
        Number(u64),
        Object(Value),
    }

    match Option::<TokenCount>::deserialize(deserializer)? {
        Some(TokenCount::Number(n)) => Ok(Some(n)),
        Some(TokenCount::Object(obj)) => {
            // Try to extract token count from object if it has a specific structure
            if let Some(count) = obj.get("count").and_then(|v| v.as_u64()) {
                Ok(Some(count))
            } else if let Some(count) = obj.as_u64() {
                Ok(Some(count))
            } else {
                // If we can't extract a meaningful count, return None
                log::warn!("Unexpected token count object format: {:?}", obj);
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

impl GeminiRequest {
    /// Validate request parameters according to Gemini API constraints
    pub fn validate(&self) -> Result<(), String> {
        // Validate generation config if present
        if let Some(ref config) = self.generation_config {
            // Validate temperature range (0.0 to 1.0)
            if let Some(temp) = config.temperature {
                if temp < 0.0 || temp > 1.0 {
                    return Err("Temperature must be between 0.0 and 1.0".to_string());
                }
            }

            // Validate top_p range (0.0 to 1.0)
            if let Some(top_p) = config.top_p {
                if top_p < 0.0 || top_p > 1.0 {
                    return Err("top_p must be between 0.0 and 1.0".to_string());
                }
            }

            // Validate top_k is non-negative
            if let Some(top_k) = config.top_k {
                if top_k < 0 {
                    return Err("top_k must be non-negative".to_string());
                }
            }

            // Validate max_output_tokens is positive
            if let Some(max_tokens) = config.max_output_tokens {
                if max_tokens <= 0 {
                    return Err("max_output_tokens must be positive".to_string());
                }
            }

            // Validate thinking budget if present
            if let Some(ref thinking) = config.thinking_config {
                if let Some(budget) = thinking.thinking_budget {
                    if budget < 0 {
                        return Err("thinking_budget must be non-negative".to_string());
                    }
                }
            }
        }

        // Validate contents is not empty
        if self.contents.is_empty() {
            return Err("Contents cannot be empty".to_string());
        }

        // Validate each content has valid role
        for content in &self.contents {
            if content.role != "user" && content.role != "model" {
                return Err("Content role must be 'user' or 'model'".to_string());
            }

            if content.parts.is_empty() {
                return Err("Content parts cannot be empty".to_string());
            }
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileData {
    pub mime_type: String,
    pub file_uri: String, // e.g., "gs://bucket/path/to/file.png"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_gemini_usage_metadata_deserialization() {
        // Test with normal u64 values
        let normal_json = json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": 20,
            "totalTokenCount": 30
        });

        let usage: GeminiUsageMetadata = serde_json::from_value(normal_json).unwrap();
        assert_eq!(usage.prompt_token_count, 10);
        assert_eq!(usage.candidates_token_count, Some(20));
        assert_eq!(usage.total_token_count, 30);

        // Test with object format for candidatesTokenCount
        let object_json = json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": {
                "count": 25
            },
            "totalTokenCount": 35
        });

        let usage: GeminiUsageMetadata = serde_json::from_value(object_json).unwrap();
        assert_eq!(usage.prompt_token_count, 10);
        assert_eq!(usage.candidates_token_count, Some(25));
        assert_eq!(usage.total_token_count, 35);

        // Test with missing candidatesTokenCount
        let missing_json = json!({
            "promptTokenCount": 10,
            "totalTokenCount": 10
        });

        let usage: GeminiUsageMetadata = serde_json::from_value(missing_json).unwrap();
        assert_eq!(usage.prompt_token_count, 10);
        assert_eq!(usage.candidates_token_count, None);
        assert_eq!(usage.total_token_count, 10);
    }

    #[test]
    fn test_gemini_response_deserialization() {
        let response_json = json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {
                                "text": "Hello, this is a test response."
                            }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0,
                    "safetyRatings": []
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": {
                    "count": 20
                },
                "totalTokenCount": 30
            }
        });

        let response: GeminiResponse = serde_json::from_value(response_json).unwrap();
        assert!(response.candidates.is_some());
        assert!(response.usage_metadata.is_some());

        let usage = response.usage_metadata.unwrap();
        assert_eq!(usage.prompt_token_count, 10);
        assert_eq!(usage.candidates_token_count, Some(20));
        assert_eq!(usage.total_token_count, 30);
    }
}
