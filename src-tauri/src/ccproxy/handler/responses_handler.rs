use axum::body::Body;
use axum::response::{IntoResponse, Response};
use reqwest::header::HeaderMap;
use rust_i18n::t;
use serde_json::Value;
use std::sync::{Arc, RwLock};

use crate::ccproxy::{
    adapter::{
        input::from_openai_responses,
        output::{OpenAIResponsesOutputAdapter, OutputAdapterEnum},
    },
    errors::{CCProxyError, ProxyResult},
    handler::chat_handler::{
        execute_unified_chat_request, prepare_unified_request_for_proxy_model,
    },
    helper::{get_msg_id, send_with_retry, CcproxyQuery, ModelResolver, RetryConfig},
    types::{openai_responses::OpenAIResponsesRequest, ProxyModel},
    ChatProtocol,
};
use crate::constants::{
    CFG_CCPROXY_LOG_PROXY_TO_FILE, CFG_CCPROXY_LOG_TO_FILE, CFG_CCPROXY_RETRY_ON_429,
    CFG_CCPROXY_RETRY_ON_429_DEFAULT,
};
use crate::db::{CcproxyStat, MainStore};

fn supports_responses_api(proxy_model: &ProxyModel) -> bool {
    proxy_model
        .model_metadata
        .as_ref()
        .and_then(|metadata| {
            metadata
                .get("supports_responses_api")
                .or_else(|| metadata.get("supportsResponsesApi"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn get_provider_responses_full_url(base_url: &str) -> String {
    format!("{}/responses", base_url.trim_end_matches('/'))
}

fn response_usage_tokens(body_json: &Value) -> (i64, i64, i64) {
    response_usage_tokens_from_usage(body_json.get("usage").unwrap_or(&Value::Null))
}

fn response_usage_tokens_from_usage(usage: &Value) -> (i64, i64, i64) {
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_i64)
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_i64))
        .or_else(|| usage.get("cached_content_tokens").and_then(Value::as_i64))
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_i64))
        .unwrap_or(0);
    (input, output, cache)
}

fn response_usage_tokens_from_body(body_bytes: &[u8]) -> (i64, i64, i64) {
    if let Ok(body_json) = serde_json::from_slice::<Value>(body_bytes) {
        return response_usage_tokens(&body_json);
    }

    let body_text = String::from_utf8_lossy(body_bytes);
    for line in body_text.lines().rev() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(event_json) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if event_json.get("type").and_then(Value::as_str) != Some("response.completed") {
            continue;
        }
        let usage = event_json
            .get("response")
            .and_then(|response| response.get("usage"))
            .unwrap_or(&Value::Null);
        return response_usage_tokens_from_usage(usage);
    }

    (0, 0, 0)
}

async fn direct_forward_responses(
    client_headers: HeaderMap,
    client_request_body: bytes::Bytes,
    proxy_model: ProxyModel,
    main_store_arc: Arc<RwLock<MainStore>>,
    log_proxy_to_file: bool,
) -> ProxyResult<Response> {
    let message_id = get_msg_id();
    let provider_name = proxy_model.provider.clone();
    let model_name = proxy_model.model.clone();
    let full_url = get_provider_responses_full_url(&proxy_model.base_url);

    let http_client = ModelResolver::build_http_client(
        main_store_arc.clone(),
        proxy_model.model_metadata.clone(),
    )?;

    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    ModelResolver::inject_proxy_headers(
        &mut reqwest_headers,
        &client_headers,
        &proxy_model,
        &message_id,
    );
    reqwest_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    reqwest_headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!(
            "Failed to deserialize Responses request body: {}",
            e
        ))
    })?;

    crate::ai::util::merge_custom_params_value(&mut body_json, &proxy_model.custom_params);

    if let Some(obj) = body_json.as_object_mut() {
        obj.insert(
            "model".to_string(),
            Value::String(proxy_model.model.trim().to_string()),
        );
    }

    let modified_body = serde_json::to_vec(&body_json).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to serialize Responses request body: {}", e))
    })?;

    if log_proxy_to_file {
        log::info!(
            target: "ccproxy_logger",
            "[Direct] OpenAI Responses Final Request Body: \n{}\n----------------\n",
            serde_json::to_string_pretty(&body_json).unwrap_or_default()
        );
    }

    let onward_request_builder = http_client
        .post(&full_url)
        .headers(reqwest_headers)
        .body(modified_body);

    let max_retries = if let Ok(store) = main_store_arc.read() {
        store.get_config(CFG_CCPROXY_RETRY_ON_429, CFG_CCPROXY_RETRY_ON_429_DEFAULT)
    } else {
        CFG_CCPROXY_RETRY_ON_429_DEFAULT
    };
    let retry_config = RetryConfig::from_settings(max_retries);
    let target_response = send_with_retry(onward_request_builder, &retry_config).await?;

    let status_code = target_response.status();
    let response_headers = target_response.headers().clone();
    let body_bytes = target_response.bytes().await.map_err(|e| {
        CCProxyError::InternalError(
            t!("network.response_read_error", error = e.to_string()).to_string(),
        )
    })?;

    if log_proxy_to_file {
        log::info!(
            target: "ccproxy_logger",
            "[Direct] OpenAI Responses Response Body: {}\n================\n\n",
            String::from_utf8_lossy(&body_bytes)
        );
    }

    let filtered_headers = crate::ccproxy::utils::http::filter_proxy_headers(&response_headers);
    let mut response = Response::builder()
        .status(status_code)
        .body(Body::from(body_bytes.clone()))
        .map_err(|e| CCProxyError::InternalError(format!("Failed to build response: {}", e)))?;
    *response.headers_mut() = filtered_headers;

    let (input_tokens, output_tokens, cache_tokens) = if status_code.is_success() {
        response_usage_tokens_from_body(&body_bytes)
    } else {
        (0, 0, 0)
    };
    let error_message = if status_code.is_success() {
        None
    } else {
        Some(String::from_utf8_lossy(&body_bytes).to_string())
    };

    if let Ok(store) = main_store_arc.read() {
        let _ = store.record_ccproxy_stat(CcproxyStat {
            id: None,
            client_model: proxy_model.client_alias.clone(),
            backend_model: model_name,
            provider_id: Some(proxy_model.provider_id),
            provider: provider_name,
            protocol: ChatProtocol::OpenAI.to_string(),
            tool_compat_mode: 0,
            status_code: status_code.as_u16() as i32,
            error_message,
            input_tokens,
            output_tokens,
            cache_tokens,
            request_at: None,
        });
    }

    Ok(response)
}

pub async fn handle_responses(
    client_headers: HeaderMap,
    client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    tool_compat_mode: bool,
    main_store_arc: Arc<RwLock<MainStore>>,
) -> ProxyResult<Response> {
    let message_id = get_msg_id();
    let log_org_to_file = if let Ok(store) = main_store_arc.read() {
        store.get_config(CFG_CCPROXY_LOG_TO_FILE, false)
    } else {
        client_query.debug.unwrap_or(false)
    };
    let log_proxy_to_file = if let Ok(store) = main_store_arc.read() {
        store.get_config(CFG_CCPROXY_LOG_PROXY_TO_FILE, false)
    } else {
        false
    };

    let client_request_payload: OpenAIResponsesRequest =
        serde_json::from_slice(&client_request_body).map_err(|e| {
            CCProxyError::InternalError(
                t!("proxy.error.invalid_request_format", error = e.to_string()).to_string(),
            )
        })?;

    let proxy_alias_raw = client_request_payload.model.clone();
    let (proxy_alias, group_name) = if let Some((g, a)) = proxy_alias_raw.split_once('@') {
        (a.to_string(), Some(g.to_string()))
    } else {
        (proxy_alias_raw, group_name)
    };

    let proxy_model = if let Some(provider_id) = client_headers
        .get("x-cs-provider-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        let model_id = client_headers
            .get("x-cs-model-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| proxy_alias.clone());

        ModelResolver::get_ai_model_by_provider_and_model(
            main_store_arc.clone(),
            provider_id,
            model_id,
        )
        .await?
    } else {
        ModelResolver::get_ai_model_by_alias(
            main_store_arc.clone(),
            proxy_alias.clone(),
            group_name.as_deref(),
        )
        .await?
    };

    let final_tool_compat_mode = match proxy_model.tool_compat_mode.as_deref() {
        Some("compat") => true,
        Some("native") => false,
        Some("auto") | None => tool_compat_mode,
        _ => tool_compat_mode,
    };

    if supports_responses_api(&proxy_model) {
        let response = direct_forward_responses(
            client_headers,
            client_request_body,
            proxy_model,
            main_store_arc,
            log_proxy_to_file,
        )
        .await?;
        return Ok(response.into_response());
    }

    // TODO: The Responses-to-chat-completion fallback still needs detailed testing and
    // debugging, especially for streaming reasoning output and client-side completion
    // behavior. Prefer the direct Responses path when a provider supports it.
    if log_org_to_file {
        log::info!(target: "ccproxy_logger", "message id:{}\nOpenAI Responses Origin Request Body: \n{}\n----------------\n", &message_id, String::from_utf8_lossy(&client_request_body));
    }

    let is_streaming_request = client_request_payload.stream.unwrap_or(false);
    let mut unified_request = from_openai_responses(client_request_payload, final_tool_compat_mode)
        .map_err(|e| {
            CCProxyError::InternalError(
                t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
            )
        })?;
    prepare_unified_request_for_proxy_model(&mut unified_request, &proxy_model);

    execute_unified_chat_request(
        ChatProtocol::OpenAI,
        client_headers,
        unified_request,
        proxy_alias,
        proxy_model,
        is_streaming_request,
        tool_compat_mode,
        final_tool_compat_mode,
        message_id,
        log_org_to_file,
        log_proxy_to_file,
        main_store_arc,
        OutputAdapterEnum::OpenAIResponses(OpenAIResponsesOutputAdapter),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{response_usage_tokens, response_usage_tokens_from_body, supports_responses_api};
    use crate::ccproxy::types::{ChatProtocol, ProxyModel};
    use serde_json::json;

    fn proxy_model_with_metadata(metadata: Option<serde_json::Value>) -> ProxyModel {
        ProxyModel {
            client_alias: "alias".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            chat_protocol: ChatProtocol::OpenAI,
            base_url: "https://api.example.com/v1".to_string(),
            model: "model".to_string(),
            api_key: String::new(),
            model_metadata: metadata,
            custom_params: None,
            prompt_injection: "off".to_string(),
            prompt_injection_position: None,
            prompt_text: String::new(),
            tool_filter: Default::default(),
            prompt_replace: Vec::new(),
            temp_ratio: 1.0,
            max_tokens: None,
            temperature: None,
            presence_penalty: None,
            frequency_penalty: None,
            top_p: None,
            top_k: None,
            stop: Vec::new(),
            tool_compat_mode: None,
        }
    }

    #[test]
    fn responses_support_defaults_to_false() {
        assert!(!supports_responses_api(&proxy_model_with_metadata(None)));
    }

    #[test]
    fn responses_support_reads_snake_case_metadata() {
        assert!(supports_responses_api(&proxy_model_with_metadata(Some(
            json!({
                "supports_responses_api": true
            })
        ))));
    }

    #[test]
    fn responses_support_reads_camel_case_metadata() {
        assert!(supports_responses_api(&proxy_model_with_metadata(Some(
            json!({
                "supportsResponsesApi": true
            })
        ))));
    }

    #[test]
    fn responses_usage_tokens_reads_direct_response_usage() {
        let (input, output, cache) = response_usage_tokens(&json!({
            "usage": {
                "input_tokens": 120,
                "output_tokens": 35,
                "total_tokens": 155,
                "input_tokens_details": {
                    "cached_tokens": 80
                }
            }
        }));

        assert_eq!(input, 120);
        assert_eq!(output, 35);
        assert_eq!(cache, 80);
    }

    #[test]
    fn responses_usage_tokens_accepts_openai_compatible_usage_names() {
        let (input, output, cache) = response_usage_tokens(&json!({
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 35,
                "prompt_tokens_details": {
                    "cached_tokens": 80
                }
            }
        }));

        assert_eq!(input, 120);
        assert_eq!(output, 35);
        assert_eq!(cache, 80);
    }

    #[test]
    fn responses_usage_tokens_reads_stream_completed_event() {
        let body = r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_123","status":"in_progress"}}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","status":"completed","usage":{"input_tokens":81345,"input_tokens_details":{"cached_tokens":76672},"output_tokens":397,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":81742}}}

"#;

        let (input, output, cache) = response_usage_tokens_from_body(body.as_bytes());

        assert_eq!(input, 81345);
        assert_eq!(output, 397);
        assert_eq!(cache, 76672);
    }
}
