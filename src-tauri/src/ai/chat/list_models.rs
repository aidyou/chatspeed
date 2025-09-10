use chrono::{DateTime, Utc};
use rust_i18n::t;
use serde::Deserialize;
use serde_json::{from_str, json, Value};

use crate::{
    ai::{
        error::AiError,
        network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat},
        traits::chat::ModelDetails,
        util::{
            get_family_from_model_id, get_proxy_type, is_function_call_supported,
            is_image_input_supported, is_reasoning_supported,
        },
    },
    ccproxy::ChatProtocol,
};

const OPENAI_DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
const CALUDE_BASE_URL: &str = "https://api.anthropic.com/v1";
const GEMINI_DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Deserialize, Debug)]
struct OpenAIModel {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owned_by: Option<String>,
    // We can add more fields if needed from the OpenAI response, like 'permission'
}

#[derive(Deserialize, Debug)]
struct OpenAIListModelsResponse {
    data: Vec<OpenAIModel>,
}

#[derive(serde::Deserialize, Debug)]
struct ClaudeApiModel {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_token_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_token_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_tools: Option<bool>,
}

#[derive(serde::Deserialize, Debug)]
struct ClaudeListModelsResponse {
    data: Vec<ClaudeApiModel>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiModel {
    name: String,         // e.g., "models/gemini-1.5-pro-latest"
    display_name: String, // e.g., "Gemini 1.5 Pro"
    description: Option<String>,
    #[serde(default)] // Make optional as not all models might have it explicitly
    input_token_limit: Option<u32>,
    #[serde(default)]
    output_token_limit: Option<u32>,
    supported_generation_methods: Vec<String>,
    #[serde(default)]
    version: Option<String>,
    // temperature, topP, topK could also be here if needed
}

#[derive(Deserialize, Debug)]
struct GeminiListModelsResponse {
    models: Vec<GeminiModel>,
}

pub async fn openai_list_models(
    api_url: Option<&str>,
    api_key: Option<&str>,
    extra_args: Option<Value>,
) -> Result<Vec<ModelDetails>, AiError> {
    let base_url = api_url.unwrap_or(OPENAI_DEFAULT_API_BASE);
    let mut effective_api_key = api_key.map(String::from);

    let mut custom_headers = json!({});
    if let Some(args) = &extra_args {
        if let Some(key_from_extra) = args.get("api_key").and_then(|v| v.as_str()) {
            if !key_from_extra.is_empty() {
                effective_api_key = Some(key_from_extra.to_string());
            }
        }
        if let Some(org_id) = args.get("organization").and_then(|v| v.as_str()) {
            if !org_id.is_empty() {
                custom_headers
                    .as_object_mut()
                    .unwrap()
                    .insert("OpenAI-Organization".to_string(), json!(org_id));
            }
        }
    }

    let config = ApiConfig::new(
        Some(base_url.to_string()),
        effective_api_key, // Handled by DefaultApiClient for Bearer token
        get_proxy_type(extra_args.clone()), // Pass extra_args for proxy settings
        if custom_headers.as_object().map_or(true, |m| m.is_empty()) {
            None
        } else {
            Some(custom_headers)
        },
    );

    let client = DefaultApiClient::new(ErrorFormat::OpenAI);
    let response = client
        .get_request(&config, "/models", None)
        .await
        .map_err(|e| AiError::ApiRequestFailed {
            provider: "OpenAI".to_string(),
            details: e,
        })?;

    if response.is_error || response.content.is_empty() {
        return Err(AiError::ApiRequestFailed {
            provider: "OpenAI".to_string(),
            details: response.content,
        });
    }

    #[cfg(debug_assertions)]
    log::debug!("OpenAI list_models response: {}", &response.content);

    let models_response: OpenAIListModelsResponse = serde_json::from_str(&response.content)
        .map_err(|e| {
            log::error!(
                "Failed to parse OpenAI models response: {}, content:{}",
                e,
                &response.content
            );
            AiError::ResponseParseFailed {
                provider: "OpenAI".to_string(),
                details: e.to_string(),
            }
        })?;

    let model_details: Vec<ModelDetails> = models_response
        .data
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, model| {
            acc.insert(model.id.to_lowercase(), model);
            acc
        })
        .into_values()
        .map(|model| {
            let id = model.id.to_lowercase();

            ModelDetails {
                id: model.id.clone(),
                name: model.id, // OpenAI API doesn't provide a separate "friendly name"
                protocol: ChatProtocol::OpenAI,
                max_input_tokens: None,  // Not provided by /v1/models
                max_output_tokens: None, // Not provided by /v1/models
                description: Some(format!(
                    "Owned by: {}",
                    model.owned_by.as_deref().unwrap_or("unknown")
                )),
                last_updated: DateTime::from_timestamp(model.created.unwrap_or_default(), 0)
                    .map(|dt: DateTime<Utc>| dt.to_rfc3339()),
                family: get_family_from_model_id(&id),
                function_call: Some(is_function_call_supported(&id)),
                reasoning: Some(is_reasoning_supported(&id)),
                image_input: Some(is_image_input_supported(&id)),
                metadata: Some(json!({
                    "object": model.object.unwrap_or_default(),
                    "owned_by": model.owned_by.unwrap_or_default(),
                })),
            }
        })
        .collect();

    Ok(model_details)
}

/// Lists available models of claude
///
/// # Arguments
/// * `api_url` - Optional API endpoint URL
/// * `api_key` - Optional API key
/// * `extra_params` - Additional parameters including proxy settings
///
/// # Returns
/// * `Vec<ModelDetails>` - List of available models
/// * `AiError` - Error if the request fails
pub async fn claude_list_models(
    api_url: Option<&str>,
    api_key: Option<&str>,
    extra_params: Option<Value>,
) -> Result<Vec<ModelDetails>, AiError> {
    let headers = json!({
        "x-api-key": api_key.unwrap_or(""),
        "anthropic-version": "2023-06-01", // Consistent with chat endpoint
    });

    let query_params = json!({
        "limit": 500 // Fetch up to 100 models, adjust if needed
    });

    let client = DefaultApiClient::new(ErrorFormat::Claude);
    let response = client
        .get_request(
            &ApiConfig::new(
                Some(api_url.unwrap_or(CALUDE_BASE_URL).to_string()),
                None,
                get_proxy_type(extra_params),
                Some(headers),
            ),
            "models", // Endpoint path
            Some(query_params),
        )
        .await
        .map_err(|network_err| {
            let err = AiError::ApiRequestFailed {
                provider: "Claude".to_string(),
                details: network_err.to_string(),
            };
            log::error!("Claude list_models API request failed: {}", err);
            err
        })?;

    if response.is_error || response.content.is_empty() {
        let err = AiError::ApiRequestFailed {
            provider: "Claude".to_string(),
            details: response.content,
        };
        log::error!("{}", err);
        return Err(err);
    }

    #[cfg(debug_assertions)]
    log::debug!("Claude list_models response: {}", &response.content);

    let api_response: ClaudeListModelsResponse = from_str(&response.content).map_err(|e| {
        let err = AiError::ResponseParseFailed {
            provider: "Claude".to_string(),
            details: e.to_string(),
        };
        log::error!("Claude list_models response parsing failed: {}", err);
        err
    })?;

    let models = api_response
        .data
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, model| {
            acc.insert(model.id.to_lowercase(), model);
            acc
        })
        .into_values()
        .map(|api_model| {
            let model_id = api_model.id.to_lowercase();
            ModelDetails {
                id: api_model.id.clone(),
                name: api_model.display_name.unwrap_or(model_id.clone()),
                protocol: ChatProtocol::Claude,
                max_input_tokens: api_model.input_token_limit,
                max_output_tokens: api_model.output_token_limit,
                description: api_model.description,
                last_updated: api_model.created_at, // Use created_at from API
                family: get_family_from_model_id(&model_id),
                // Prioritize API's supports_tools, fallback to helper function if not present
                function_call: api_model
                    .supports_tools
                    .or_else(|| Some(is_function_call_supported(&model_id))),
                reasoning: Some(is_reasoning_supported(&model_id)),
                image_input: Some(is_image_input_supported(&model_id)),
                metadata: None,
            }
        })
        .collect();

    Ok(models)
}

pub async fn gemini_list_models(
    api_url: Option<&str>,
    api_key: Option<&str>, // This is the Gemini API Key
    extra_args: Option<Value>,
) -> Result<Vec<ModelDetails>, AiError> {
    let base_url = api_url.unwrap_or(GEMINI_DEFAULT_API_BASE);

    if api_key.is_none() || api_key.as_deref().unwrap_or("").is_empty() {
        return Err(AiError::InvalidInput(
            t!(
                "chat.api_key_is_require_for_list_models",
                provider = "Gemini"
            )
            .to_string(),
        ));
    }
    let endpoint_with_key = format!("/models?key={}", api_key.unwrap_or_default());

    // For Gemini, API key is in query param, so ApiConfig.api_key should be None
    // to prevent DefaultApiClient from adding a "Bearer" token.
    let config = ApiConfig::new(
        Some(base_url.to_string()),
        None,
        get_proxy_type(extra_args.clone()),
        None, // No special headers needed if key is in URL for this request
    );

    let client = DefaultApiClient::new(ErrorFormat::Google);
    let response = client
        .get_request(&config, &endpoint_with_key, None)
        .await
        .map_err(|e| AiError::ApiRequestFailed {
            provider: "Gemini".to_string(),
            details: e.to_string(), // Changed from e to e.to_string()
        })?;

    if response.is_error || response.content.is_empty() {
        return Err(AiError::ApiRequestFailed {
            provider: "Gemini".to_string(),
            details: response.content,
        });
    }

    #[cfg(debug_assertions)]
    log::debug!("Gemini list_models response: {}", &response.content);

    let models_response: GeminiListModelsResponse = serde_json::from_str(&response.content)
        .map_err(|e| AiError::ResponseParseFailed {
            provider: "Gemini".to_string(),
            details: e.to_string(),
        })?;

    let model_details: Vec<ModelDetails> = models_response
        .models
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut acc, model| {
            acc.insert(model.name.clone(), model);
            acc
        })
        .into_values()
        // Filter for models that support 'generateContent', common for chat
        .filter(|m| {
            m.supported_generation_methods
                .contains(&"generateContent".to_string())
        })
        .map(|model| {
            let id_for_utils = model.name.to_lowercase();

            ModelDetails {
                id: model
                    .name
                    .clone()
                    .strip_prefix("models/")
                    .map(|s| s.to_string())
                    .unwrap_or_default(), // e.g. "models/gemini-1.5-pro-latest"
                name: model.display_name.clone(),
                protocol: ChatProtocol::Gemini,
                max_input_tokens: model.input_token_limit,
                max_output_tokens: model.output_token_limit,
                description: model.description,
                last_updated: model.version.clone(), // Use version as a proxy for update info
                family: get_family_from_model_id(&id_for_utils),
                function_call: Some(is_function_call_supported(&id_for_utils)),
                reasoning: Some(is_reasoning_supported(&id_for_utils)),
                image_input: Some(is_image_input_supported(&id_for_utils)),
                metadata: Some(json!({
                    "version": model.version,
                    "supported_generation_methods": model.supported_generation_methods,
                })),
            }
        })
        .collect();

    Ok(model_details)
}
