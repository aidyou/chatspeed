use axum::response::Response;
use reqwest::header::HeaderMap;
use std::sync::Arc;

use crate::ccproxy::diagnostic::{self, RequestContext};
use crate::ccproxy::ChatProtocol;
use crate::ccproxy::{
    adapter::{
        backend::{
            BackendAdapter, ClaudeBackendAdapter, GeminiBackendAdapter, OllamaBackendAdapter,
            OpenAIBackendAdapter,
        },
        input::{
            from_gemini_embedding, from_ollama_embed, from_ollama_embedding, from_openai_embedding,
        },
        output::{
            ClaudeOutputAdapter, GeminiOutputAdapter, OllamaOutputAdapter, OpenAIOutputAdapter,
            OutputAdapter, OutputAdapterEnum,
        },
        unified::UnifiedEmbeddingRequest,
    },
    errors::CCProxyError,
    gemini::GeminiEmbedRequest,
    helper::{
        get_provider_embedding_full_url, send_with_retry, CcproxyQuery, ModelResolver, RetryConfig,
    },
    openai::OpenAIEmbeddingRequest,
    types::ollama::{OllamaEmbedRequest, OllamaEmbeddingsRequest},
};
use crate::constants::{CFG_CCPROXY_RETRY_ON_429, CFG_CCPROXY_RETRY_ON_429_DEFAULT};
use crate::db::{CcproxyStat, MainStore};

fn get_proxy_alias_from_body(
    chat_protocol: &ChatProtocol,
    client_request_body: &bytes::Bytes,
    route_model_alias: &str,
) -> Result<String, CCProxyError> {
    match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            let payload: OpenAIEmbeddingRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    CCProxyError::InternalError(format!(
                        "Failed to deserialize OpenAI embedding request: {}",
                        e
                    ))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Ollama => {
            // Try to deserialize as modern first, then legacy
            if let Ok(payload) = serde_json::from_slice::<OllamaEmbedRequest>(client_request_body) {
                return Ok(payload.model);
            }
            let payload: OllamaEmbeddingsRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    CCProxyError::InternalError(format!(
                        "Failed to deserialize Ollama embedding request: {}",
                        e
                    ))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Claude => Err(CCProxyError::InvalidProtocolError(
            "Claude protocol does not support embeddings".to_string(),
        )),
        ChatProtocol::Gemini => Ok(route_model_alias.to_string()),
        ChatProtocol::Decision => Err(CCProxyError::InvalidProtocolError(
            "Decision protocol does not support embeddings".to_string(),
        )),
    }
}

fn build_unified_request(
    chat_protocol: ChatProtocol,
    client_request_body: bytes::Bytes,
    route_model_alias: String,
) -> Result<UnifiedEmbeddingRequest, CCProxyError> {
    match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            let client_request_payload: OpenAIEmbeddingRequest =
                serde_json::from_slice(&client_request_body)
                    .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
            from_openai_embedding(client_request_payload)
                .map_err(|e| CCProxyError::InternalError(e.to_string()))
        }
        ChatProtocol::Gemini => {
            let client_request_payload: GeminiEmbedRequest =
                serde_json::from_slice(&client_request_body)
                    .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
            from_gemini_embedding(client_request_payload, route_model_alias)
                .map_err(|e| CCProxyError::InternalError(e.to_string()))
        }
        ChatProtocol::Ollama => {
            if let Ok(payload) = serde_json::from_slice::<OllamaEmbedRequest>(&client_request_body)
            {
                return from_ollama_embed(payload)
                    .map_err(|e| CCProxyError::InternalError(e.to_string()));
            }
            let payload: OllamaEmbeddingsRequest = serde_json::from_slice(&client_request_body)
                .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
            from_ollama_embedding(payload).map_err(|e| CCProxyError::InternalError(e.to_string()))
        }
        ChatProtocol::Claude => Err(CCProxyError::InvalidProtocolError(
            "Claude protocol does not support embeddings".to_string(),
        )),
        ChatProtocol::Decision => Err(CCProxyError::InvalidProtocolError(
            "Decision protocol does not support embeddings".to_string(),
        )),
    }
}

pub async fn handle_embedding(
    chat_protocol: ChatProtocol,
    _client_headers: HeaderMap,
    _client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    route_model_alias: String,
    store_arc: Arc<MainStore>,
) -> Result<Response, CCProxyError> {
    if chat_protocol == ChatProtocol::Claude {
        return Err(CCProxyError::InvalidProtocolError(
            "Claude protocol does not support embeddings".to_string(),
        ));
    }

    let proxy_alias =
        get_proxy_alias_from_body(&chat_protocol, &client_request_body, &route_model_alias)?;

    let unified_request = build_unified_request(
        chat_protocol.clone(),
        client_request_body.clone(),
        route_model_alias.clone(),
    )?;

    let proxy_model = ModelResolver::get_ai_model_by_alias(
        store_arc.clone(),
        proxy_alias.clone(),
        group_name.as_deref(),
    )
    .await?;

    let backend_adapter: Arc<dyn BackendAdapter> = match proxy_model.chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => Arc::new(OpenAIBackendAdapter),
        ChatProtocol::Gemini => Arc::new(GeminiBackendAdapter),
        ChatProtocol::Ollama => Arc::new(OllamaBackendAdapter),
        ChatProtocol::Claude => Arc::new(ClaudeBackendAdapter),
        ChatProtocol::Decision => {
            return Err(CCProxyError::InvalidProtocolError(
                "Decision protocol does not support embeddings".to_string(),
            ))
        }
    };

    let client = reqwest::Client::new();
    let mut backend_headers = HeaderMap::new();

    let full_url = get_provider_embedding_full_url(
        proxy_model.chat_protocol.clone(),
        &proxy_model.base_url,
        &proxy_model.model,
        &proxy_model.api_key,
    );

    let mut request_builder = backend_adapter
        .adapt_embedding_request(
            &client,
            &unified_request,
            &proxy_model.api_key,
            &full_url,
            &proxy_model.model,
            &mut backend_headers,
        )
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

    // Add additional headers from client if needed, or specific ones
    for (key, value) in backend_headers.iter() {
        request_builder = request_builder.header(key, value);
    }

    // Get retry configuration from settings
    let max_retries =
        store_arc.get_config(CFG_CCPROXY_RETRY_ON_429, CFG_CCPROXY_RETRY_ON_429_DEFAULT);
    let retry_config = RetryConfig::from_settings(max_retries);

    // Send request with retry support for 429 status code
    let response = match send_with_retry(request_builder, &retry_config).await {
        Ok(response) => response,
        Err(error) => {
            let client_protocol_label = chat_protocol.to_string();
            let backend_protocol_label = proxy_model.chat_protocol.to_string();
            let diagnostic = diagnostic::from_message(
                "ccproxy",
                "backend_request",
                error.to_string(),
                diagnostic::request_frames(RequestContext {
                    request_id: "none",
                    client_protocol: &client_protocol_label,
                    backend_protocol: &backend_protocol_label,
                    client_model: &proxy_alias,
                    backend_model: &proxy_model.model,
                    provider_id: proxy_model.provider_id,
                    provider: &proxy_model.provider,
                    group: group_name.as_deref(),
                    path: "embedding",
                    compat: false,
                }),
            );
            diagnostic::emit(log::Level::Warn, &diagnostic);
            return Err(error);
        }
    };

    let status_code = response.status();
    if !status_code.is_success() {
        let error_headers = response.headers().clone();
        let error_body = response.text().await.unwrap_or_default();
        let error_message = error_body.clone();
        let client_protocol_label = chat_protocol.to_string();
        let backend_protocol_label = proxy_model.chat_protocol.to_string();
        let mut frames = diagnostic::request_frames(RequestContext {
            request_id: "none",
            client_protocol: &client_protocol_label,
            backend_protocol: &backend_protocol_label,
            client_model: &proxy_alias,
            backend_model: &proxy_model.model,
            provider_id: proxy_model.provider_id,
            provider: &proxy_model.provider,
            group: group_name.as_deref(),
            path: "embedding",
            compat: false,
        });
        frames.push(("response", Some(format!("status={status_code}"))));
        let diagnostic = diagnostic::from_upstream(
            "backend_error",
            &proxy_model.chat_protocol,
            status_code,
            &error_headers,
            error_body.as_bytes(),
            frames,
        );
        diagnostic::emit(log::Level::Warn, &diagnostic);
        if let Err(error) = store_arc.record_ccproxy_stat(CcproxyStat {
            id: None,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            client_model: proxy_alias,
            backend_model: proxy_model.model.clone(),
            provider_id: Some(proxy_model.provider_id),
            provider: proxy_model.provider.clone(),
            protocol: chat_protocol.to_string(),
            tool_compat_mode: 0,
            status_code: status_code.as_u16() as i32,
            error_message: Some(error_message),
            input_tokens: 0,
            output_tokens: 0,
            cache_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            audio_input_tokens: 0,
            audio_output_tokens: 0,
            estimated_cost: None,
            pricing_status: Some("unpriced".to_string()),
            pricing_snapshot: None,
            request_at: None,
        }) {
            let mut frames = diagnostic::request_frames(RequestContext {
                request_id: "none",
                client_protocol: &client_protocol_label,
                backend_protocol: &backend_protocol_label,
                client_model: &proxy_model.client_alias,
                backend_model: &proxy_model.model,
                provider_id: proxy_model.provider_id,
                provider: &proxy_model.provider,
                group: group_name.as_deref(),
                path: "embedding",
                compat: false,
            });
            frames.push(("response", Some(format!("status={status_code}"))));
            let diagnostic = diagnostic::from_error("ccproxy", "stat_enqueue", &error, frames);
            diagnostic::emit(log::Level::Error, &diagnostic);
        }
        return Err(CCProxyError::InternalError(format!(
            "Backend returned error ({}): {}",
            status_code, error_body
        )));
    }

    let body_bytes = response
        .bytes()
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

    let backend_response = crate::ccproxy::adapter::backend::BackendResponse {
        body: body_bytes,
        tool_compat_mode: false,
    };

    let unified_response = backend_adapter
        .adapt_embedding_response(backend_response)
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

    let output_adapter: Box<dyn OutputAdapter> = match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            Box::new(OutputAdapterEnum::OpenAI(OpenAIOutputAdapter))
        }
        ChatProtocol::Claude => Box::new(OutputAdapterEnum::Claude(ClaudeOutputAdapter)),
        ChatProtocol::Gemini => Box::new(OutputAdapterEnum::Gemini(GeminiOutputAdapter)),
        ChatProtocol::Ollama => Box::new(OutputAdapterEnum::Ollama(OllamaOutputAdapter)),
        ChatProtocol::Decision => {
            return Err(CCProxyError::InvalidProtocolError(
                "Decision protocol does not support embeddings".to_string(),
            ))
        }
    };

    let usage = unified_response.usage.clone();

    let (estimated_cost, pricing_status, pricing_snapshot) =
        crate::ccproxy::helper::stat_guard::finalize_pricing(
            usage.input_tokens as i64,
            usage.output_tokens as i64,
            usage.cache_read_input_tokens.unwrap_or(0) as i64,
            usage.cache_creation_input_tokens.unwrap_or(0) as i64,
            usage.thoughts_tokens.unwrap_or(0) as i64,
            usage.audio_input_tokens.unwrap_or(0) as i64,
            usage.audio_output_tokens.unwrap_or(0) as i64,
            proxy_model.pricing.as_ref(),
        );

    let final_response = output_adapter
        .adapt_embedding_response(unified_response)
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

    // Record stats
    if let Err(error) = store_arc.record_ccproxy_stat(CcproxyStat {
        id: None,
        workflow_session_id: None,
        workflow_task_run_id: None,
        workflow_segment_id: None,
        root_session_id: None,
        root_task_run_id: None,
        request_kind: None,
        client_model: proxy_alias,
        backend_model: proxy_model.model.clone(),
        provider_id: Some(proxy_model.provider_id),
        provider: proxy_model.provider.clone(),
        protocol: chat_protocol.to_string(),
        tool_compat_mode: 0,
        status_code: status_code.as_u16() as i32,
        error_message: None,
        input_tokens: usage.input_tokens as i64,
        output_tokens: usage.output_tokens as i64,
        cache_tokens: usage
            .cache_read_input_tokens
            .or(usage.cached_content_tokens)
            .unwrap_or(0) as i64,
        cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or(0) as i64,
        reasoning_tokens: usage.thoughts_tokens.unwrap_or(0) as i64,
        audio_input_tokens: usage.audio_input_tokens.unwrap_or(0) as i64,
        audio_output_tokens: usage.audio_output_tokens.unwrap_or(0) as i64,
        estimated_cost,
        pricing_status,
        pricing_snapshot,
        request_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
    }) {
        let client_protocol_label = chat_protocol.to_string();
        let backend_protocol_label = proxy_model.chat_protocol.to_string();
        let mut frames = diagnostic::request_frames(RequestContext {
            request_id: "none",
            client_protocol: &client_protocol_label,
            backend_protocol: &backend_protocol_label,
            client_model: &proxy_model.client_alias,
            backend_model: &proxy_model.model,
            provider_id: proxy_model.provider_id,
            provider: &proxy_model.provider,
            group: group_name.as_deref(),
            path: "embedding",
            compat: false,
        });
        frames.push(("response", Some(format!("status={status_code}"))));
        let diagnostic = diagnostic::from_error("ccproxy", "stat_enqueue", &error, frames);
        diagnostic::emit(log::Level::Error, &diagnostic);
    }
    Ok(final_response)
}

#[cfg(test)]
mod tests {
    use super::handle_embedding;
    use crate::ccproxy::helper::CcproxyQuery;
    use crate::ccproxy::ChatProtocol;
    use crate::constants::CFG_CHAT_COMPLETION_PROXY;
    use crate::db::{MainStore, ModelConfig};
    use axum::{body::Body, response::Response, routing::post, Router};
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn embedding_upstream_error_surfaces_status_body_and_records_failure_stat() {
        let app = Router::new().route(
            "/embeddings",
            post(|| async {
                Response::builder()
                    .status(http::StatusCode::TOO_MANY_REQUESTS)
                    .header("content-type", "application/json")
                    .header("x-upstream", "kept")
                    .body(Body::from(
                        r#"{"error":{"message":"embedding rate limited"}}"#,
                    ))
                    .expect("mock embedding error should build")
            }),
        );
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
            MainStore::new(directory.path().join("embedding-errors.db"))
                .expect("test store should initialize"),
        );
        let provider_id = store
            .add_ai_model(
                "provider".to_string(),
                vec![ModelConfig {
                    id: "embed-model".to_string(),
                    ..Default::default()
                }],
                "embed-model".to_string(),
                "openai".to_string(),
                format!("http://{address}"),
                "secret".to_string(),
                0,
                0.0,
                0.0,
                0,
                false,
                None,
            )
            .expect("test provider should be inserted");
        store
            .set_config(
                CFG_CHAT_COMPLETION_PROXY,
                &json!({
                    "default": {
                        "embed-alias": [{"id": provider_id, "model": "embed-model"}]
                    }
                }),
            )
            .expect("test proxy config should be stored");

        let error = handle_embedding(
            ChatProtocol::OpenAI,
            reqwest::header::HeaderMap::new(),
            CcproxyQuery {
                key: None,
                _debug: None,
            },
            bytes::Bytes::from_static(br#"{"model":"embed-alias","input":"hello"}"#),
            None,
            "embed-alias".to_string(),
            store.clone(),
        )
        .await
        .expect_err("embedding upstream errors must fail explicitly");

        match error {
            crate::ccproxy::errors::CCProxyError::InternalError(message) => {
                assert!(message.contains("429"));
                assert!(message.contains("embedding rate limited"));
            }
            other => panic!("expected an internal embedding error, got {other:?}"),
        }

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("embedding error stat should flush");
        let stat = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT status_code, error_message, input_tokens, output_tokens
                     FROM ccproxy_stats",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i32>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )?)
            })
            .await
            .expect("embedding error stat should persist");
        assert_eq!(stat.0, 429);
        assert!(stat
            .1
            .is_some_and(|message| message.contains("embedding rate limited")));
        assert_eq!((stat.2, stat.3), (0, 0));
        server.abort();
    }

    #[derive(Clone, Copy)]
    struct EmbeddingErrorBody {
        content_type: &'static str,
        body: &'static [u8],
    }

    const EMBEDDING_ERROR_BODIES: [EmbeddingErrorBody; 3] = [
        EmbeddingErrorBody {
            content_type: "application/json",
            body: br#"{"error":{"message":"embedding upstream rejected"}}"#,
        },
        EmbeddingErrorBody {
            content_type: "text/plain",
            body: b"",
        },
        EmbeddingErrorBody {
            content_type: "text/html",
            body: b"<html><body><h1>Embedding upstream rejected</h1></body></html>",
        },
    ];

    #[tokio::test]
    async fn embedding_supported_protocols_characterize_http_error_mapping() {
        use axum::extract::State;
        use std::collections::HashMap;
        use std::sync::Mutex;

        type UpstreamState = Arc<Mutex<(u16, &'static str, &'static [u8])>>;
        let upstream_state: UpstreamState = Arc::new(Mutex::new((
            reqwest::StatusCode::UNAUTHORIZED.as_u16(),
            EMBEDDING_ERROR_BODIES[0].content_type,
            EMBEDDING_ERROR_BODIES[0].body,
        )));
        let app = Router::new()
            .fallback(|State(state): State<UpstreamState>| async move {
                let (status, content_type, body) = {
                    let state = state.lock().expect("upstream state should lock");
                    *state
                };
                Response::builder()
                    .status(
                        reqwest::StatusCode::from_u16(status)
                            .expect("test upstream status should be valid"),
                    )
                    .header("content-type", content_type)
                    .header("content-length", body.len().to_string())
                    .header("x-upstream", "kept")
                    .header("x-request-id", "embedding-upstream-request")
                    .body(Body::from(body.to_vec()))
                    .expect("mock embedding response should build")
            })
            .with_state(upstream_state.clone());
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
            MainStore::new(directory.path().join("embedding-protocol-errors.db"))
                .expect("test store should initialize"),
        );
        let mut provider_ids = Vec::new();
        for protocol in ["openai", "gemini", "ollama"] {
            // The Gemini embedding URL helper strips the suffix after the last colon. A bare
            // loopback base URL would lose its port, so keep an action-like path segment that
            // the mock fallback still accepts.
            let base_url = if protocol == "gemini" {
                format!("http://{address}/google:generateContent")
            } else {
                format!("http://{address}")
            };
            let provider_id = store
                .add_ai_model(
                    format!("{protocol}-provider"),
                    vec![ModelConfig {
                        id: "embed-model".to_string(),
                        ..Default::default()
                    }],
                    "embed-model".to_string(),
                    protocol.to_string(),
                    base_url,
                    "secret".to_string(),
                    0,
                    0.0,
                    0.0,
                    0,
                    false,
                    None,
                )
                .expect("test provider should be inserted");
            provider_ids.push(provider_id);
        }
        store
            .set_config(
                CFG_CHAT_COMPLETION_PROXY,
                &json!({
                    "default": {
                        "openai-alias": [{"id": provider_ids[0], "model": "embed-model"}],
                        "gemini-alias": [{"id": provider_ids[1], "model": "embed-model"}],
                        "ollama-alias": [{"id": provider_ids[2], "model": "embed-model"}]
                    }
                }),
            )
            .expect("test proxy config should be stored");

        let protocols = [
            (
                ChatProtocol::OpenAI,
                "openai-alias",
                br#"{"model":"openai-alias","input":"hello"}"#.as_slice(),
            ),
            (
                ChatProtocol::Gemini,
                "gemini-alias",
                br#"{"model":"gemini-alias","content":{"parts":[{"text":"hello"}]}}"#.as_slice(),
            ),
            (
                ChatProtocol::Ollama,
                "ollama-alias",
                br#"{"model":"ollama-alias","input":"hello"}"#.as_slice(),
            ),
        ];

        let mut expected_stats = HashMap::new();
        for (client_protocol, alias, request_body) in protocols {
            for status in [
                reqwest::StatusCode::UNAUTHORIZED.as_u16(),
                reqwest::StatusCode::TOO_MANY_REQUESTS.as_u16(),
                reqwest::StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            ] {
                for variant in EMBEDDING_ERROR_BODIES {
                    *upstream_state.lock().expect("upstream state should lock") =
                        (status, variant.content_type, variant.body);

                    let error = handle_embedding(
                        client_protocol.clone(),
                        reqwest::header::HeaderMap::new(),
                        CcproxyQuery {
                            key: None,
                            _debug: None,
                        },
                        bytes::Bytes::from_static(request_body),
                        None,
                        alias.to_string(),
                        store.clone(),
                    )
                    .await
                    .expect_err("embedding upstream errors must fail explicitly");

                    let expected_body = String::from_utf8_lossy(variant.body).to_string();
                    let status_label = match status {
                        401 => "401 Unauthorized",
                        429 => "429 Too Many Requests",
                        503 => "503 Service Unavailable",
                        other => panic!("unexpected test status {other}"),
                    };
                    match error {
                        crate::ccproxy::errors::CCProxyError::InternalError(message) => {
                            assert_eq!(
                                message,
                                format!("Backend returned error ({status_label}): {expected_body}")
                            );
                        }
                        other => panic!("expected an internal embedding error, got {other:?}"),
                    }

                    *expected_stats
                        .entry((client_protocol.to_string(), status as i32))
                        .or_insert(0_i64) += 1;
                }
            }
        }

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("embedding error stats should flush");
        let stats = runtime
            .read(|conn| {
                let mut statement = conn.prepare(
                    "SELECT status_code, error_message, input_tokens, output_tokens, protocol
                     FROM ccproxy_stats",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i32>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .expect("embedding error stats should be readable");

        assert_eq!(stats.len(), 27);
        let mut actual_stats = HashMap::new();
        for (status, error_message, input_tokens, output_tokens, protocol) in stats {
            assert_eq!((input_tokens, output_tokens), (0, 0));
            let message = error_message.expect("embedding errors should record a body");
            assert!(
                EMBEDDING_ERROR_BODIES
                    .iter()
                    .any(|variant| String::from_utf8_lossy(variant.body) == message),
                "unexpected embedding error body in stat: {message:?}"
            );
            *actual_stats
                .entry((protocol, status))
                .or_insert(0_i64) += 1;
        }
        assert_eq!(actual_stats, expected_stats);
        server.abort();
    }

    #[tokio::test]
    async fn embedding_claude_is_explicitly_unsupported_without_recording_a_stat() {
        let directory = tempdir().expect("test directory should exist");
        let store = Arc::new(
            MainStore::new(directory.path().join("embedding-claude.db"))
                .expect("test store should initialize"),
        );

        let error = handle_embedding(
            ChatProtocol::Claude,
            reqwest::header::HeaderMap::new(),
            CcproxyQuery {
                key: None,
                _debug: None,
            },
            bytes::Bytes::from_static(br#"{"model":"claude-embed","input":"hello"}"#),
            None,
            "claude-embed".to_string(),
            store.clone(),
        )
        .await
        .expect_err("Claude embeddings must stay explicitly unsupported");

        match error {
            crate::ccproxy::errors::CCProxyError::InvalidProtocolError(message) => {
                assert_eq!(message, "Claude protocol does not support embeddings");
            }
            other => panic!("expected an invalid protocol error, got {other:?}"),
        }

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("stat queue should flush");
        let stat_count = runtime
            .read(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM ccproxy_stats", [], |row| {
                    row.get::<_, i64>(0)
                })?)
            })
            .await
            .expect("stat count should be readable");
        assert_eq!(stat_count, 0);
    }
}
