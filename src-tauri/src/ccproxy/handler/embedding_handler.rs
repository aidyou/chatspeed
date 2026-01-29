use axum::response::Response;
use reqwest::header::HeaderMap;
use std::sync::{Arc, RwLock};

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
    helper::{get_provider_embedding_full_url, CcproxyQuery, ModelResolver},
    openai::OpenAIEmbeddingRequest,
    types::ollama::{OllamaEmbedRequest, OllamaEmbeddingsRequest},
};
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
    }
}

pub async fn handle_embedding(
    chat_protocol: ChatProtocol,
    _client_headers: HeaderMap,
    _client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    route_model_alias: String,
    store_arc: Arc<RwLock<MainStore>>,
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

    let response = request_builder
        .send()
        .await
        .map_err(|e| CCProxyError::InternalError(format!("Request to backend failed: {}", e)))?;

    let status_code = response.status();
    if !status_code.is_success() {
        let error_body = response.text().await.unwrap_or_default();
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
    };

    let final_response = output_adapter
        .adapt_embedding_response(unified_response)
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

        // Record stats
        if let Ok(store) = store_arc.read() {
            let _ = store.record_ccproxy_stat(CcproxyStat {
                id: None,
                client_model: proxy_alias,
                backend_model: proxy_model.model.clone(),
                provider: proxy_model.provider.clone(),
                protocol: chat_protocol.to_string(),
                tool_compat_mode: 0,
                status_code: status_code.as_u16() as i32,
                error_message: None,
                input_tokens: 0, 
                output_tokens: 0,
                cache_tokens: 0,
                request_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            });
        } else {
            log::error!("Failed to acquire store lock for recording ccproxy stats");
        }
        Ok(final_response)
}
