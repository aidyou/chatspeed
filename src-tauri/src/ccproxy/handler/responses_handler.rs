use axum::body::Body;
use axum::response::{IntoResponse, Response};
use reqwest::header::HeaderMap;
use rust_i18n::t;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

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
    utils::token_estimator::{estimate_known_request_json_tokens, token_usage_is_missing_or_zero},
    ChatProtocol,
};
use crate::constants::{
    CFG_CCPROXY_LOG_PROXY_TO_FILE, CFG_CCPROXY_LOG_TO_FILE, CFG_CCPROXY_RETRY_ON_429,
    CFG_CCPROXY_RETRY_ON_429_DEFAULT,
};
use crate::db::{CcproxyStat, MainStore};

// Native Responses forwarding must tolerate input item variants added by upstream APIs.
#[derive(Deserialize)]
struct OpenAIResponsesRoutingRequest {
    model: String,
}

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

fn prepare_direct_responses_body(
    client_request_body: &[u8],
    backend_model: &str,
) -> ProxyResult<Value> {
    let mut body_json: Value = serde_json::from_slice(client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!(
            "Failed to deserialize Responses request body: {}",
            e
        ))
    })?;

    if let Some(obj) = body_json.as_object_mut() {
        obj.insert(
            "model".to_string(),
            Value::String(backend_model.trim().to_string()),
        );
    }

    Ok(body_json)
}

fn response_usage_tokens(body_json: &Value) -> (i64, i64, i64, i64, i64, i64, i64) {
    response_usage_tokens_from_usage(body_json.get("usage").unwrap_or(&Value::Null))
}

fn response_usage_tokens_from_usage(usage: &Value) -> (i64, i64, i64, i64, i64, i64, i64) {
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
    let cache_creation = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(|details| details.get("cache_creation_input_tokens"))
        .and_then(Value::as_i64)
        .or_else(|| {
            usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let audio_input = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("audio_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let audio_output = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("audio_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    (
        input,
        output,
        cache,
        cache_creation,
        reasoning,
        audio_input,
        audio_output,
    )
}

fn response_usage_tokens_from_body(body_bytes: &[u8]) -> (i64, i64, i64, i64, i64, i64, i64) {
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

    (0, 0, 0, 0, 0, 0, 0)
}

fn response_output_item_has_content(item: &Value) -> bool {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") | Some("custom_tool_call") => true,
        Some("reasoning") => item
            .get("summary")
            .and_then(Value::as_array)
            .is_some_and(|summary| {
                summary.iter().any(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                })
            }),
        Some("message") => item
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|content| {
                content.iter().any(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                })
            }),
        _ => false,
    }
}

fn response_output_has_content(output: &[Value]) -> bool {
    output.iter().any(response_output_item_has_content)
}

fn responses_has_output(body_bytes: &[u8]) -> bool {
    if let Ok(body) = serde_json::from_slice::<Value>(body_bytes) {
        return body
            .get("output")
            .and_then(Value::as_array)
            .is_some_and(|output| response_output_has_content(output));
    }

    String::from_utf8_lossy(body_bytes).lines().any(|line| {
        let Some(data) = line.strip_prefix("data:") else {
            return false;
        };
        serde_json::from_str::<Value>(data.trim())
            .ok()
            .and_then(|event| event.get("response").cloned())
            .and_then(|response| response.get("output").and_then(Value::as_array).cloned())
            .is_some_and(|output| response_output_has_content(&output))
    })
}

async fn direct_forward_responses(
    client_headers: HeaderMap,
    client_request_body: bytes::Bytes,
    proxy_model: ProxyModel,
    main_store_arc: Arc<MainStore>,
    log_proxy_to_file: bool,
) -> ProxyResult<Response> {
    let message_id = get_msg_id();
    let provider_name = proxy_model.provider.clone();
    let model_name = proxy_model.model.clone();
    let full_url = get_provider_responses_full_url(&proxy_model.base_url);

    let http_client = ModelResolver::build_http_client(
        main_store_arc.clone(),
        proxy_model.model_metadata.clone(),
        proxy_model.key_index,
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

    let body_json = prepare_direct_responses_body(&client_request_body, &proxy_model.model)?;

    // Mirror the client's streaming intent: Codex always sends
    // `Accept: text/event-stream` for streaming Responses requests.
    let accept_value = if body_json.get("stream").and_then(Value::as_bool) == Some(true) {
        "text/event-stream"
    } else {
        "application/json"
    };
    reqwest_headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static(accept_value),
    );

    let modified_body = serde_json::to_vec(&body_json).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to serialize Responses request body: {}", e))
    })?;

    if log_proxy_to_file {
        log::info!(
            target: "ccproxy_upstream_logger",
            "[Direct] OpenAI Responses Final Request Body: \n{}\n----------------\n",
            serde_json::to_string_pretty(&body_json).unwrap_or_default()
        );
    }

    let onward_request_builder = http_client
        .post(&full_url)
        .headers(reqwest_headers)
        .body(modified_body);

    let max_retries =
        main_store_arc.get_config(CFG_CCPROXY_RETRY_ON_429, CFG_CCPROXY_RETRY_ON_429_DEFAULT);
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
            target: "ccproxy_upstream_logger",
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

    let (
        input_tokens,
        output_tokens,
        cache_tokens,
        cache_creation_tokens,
        reasoning_tokens,
        audio_input_tokens,
        audio_output_tokens,
    ) = if status_code.is_success() {
        response_usage_tokens_from_body(&body_bytes)
    } else {
        (0, 0, 0, 0, 0, 0, 0)
    };
    let should_estimate = status_code.is_success()
        && token_usage_is_missing_or_zero(&[
            Some(input_tokens as u64),
            Some(output_tokens as u64),
            Some(cache_tokens as u64),
            Some(cache_creation_tokens as u64),
        ]);
    let estimated_input_tokens = if should_estimate {
        estimate_known_request_json_tokens(&body_json).ceil() as i64
    } else {
        input_tokens
    };
    let error_message = if status_code.is_success() {
        None
    } else {
        Some(String::from_utf8_lossy(&body_bytes).to_string())
    };

    if status_code.is_success() && responses_has_output(&body_bytes) {
        let store = main_store_arc.as_ref();
        let (estimated_cost, pricing_status, pricing_snapshot) =
            crate::ccproxy::helper::stat_guard::finalize_pricing(
                estimated_input_tokens,
                output_tokens,
                cache_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                audio_input_tokens,
                audio_output_tokens,
                proxy_model.pricing.as_ref(),
            );
        let _ = store.record_ccproxy_stat(
            CcproxyStat {
                id: None,
                workflow_session_id: None,
                workflow_task_run_id: None,
                workflow_segment_id: None,
                root_session_id: None,
                root_task_run_id: None,
                request_kind: None,
                client_model: proxy_model.client_alias.clone(),
                backend_model: model_name,
                provider_id: Some(proxy_model.provider_id),
                provider: provider_name,
                protocol: ChatProtocol::OpenAI.to_string(),
                tool_compat_mode: 0,
                status_code: status_code.as_u16() as i32,
                error_message,
                input_tokens: estimated_input_tokens,
                output_tokens,
                cache_tokens,
                cache_write_tokens: cache_creation_tokens,
                reasoning_tokens,
                audio_input_tokens,
                audio_output_tokens,
                estimated_cost,
                pricing_status,
                pricing_snapshot,
                request_at: None,
            }
            .with_workflow_attribution(&client_headers),
        );
    }

    Ok(response)
}

pub async fn handle_responses(
    client_headers: HeaderMap,
    _client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    tool_compat_mode: bool,
    main_store_arc: Arc<MainStore>,
) -> ProxyResult<Response> {
    let message_id = get_msg_id();
    let log_org_to_file = main_store_arc.get_config(CFG_CCPROXY_LOG_TO_FILE, false);
    let log_proxy_to_file = main_store_arc.get_config(CFG_CCPROXY_LOG_PROXY_TO_FILE, false);

    let routing_request: OpenAIResponsesRoutingRequest =
        serde_json::from_slice(&client_request_body).map_err(|e| {
            CCProxyError::InternalError(
                t!("proxy.error.invalid_request_format", error = e.to_string()).to_string(),
            )
        })?;

    let proxy_alias_raw = routing_request.model;
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

    let client_request_payload: OpenAIResponsesRequest =
        serde_json::from_slice(&client_request_body).map_err(|e| {
            CCProxyError::InternalError(
                t!("proxy.error.invalid_request_format", error = e.to_string()).to_string(),
            )
        })?;

    // TODO: The Responses-to-chat-completion fallback still needs detailed testing and
    // debugging, especially for streaming reasoning output and client-side completion
    // behavior. Prefer the direct Responses path when a provider supports it.
    if log_org_to_file {
        log::info!(target: "ccproxy_client_logger", "message id:{}\nOpenAI Responses Client Request Body: \n{}\n----------------\n", &message_id, String::from_utf8_lossy(&client_request_body));
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
    use super::{
        direct_forward_responses, prepare_direct_responses_body, response_usage_tokens,
        response_usage_tokens_from_body, supports_responses_api, OpenAIResponsesRoutingRequest,
    };
    use crate::ccproxy::types::{ChatProtocol, ProxyModel};
    use crate::db::MainStore;
    use axum::body::Body;
    use axum::response::Response;
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
            key_index: None,
            model_metadata: metadata,
            custom_params: None,
            pricing: None,
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
            thinking_adapter: None,
            matched_transport_id: None,
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
    fn responses_support_ignores_chat_completions_preference() {
        assert!(supports_responses_api(&proxy_model_with_metadata(Some(
            json!({
                "supportsResponsesApi": true,
                "responsesApiPreference": "chatCompletions"
            })
        ))));
    }

    #[test]
    fn responses_routing_accepts_structured_custom_tool_output() {
        let request = json!({
            "model": "gpt-5.6",
            "input": [
                {
                    "type": "custom_tool_call",
                    "call_id": "call_123",
                    "name": "functions.exec",
                    "input": "const result = await tools.exec_command({\"cmd\":\"pwd\"});"
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_123",
                    "output": [
                        {
                            "type": "input_text",
                            "text": "Process exited with code 0"
                        }
                    ]
                }
            ],
            "store": false,
            "stream": true
        });

        let routing_request: OpenAIResponsesRoutingRequest =
            serde_json::from_value(request).expect("routing request should deserialize");

        assert_eq!(routing_request.model, "gpt-5.6");
    }

    #[test]
    fn direct_responses_body_only_replaces_model() {
        let request = json!({
            "model": "gpt-5.6-alias",
            "conversation": "conv_123",
            "input": [
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_123",
                    "output": [
                        {
                            "type": "input_text",
                            "text": "Process exited with code 0"
                        }
                    ]
                }
            ],
            "prompt_cache_retention": "24h",
            "reasoning": {
                "context": "all_turns",
                "effort": "high"
            },
            "store": false,
            "stream": true
        });
        let body = serde_json::to_vec(&request).expect("request should serialize");

        let modified =
            prepare_direct_responses_body(&body, "gpt-5.6").expect("body should prepare");
        let mut expected = request;
        expected["model"] = json!("gpt-5.6");

        assert_eq!(modified, expected);
    }

    #[test]
    fn responses_usage_tokens_reads_direct_response_usage() {
        let (input, output, cache, cache_creation, _, _, _) = response_usage_tokens(&json!({
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
        assert_eq!(cache_creation, 0);
    }

    #[test]
    fn responses_usage_tokens_accepts_openai_compatible_usage_names() {
        let (input, output, cache, cache_creation, _, _, _) = response_usage_tokens(&json!({
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
        assert_eq!(cache_creation, 0);
    }

    #[tokio::test]
    async fn native_responses_non_stream_persists_attribution_and_filters_headers() {
        use axum::{extract::State, routing::post, Router};
        use std::sync::Arc;
        use tempfile::tempdir;
        use tokio::sync::mpsc;

        let response_body = serde_json::to_vec(&json!({
            "id": "resp_test",
            "object": "response",
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
            "usage": {
                "input_tokens": 120,
                "output_tokens": 35,
                "input_tokens_details": {"cached_tokens": 80}
            }
        }))
        .expect("response should serialize");
        let content_length = response_body.len().to_string();
        let (headers_tx, mut headers_rx) = mpsc::channel(1);
        let app = Router::new()
            .route(
                "/responses",
                post(
                    move |State(sender): State<mpsc::Sender<reqwest::header::HeaderMap>>,
                          headers: reqwest::header::HeaderMap| {
                        let response_body = response_body.clone();
                        let content_length = content_length.clone();
                        async move {
                            sender
                                .send(headers)
                                .await
                                .expect("receiver should remain open");
                            Response::builder()
                                .header("content-type", "application/json")
                                .header("content-length", content_length)
                                .header("x-upstream", "kept")
                                .body(Body::from(response_body))
                                .expect("mock response should build")
                        }
                    },
                ),
            )
            .with_state(headers_tx);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        listener
            .set_nonblocking(true)
            .expect("listener should be nonblocking");
        let address = listener.local_addr().expect("listener should have address");
        let server = tokio::spawn(async move {
            axum::serve(
                tokio::net::TcpListener::from_std(listener).expect("tokio listener should convert"),
                app,
            )
            .await
            .expect("mock upstream should serve");
        });

        let directory = tempdir().expect("test directory should exist");
        let store = Arc::new(
            MainStore::new(directory.path().join("responses.db"))
                .expect("test store should initialize"),
        );
        let proxy_model = proxy_model_with_metadata(Some(json!({
            "supports_responses_api": true
        })));
        let proxy_model = ProxyModel {
            base_url: format!("http://{address}"),
            ..proxy_model
        };
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in [
            ("x-cs-workflow-session-id", "workflow-session"),
            ("x-cs-workflow-task-run-id", "workflow-session:task:1"),
            ("x-cs-workflow-segment-id", "3"),
            ("x-cs-root-session-id", "root-session"),
            ("x-cs-root-task-run-id", "root-session:task:1"),
            ("x-cs-request-kind", "react"),
        ] {
            headers.insert(name, value.parse().expect("test header should parse"));
        }
        let response = direct_forward_responses(
            headers,
            bytes::Bytes::from_static(br#"{"model":"alias","input":[]}"#),
            proxy_model,
            store.clone(),
            false,
        )
        .await
        .expect("native Responses request should succeed");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers().get("x-upstream").unwrap(), "kept");
        assert!(!response.headers().contains_key("content-length"));
        let upstream_headers = headers_rx
            .recv()
            .await
            .expect("upstream should receive request");
        for name in [
            "x-cs-workflow-session-id",
            "x-cs-workflow-task-run-id",
            "x-cs-workflow-segment-id",
            "x-cs-root-session-id",
            "x-cs-root-task-run-id",
            "x-cs-request-kind",
        ] {
            assert!(!upstream_headers.contains_key(name));
        }

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("telemetry should flush");
        let stat = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT workflow_session_id, workflow_task_run_id, workflow_segment_id,
                            root_session_id, root_task_run_id, request_kind,
                            input_tokens, output_tokens, cache_tokens
                     FROM ccproxy_stats",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i32>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )?)
            })
            .await
            .expect("attributed Responses stat should persist");
        assert_eq!(
            stat,
            (
                "workflow-session".to_string(),
                "workflow-session:task:1".to_string(),
                3,
                "root-session".to_string(),
                "root-session:task:1".to_string(),
                "react".to_string(),
                120,
                35,
                80,
            )
        );
        server.abort();
    }

    #[test]
    fn responses_usage_tokens_reads_stream_completed_event() {
        let body = r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_123","status":"in_progress"}}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","status":"completed","usage":{"input_tokens":81345,"input_tokens_details":{"cached_tokens":76672},"output_tokens":397,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":81742}}}

"#;

        let (input, output, cache, cache_creation, _, _, _) =
            response_usage_tokens_from_body(body.as_bytes());

        assert_eq!(input, 81345);
        assert_eq!(output, 397);
        assert_eq!(cache, 76672);
        assert_eq!(cache_creation, 0);
    }

    #[tokio::test]
    async fn native_responses_stream_persists_attribution_and_filters_headers() {
        use axum::{extract::State, routing::post, Router};
        use std::sync::Arc;
        use tempfile::tempdir;
        use tokio::sync::mpsc;

        let response_body = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}],\"usage\":{\"input_tokens\":120,\"input_tokens_details\":{\"cached_tokens\":80},\"output_tokens\":35}}}\n\n"
        );
        let content_length = response_body.len().to_string();
        let (headers_tx, mut headers_rx) = mpsc::channel(1);
        let app = Router::new()
            .route(
                "/responses",
                post(
                    move |State(sender): State<mpsc::Sender<reqwest::header::HeaderMap>>,
                          headers: reqwest::header::HeaderMap| {
                        let content_length = content_length.clone();
                        async move {
                            sender
                                .send(headers)
                                .await
                                .expect("receiver should remain open");
                            Response::builder()
                                .header("content-type", "text/event-stream")
                                .header("content-length", content_length)
                                .header("x-upstream", "kept")
                                .body(Body::from(response_body))
                                .expect("mock stream response should build")
                        }
                    },
                ),
            )
            .with_state(headers_tx);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        listener
            .set_nonblocking(true)
            .expect("listener should be nonblocking");
        let address = listener.local_addr().expect("listener should have address");
        let server = tokio::spawn(async move {
            axum::serve(
                tokio::net::TcpListener::from_std(listener).expect("tokio listener should convert"),
                app,
            )
            .await
            .expect("mock upstream should serve");
        });
        let directory = tempdir().expect("test directory should exist");
        let store = Arc::new(
            MainStore::new(directory.path().join("responses-stream.db"))
                .expect("test store should initialize"),
        );
        let base_model = proxy_model_with_metadata(Some(json!({
            "supports_responses_api": true
        })));
        let proxy_model = ProxyModel {
            base_url: format!("http://{address}"),
            ..base_model
        };
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in [
            ("x-cs-workflow-session-id", "workflow-session"),
            ("x-cs-workflow-task-run-id", "workflow-session:task:1"),
            ("x-cs-workflow-segment-id", "3"),
            ("x-cs-root-session-id", "root-session"),
            ("x-cs-root-task-run-id", "root-session:task:1"),
            ("x-cs-request-kind", "react"),
        ] {
            headers.insert(name, value.parse().expect("test header should parse"));
        }
        let response = direct_forward_responses(
            headers,
            bytes::Bytes::from_static(br#"{"model":"alias","input":[],"stream":true}"#),
            proxy_model,
            store.clone(),
            false,
        )
        .await
        .expect("native Responses stream should succeed");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers().get("x-upstream").unwrap(), "kept");
        assert!(!response.headers().contains_key("content-length"));
        let upstream_headers = headers_rx
            .recv()
            .await
            .expect("upstream should receive request");
        for name in [
            "x-cs-workflow-session-id",
            "x-cs-workflow-task-run-id",
            "x-cs-workflow-segment-id",
            "x-cs-root-session-id",
            "x-cs-root-task-run-id",
            "x-cs-request-kind",
        ] {
            assert!(!upstream_headers.contains_key(name));
        }
        let _body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("stream body should be readable");

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("telemetry should flush");
        let stat = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT workflow_session_id, workflow_task_run_id, workflow_segment_id,
                            root_session_id, root_task_run_id, request_kind,
                            input_tokens, output_tokens, cache_tokens
                     FROM ccproxy_stats",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i32>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )?)
            })
            .await
            .expect("attributed Responses stream stat should persist");
        assert_eq!(
            stat,
            (
                "workflow-session".to_string(),
                "workflow-session:task:1".to_string(),
                3,
                "root-session".to_string(),
                "root-session:task:1".to_string(),
                "react".to_string(),
                120,
                35,
                80,
            )
        );
        server.abort();
    }
}
