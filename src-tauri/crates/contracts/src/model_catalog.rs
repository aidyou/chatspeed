//! Models.dev catalog wire contracts.
//!
//! The runtime owns the Models.dev catalog snapshot, its loading and its
//! refresh, and the desktop reaches those reads only through the allowlisted
//! data commands `get_models_dev_providers`, `get_models_dev_provider_models`
//! and `resolve_model_profile`. This module is the single source of truth for
//! the JSON names of those requests and for the provider display type that the
//! desktop re-exports instead of maintaining a second definition.
//!
//! Two naming rules apply, matching the rest of the control plane (decision
//! D-7):
//! - requests are canonical snake_case;
//! - the provider display payload keeps the exact camelCase shape the Tauri
//!   frontend already consumes, so the desktop adapter can forward a runtime
//!   value without redefining the UI payload.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Canonical request body for `get_models_dev_provider_models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ModelsDevProviderModelsRequest {
    /// Identifier of the provider whose embedded catalog models are requested.
    pub provider_id: String,
}

/// Canonical request body for `resolve_model_profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ResolveModelProfileRequest {
    /// Model identifier to resolve.
    pub model_id: String,
    /// Optional endpoint base URL used as a provider hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Optional backend wire protocol used for transport resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_protocol: Option<String>,
    /// Optional provider/model metadata (proxy settings, custom params, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// One generated Models.dev provider preset shown by the desktop.
///
/// Field names match the camelCase shape the Tauri frontend has always
/// consumed, so the desktop re-exports this type instead of defining a second
/// catalog display type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDevPresetProviderDto {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub documentation_url: Option<String>,
    #[serde(default)]
    pub model_list_url: Option<String>,
    #[serde(default)]
    pub key_apply_url: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub responses: bool,
    #[serde(default)]
    pub model_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_models_request_is_snake_case() {
        let request = ModelsDevProviderModelsRequest {
            provider_id: "openrouter".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&request).expect("serialize"),
            json!({"provider_id": "openrouter"})
        );
    }

    #[test]
    fn resolve_model_profile_request_shape_is_snake_case_and_omits_absent_optionals() {
        let bare = ResolveModelProfileRequest {
            model_id: "gpt-4o".to_string(),
            base_url: None,
            backend_protocol: None,
            metadata: None,
        };
        assert_eq!(
            serde_json::to_value(&bare).expect("serialize"),
            json!({"model_id": "gpt-4o"})
        );

        let full = ResolveModelProfileRequest {
            model_id: "gpt-4o".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            backend_protocol: Some("openai".to_string()),
            metadata: Some(json!({"modelsDevProviderId": "openai"})),
        };
        let value = serde_json::to_value(&full).expect("serialize");
        assert_eq!(value["model_id"], json!("gpt-4o"));
        assert_eq!(value["base_url"], json!("https://api.openai.com/v1"));
        assert_eq!(value["backend_protocol"], json!("openai"));
        assert_eq!(value["metadata"], json!({"modelsDevProviderId": "openai"}));
        let parsed: ResolveModelProfileRequest =
            serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed, full);
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        assert!(serde_json::from_value::<ModelsDevProviderModelsRequest>(
            json!({"provider_id": "x", "extra": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<ResolveModelProfileRequest>(
            json!({"model_id": "x", "other": 1})
        )
        .is_err());
    }

    #[test]
    fn provider_display_payload_keeps_the_canonical_camel_case_shape() {
        let provider = ModelsDevPresetProviderDto {
            id: "openrouter".to_string(),
            name: "OpenRouter".to_string(),
            protocol: Some("openai".to_string()),
            logo: None,
            description: None,
            documentation_url: Some("https://openrouter.ai/docs".to_string()),
            model_list_url: None,
            key_apply_url: None,
            api: Some("https://openrouter.ai/api/v1".to_string()),
            responses: false,
            model_count: 42,
        };
        assert_eq!(
            serde_json::to_value(&provider).expect("serialize"),
            json!({
                "id": "openrouter",
                "name": "OpenRouter",
                "protocol": "openai",
                "logo": null,
                "description": null,
                "documentationUrl": "https://openrouter.ai/docs",
                "modelListUrl": null,
                "keyApplyUrl": null,
                "api": "https://openrouter.ai/api/v1",
                "responses": false,
                "modelCount": 42,
            })
        );
    }
}
