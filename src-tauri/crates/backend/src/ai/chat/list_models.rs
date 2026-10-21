use crate::error_diagnostics::{
    log_error_diagnostic, project_user_message, ErrorOrigin, ErrorProjection, FailureMessage,
};
use chatspeed_contracts::ErrorReason;
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
            get_family_from_model_id, get_proxy_type_for_key, is_function_call_supported,
            is_image_input_supported, is_reasoning_supported,
        },
    },
    ccproxy::ChatProtocol,
    model_catalog_engine::resolve_model_profile,
};

const OPENAI_DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
const CALUDE_BASE_URL: &str = "https://api.anthropic.com/v1";
const GEMINI_DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

fn is_known_non_chat_model(model_id: &str) -> bool {
    [
        "embedding",
        "rerank",
        "moderation",
        "whisper",
        "tts",
        "dall-e",
        "image-generation",
        "text-to-image",
        "speech-to-text",
    ]
    .iter()
    .any(|marker| model_id.contains(marker))
}

/// Describes the real upstream answer without changing the legacy AiError status.
///
/// The network layer already records the exact HTTP status and body. The list
/// models caller keeps its historical retry classification, while the error
/// origin exposes the safe status and message the UI needs (notably a 405 that
/// means the provider has no list-models endpoint).
fn response_error_origin(
    response: &crate::ai::network::ApiResponse,
    provider: &str,
) -> ErrorOrigin {
    let status_code = response.status_code;
    let body = response
        .raw_error_body
        .as_deref()
        .unwrap_or(&response.content);
    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    let message = project_user_message(&parsed, format!("{provider} returned HTTP {status_code}"));
    let reason = (status_code == 405).then_some(ErrorReason::UnsupportedListModels);
    ErrorOrigin::leaf(message, reason, None, Some(status_code)).context(
        "ai.network",
        "list_models",
        None,
    )
}

fn enrich_model_details(mut details: ModelDetails) -> Result<ModelDetails, AiError> {
    let catalog = resolve_model_profile(&details.id).map_err(|error| {
        AiError::InitFailed(
            FailureMessage::leaf(format!("model catalog resolution failed: {error}"))
                .with_origin(ErrorOrigin::from_error(&error)),
        )
    })?;

    if details.family.is_none() {
        details.family = catalog.family;
    }
    if details.reasoning.is_none() {
        details.reasoning = catalog.capabilities.reasoning;
    }
    if details.function_call.is_none() {
        details.function_call = catalog.capabilities.function_call;
    }
    if details.image_input.is_none() {
        details.image_input = catalog.capabilities.image_input;
    }
    if details.recommended_temperature.is_none() {
        details.recommended_temperature = catalog.recommended_temperature;
    }
    if details.max_output_tokens.is_none() {
        details.max_output_tokens = catalog.max_output_tokens;
    }

    let model_id = details.id.to_ascii_lowercase();

    if details.family.is_none() {
        details.family = get_family_from_model_id(&model_id);
    }
    if details.reasoning.is_none() && is_reasoning_supported(&model_id) {
        details.reasoning = Some(true);
    }
    if details.function_call.is_none() {
        if is_function_call_supported(&model_id) {
            details.function_call = Some(true);
        } else if is_known_non_chat_model(&model_id) {
            details.function_call = Some(false);
        }
    }
    if details.image_input.is_none() && is_image_input_supported(&model_id) {
        details.image_input = Some(true);
    }

    Ok(details)
}

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
        get_proxy_type_for_key(extra_args.clone(), None), // Use the first configured proxy server
        if custom_headers.as_object().map_or(true, |m| m.is_empty()) {
            None
        } else {
            Some(custom_headers)
        },
    );

    let client = DefaultApiClient::new(ErrorFormat::OpenAI);
    let response = client
        .get_request_with_origin(&config, "/models", None)
        .await
        .map_err(|e| AiError::ApiRequestFailed {
            provider: "OpenAI".to_string(),
            status_code: 0, // Network/Request failure, not an HTTP error
            details: e,
        })?;

    if response.is_error || response.content.is_empty() {
        let origin = response_error_origin(&response, "OpenAI");
        return Err(AiError::ApiRequestFailed {
            // Keep the historical fallback so retry classification is unchanged.
            status_code: 500,
            provider: "OpenAI".to_string(),
            details: FailureMessage::leaf(response.content).with_origin(origin),
        });
    }

    #[cfg(debug_assertions)]
    log::debug!("OpenAI list_models response: {}", &response.content);

    let models_response: OpenAIListModelsResponse = serde_json::from_str(&response.content)
        .map_err(|e| {
            let err = AiError::ResponseParseFailed {
                provider: "OpenAI".to_string(),
                details: FailureMessage::from_error(&e),
            };
            log_error_diagnostic(&err.diagnostic());
            err
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
            enrich_model_details(ModelDetails {
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
                family: None,
                function_call: None,
                reasoning: None,
                image_input: None,
                recommended_temperature: None,
                metadata: Some(json!({
                    "object": model.object.unwrap_or_default(),
                    "owned_by": model.owned_by.unwrap_or_default(),
                })),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

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
        .get_request_with_origin(
            &ApiConfig::new(
                Some(api_url.unwrap_or(CALUDE_BASE_URL).to_string()),
                None,
                get_proxy_type_for_key(extra_params, None), // Use the first configured proxy server
                Some(headers),
            ),
            "models", // Endpoint path
            Some(query_params),
        )
        .await
        .map_err(|network_err| {
            let err = AiError::ApiRequestFailed {
                status_code: 0, // Network/Request failure, not an HTTP error
                provider: "Claude".to_string(),
                details: network_err,
            };
            log_error_diagnostic(&err.diagnostic());
            err
        })?;

    if response.is_error || response.content.is_empty() {
        let origin = response_error_origin(&response, "Claude");
        let err = AiError::ApiRequestFailed {
            // Keep the historical fallback so retry classification is unchanged.
            status_code: 500,
            provider: "Claude".to_string(),
            details: FailureMessage::leaf(response.content).with_origin(origin),
        };
        log_error_diagnostic(&err.diagnostic());
        return Err(err);
    }

    #[cfg(debug_assertions)]
    log::debug!("Claude list_models response: {}", &response.content);

    let api_response: ClaudeListModelsResponse = from_str(&response.content).map_err(|e| {
        let err = AiError::ResponseParseFailed {
            provider: "Claude".to_string(),
            details: FailureMessage::from_error(&e),
        };
        log_error_diagnostic(&err.diagnostic());
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
            enrich_model_details(ModelDetails {
                id: api_model.id.clone(),
                name: api_model.display_name.unwrap_or(model_id),
                protocol: ChatProtocol::Claude,
                max_input_tokens: api_model.input_token_limit,
                max_output_tokens: api_model.output_token_limit,
                description: api_model.description,
                last_updated: api_model.created_at, // Use created_at from API
                family: None,
                // The Claude API's supports_tools is authoritative; heuristic is only fallback.
                function_call: api_model.supports_tools,
                reasoning: None,
                image_input: None,
                recommended_temperature: None,
                metadata: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

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
        get_proxy_type_for_key(extra_args.clone(), None), // Use the first configured proxy server
        None, // No special headers needed if key is in URL for this request
    );

    let client = DefaultApiClient::new(ErrorFormat::Google);
    let response = client
        .get_request_with_origin(&config, &endpoint_with_key, None)
        .await
        .map_err(|e| AiError::ApiRequestFailed {
            provider: "Gemini".to_string(),
            details: e,
            status_code: 0, // Network/Request failure, not an HTTP error
        })?;

    if response.is_error || response.content.is_empty() {
        let origin = response_error_origin(&response, "Gemini");
        return Err(AiError::ApiRequestFailed {
            // Keep the historical fallback so retry classification is unchanged.
            status_code: 500,
            provider: "Gemini".to_string(),
            details: FailureMessage::leaf(response.content).with_origin(origin),
        });
    }

    #[cfg(debug_assertions)]
    log::debug!("Gemini list_models response: {}", &response.content);

    let models_response: GeminiListModelsResponse = serde_json::from_str(&response.content)
        .map_err(|e| AiError::ResponseParseFailed {
            provider: "Gemini".to_string(),
            details: FailureMessage::from_error(&e),
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
            enrich_model_details(ModelDetails {
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
                family: None,
                function_call: None,
                reasoning: None,
                image_input: None,
                recommended_temperature: None,
                metadata: Some(json!({
                    "version": model.version,
                    "supported_generation_methods": model.supported_generation_methods,
                })),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(model_details)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(id: &str) -> ModelDetails {
        ModelDetails {
            id: id.to_string(),
            name: id.to_string(),
            protocol: ChatProtocol::OpenAI,
            max_input_tokens: None,
            max_output_tokens: None,
            description: None,
            last_updated: None,
            family: None,
            reasoning: None,
            function_call: None,
            image_input: None,
            recommended_temperature: None,
            metadata: None,
        }
    }

    #[test]
    fn enrichment_uses_catalog_before_heuristics() {
        let result = enrich_model_details(details("QWQ-32B")).expect("catalog");
        assert_eq!(result.family.as_deref(), Some("qwen"));
        assert_eq!(result.reasoning, Some(true));
        assert_eq!(result.image_input, None);
    }

    #[test]
    fn enrichment_includes_catalog_recommended_temperature() {
        let model = details("hy4-preview");
        let result = enrich_model_details(model).expect("catalog");
        assert_eq!(result.recommended_temperature, Some(0.9));
    }
    #[test]
    fn enrichment_preserves_explicit_provider_facts() {
        let mut model = details("deepseek-r1");
        model.reasoning = Some(false);
        model.function_call = Some(false);
        let result = enrich_model_details(model).expect("catalog");
        assert_eq!(result.reasoning, Some(false));
        assert_eq!(result.function_call, Some(false));
    }

    #[tokio::test]
    async fn openai_list_models_405_keeps_actual_origin_and_legacy_status() {
        use axum::{routing::get, Router};

        let app = Router::new().route(
            "/models",
            get(|| async {
                (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("content-type", "application/json")],
                    axum::Json(json!({
                        "error": {
                            "type": "method_not_allowed",
                            "message": "list models is not supported"
                        }
                    })),
                )
            }),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                .await
                .unwrap();
        });
        let base_url = format!("http://{address}");

        let error = openai_list_models(Some(&base_url), None, None)
            .await
            .expect_err("a 405 list-models answer is an error");

        match &error {
            AiError::ApiRequestFailed {
                status_code,
                details,
                ..
            } => {
                // The legacy status keeps the historical retry classification.
                assert_eq!(*status_code, 500);
                let origin = details.origin().expect("network error origin");
                assert_eq!(origin.status, Some(405));
                assert_eq!(origin.reason, Some(ErrorReason::UnsupportedListModels));
                assert_eq!(origin.message, "list models is not supported");

                let projection = details.user_error();
                assert_eq!(projection.upstream_status, Some(405));
                assert_eq!(projection.reason, Some(ErrorReason::UnsupportedListModels));
                assert_eq!(projection.message, "list models is not supported");
                assert_eq!(error.user_error(), projection);
            }
            other => panic!("unexpected error: {other:?}"),
        }

        server.abort();
    }
}
