//! Direct forwarding for the System One decision protocol served on `/v1/systemone`.
use crate::ccproxy::{
    decision::{self, DecisionError},
    errors::{CCProxyError, ProxyResult},
    helper::{stat_guard::finalize_pricing, ModelResolver},
    types::ProxyModel,
    utils::http::filter_proxy_headers,
    ChatProtocol,
};
use crate::db::{CcproxyStat, MainStore};
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use rust_i18n::t;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

/// Forwards a client decision request to the configured evaluation endpoint.
///
/// The decision protocol has no compatibility mode: the client payload is only corrected for
/// routing (proxy alias to backend model id) and then passed through with its upstream status.
pub async fn handle_decision(
    client_headers: HeaderMap,
    client_request_body: Bytes,
    group_name: Option<String>,
    main_store_arc: Arc<MainStore>,
) -> ProxyResult<Response> {
    let mut body = parse_body(&client_request_body)?;
    let requested_model = requested_model(&body)?;

    let proxy_model = resolve_proxy_model(
        &client_headers,
        main_store_arc.clone(),
        &requested_model,
        group_name.as_deref(),
    )
    .await?;

    if proxy_model.chat_protocol != ChatProtocol::Decision {
        return Err(CCProxyError::InvalidProtocolError(
            proxy_model.chat_protocol.to_string(),
        ));
    }

    // The client sends the proxy alias, but the evaluation endpoint selects the model itself.
    if let Some(object) = body.as_object_mut() {
        object.insert(
            "model".to_string(),
            Value::String(proxy_model.model.trim().to_string()),
        );
    }

    let started = Instant::now();
    let response = match decision::forward(
        main_store_arc.clone(),
        &proxy_model.base_url,
        &proxy_model.api_key,
        proxy_model.key_index,
        proxy_model.model_metadata.clone(),
        &body,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let failure = map_forward_error(error);
            log::warn!(
                "decision proxy: alias={}, provider={}, model={}, status=failed, elapsed_ms={}, error={}",
                proxy_model.client_alias,
                proxy_model.provider,
                proxy_model.model,
                started.elapsed().as_millis(),
                failure
            );
            record_stat(
                main_store_arc.as_ref(),
                &proxy_model,
                &client_headers,
                StatusCode::BAD_GATEWAY.as_u16() as i32,
                Some(failure.to_string()),
                0,
                0,
            );
            return Err(failure);
        }
    };

    let status = response.status;
    let (input_tokens, output_tokens) = decision_usage(&response.body);
    let error_message = if status.is_success() {
        None
    } else {
        Some(
            String::from_utf8_lossy(&response.body)
                .chars()
                .take(2048)
                .collect::<String>(),
        )
    };
    log::info!(
        "decision proxy: alias={}, provider={}, model={}, status={}, elapsed_ms={}, input_tokens={}, output_tokens={}",
        proxy_model.client_alias,
        proxy_model.provider,
        proxy_model.model,
        status,
        started.elapsed().as_millis(),
        input_tokens,
        output_tokens
    );
    record_stat(
        main_store_arc.as_ref(),
        &proxy_model,
        &client_headers,
        status.as_u16() as i32,
        error_message,
        input_tokens,
        output_tokens,
    );

    let mut builder = Response::builder().status(status);
    for (name, value) in filter_proxy_headers(&response.headers).iter() {
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    builder
        .body(Body::from(response.body))
        .map_err(|error| CCProxyError::InternalError(error.to_string()))
}

fn parse_body(client_request_body: &Bytes) -> ProxyResult<Value> {
    let value: Value = serde_json::from_slice(client_request_body).map_err(|error| {
        CCProxyError::InvalidRequestBody(
            t!("proxy.error.invalid_request_format", error = error.to_string()).to_string(),
        )
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CCProxyError::InvalidRequestBody(
            t!("proxy.error.invalid_request", error = "body must be a JSON object").to_string(),
        ))
    }
}

fn requested_model(body: &Value) -> ProxyResult<String> {
    body.get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CCProxyError::InvalidRequestBody(
                t!("proxy.error.invalid_request", error = "missing model").to_string(),
            )
        })
}

async fn resolve_proxy_model(
    client_headers: &HeaderMap,
    main_store: Arc<MainStore>,
    requested_model: &str,
    group_name: Option<&str>,
) -> ProxyResult<ProxyModel> {
    if let Some(provider_id) = client_headers
        .get("x-cs-provider-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
    {
        let model_id = client_headers
            .get("x-cs-model-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(requested_model)
            .to_string();
        return ModelResolver::get_ai_model_by_provider_and_model(main_store, provider_id, model_id)
            .await;
    }

    let (alias, inline_group) = match requested_model.split_once('@') {
        Some((group, alias)) => (alias.to_string(), Some(group.to_string())),
        None => (requested_model.to_string(), None),
    };
    ModelResolver::get_ai_model_by_alias(
        main_store,
        alias,
        inline_group.as_deref().or(group_name),
    )
    .await
}

/// Failed decision transport cannot answer HTTP, so it surfaces as a proxy backend failure.
fn map_forward_error(error: DecisionError) -> CCProxyError {
    match error {
        DecisionError::InvalidUrl(detail) => {
            CCProxyError::InvalidProtocolError(detail.to_string())
        }
        DecisionError::Catalog(detail) => CCProxyError::InvalidProtocolError(detail),
        other => CCProxyError::BackendRequestError(other.to_string()),
    }
}

/// Reads the decision usage block, which uses `input_tokens`/`output_tokens` instead of chat names.
fn decision_usage(body: &Bytes) -> (i64, i64) {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let usage = value.get("usage");
    (
        usage
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        usage
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
}

fn record_stat(
    store: &MainStore,
    proxy_model: &ProxyModel,
    client_headers: &HeaderMap,
    status_code: i32,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
) {
    let (estimated_cost, pricing_status, pricing_snapshot) = finalize_pricing(
        input_tokens,
        output_tokens,
        0,
        0,
        0,
        0,
        0,
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
            backend_model: proxy_model.model.clone(),
            provider_id: Some(proxy_model.provider_id),
            provider: proxy_model.provider.clone(),
            protocol: ChatProtocol::Decision.to_string(),
            tool_compat_mode: 0,
            status_code,
            error_message,
            input_tokens,
            output_tokens,
            cache_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            audio_input_tokens: 0,
            audio_output_tokens: 0,
            estimated_cost,
            pricing_status,
            pricing_snapshot,
            request_at: None,
        }
        .with_workflow_attribution(client_headers),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::CFG_CHAT_COMPLETION_PROXY;
    use crate::db::ModelConfig;
    use axum::{body::to_bytes, response::IntoResponse};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn decision_provider(
        store: &MainStore,
        base_url: String,
        api_protocol: &str,
        model_id: &str,
    ) -> i64 {
        store
            .add_ai_model(
                "provider".to_string(),
                vec![ModelConfig {
                    id: model_id.to_string(),
                    ..Default::default()
                }],
                model_id.to_string(),
                api_protocol.to_string(),
                base_url,
                "secret".to_string(),
                0,
                0.0,
                0.0,
                0,
                false,
                None,
            )
            .unwrap()
    }

    #[tokio::test]
    async fn decision_route_rewrites_alias_and_passes_the_upstream_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let length = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]).to_string();
            let body = br#"{"model":"jev-1.13.0","answers":{"urgent":{"type":"noul","noul":0.9}},"usage":{"input_tokens":7,"output_tokens":3}}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
            request
        });

        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("decision-route.db")).unwrap());
        let provider_id = decision_provider(
            &store,
            format!("http://127.0.0.1:{port}/v1/systemone"),
            "decision",
            "jev-1.13.0",
        );
        store
            .set_config(
                CFG_CHAT_COMPLETION_PROXY,
                &json!({"default": {"jev": [{"id": provider_id, "model": "jev-1.13.0"}]}}),
            )
            .unwrap();

        let body = Bytes::from(
            serde_json::to_vec(&json!({
                "state": {"message": "My card was charged twice."},
                "model": "jev",
                "questions": {"urgent": {"type": "noul", "instructions": "is this urgent?"}}
            }))
            .unwrap(),
        );
        let response = handle_decision(HeaderMap::new(), body, None, store.clone())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(payload["answers"]["urgent"]["noul"], 0.9);

        let request = upstream.await.unwrap();
        assert!(request.starts_with("POST /v1/systemone HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer secret"));
        assert!(request.contains("\"model\":\"jev-1.13.0\""));
        assert!(!request.contains("\"model\":\"jev\""));
    }

    #[tokio::test]
    async fn decision_route_rejects_bodies_without_a_usable_model() {
        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("decision-invalid.db")).unwrap());

        for body in [
            br#"{"state":"hi","questions":{}}"#.to_vec(),
            br#"{"state":"hi","model":"   ","questions":{}}"#.to_vec(),
            br#"["state","model"]"#.to_vec(),
            b"not json".to_vec(),
        ] {
            let status = handle_decision(
                HeaderMap::new(),
                Bytes::from(body),
                None,
                store.clone(),
            )
            .await
            .unwrap_err()
            .into_response()
            .status();
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn decision_route_rejects_aliases_resolving_to_chat_providers() {
        let directory = tempdir().unwrap();
        let store = Arc::new(
            MainStore::new(directory.path().join("decision-chat-provider.db")).unwrap(),
        );
        let provider_id = decision_provider(
            &store,
            "https://api.example.test/v1".to_string(),
            "openai",
            "chat-model",
        );
        store
            .set_config(
                CFG_CHAT_COMPLETION_PROXY,
                &json!({"default": {"chat-alias": [{"id": provider_id, "model": "chat-model"}]}}),
            )
            .unwrap();

        let body = Bytes::from(
            serde_json::to_vec(&json!({"state": "hi", "model": "chat-alias", "questions": {}}))
                .unwrap(),
        );
        let error = handle_decision(HeaderMap::new(), body, None, store.clone())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            CCProxyError::InvalidProtocolError(protocol) if protocol == "openai"
        ));
    }

    #[test]
    fn decision_usage_reads_the_protocol_token_names() {
        let body = Bytes::from_static(
            br#"{"model":"jev-1.13.0","answers":{},"usage":{"input_tokens":12,"output_tokens":5}}"#,
        );
        assert_eq!(decision_usage(&body), (12, 5));
        assert_eq!(decision_usage(&Bytes::from_static(b"not json")), (0, 0));
    }
}
