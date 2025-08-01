use crate::ai::interaction::ChatProtocol;
use crate::ccproxy::errors::{CCProxyError, ProxyResult};
use crate::ccproxy::helper::{get_provider_full_url, should_forward_header, ModelResolver};
use crate::ccproxy::types::ProxyModel;
use crate::db::MainStore;
use futures_util::stream::StreamExt;
use reqwest::header::{HeaderName as ReqwestHeaderName, HeaderValue as ReqwestHeaderValue};
use rust_i18n::t;
use serde_json::Value;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use warp::{hyper::Body, Reply};

pub async fn handle_direct_forward(
    client_headers: warp::http::HeaderMap,
    client_request_body: bytes::Bytes,
    proxy_model: ProxyModel,
    is_streaming_request: bool,
    main_store_arc: Arc<Mutex<MainStore>>,
    log_to_file: bool,
) -> ProxyResult<impl Reply> {
    let http_client =
        ModelResolver::build_http_client(main_store_arc.clone(), proxy_model.metadata.clone())
            .await?;

    let full_url = get_provider_full_url(
        proxy_model.chat_protocol.clone(),
        &proxy_model.base_url,
        &proxy_model.model,
        &proxy_model.api_key,
        is_streaming_request,
    );

    let mut reqwest_headers = reqwest::header::HeaderMap::new();

    // Forward relevant headers from client
    for (name, value) in client_headers.iter() {
        if should_forward_header(name.as_str()) {
            match ReqwestHeaderValue::from_bytes(value.as_bytes()) {
                Ok(h) => {
                    if let Ok(n) = ReqwestHeaderName::from_str(name.clone().as_str()) {
                        reqwest_headers.insert(n, h);
                    }
                }
                Err(_) => {}
            }
        }
    }

    // Add protocol-specific headers
    match proxy_model.chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            if !proxy_model.api_key.is_empty() {
                match ReqwestHeaderValue::from_str(&format!("Bearer {}", proxy_model.api_key)) {
                    Ok(h) => {
                        reqwest_headers.insert(reqwest::header::AUTHORIZATION, h);
                    }
                    Err(_) => {}
                }
            }
        }
        ChatProtocol::Claude => {
            match ReqwestHeaderValue::from_str(&proxy_model.api_key) {
                Ok(h) => {
                    reqwest_headers.insert(ReqwestHeaderName::from_static("x-api-key"), h);
                }
                Err(_) => {}
            }

            reqwest_headers.insert(
                ReqwestHeaderName::from_static("anthropic-version"),
                ReqwestHeaderValue::from_static("2023-06-01"),
            );
            if let Some(beta_header) = client_headers.get("anthropic-beta") {
                match ReqwestHeaderValue::from_bytes(beta_header.as_bytes()) {
                    Ok(h) => {
                        reqwest_headers.insert(ReqwestHeaderName::from_static("anthropic-beta"), h);
                    }
                    Err(_) => {}
                }
            }
        }
        ChatProtocol::Gemini => {
            // API key is in the URL for Gemini
        }
        ChatProtocol::Ollama => {
            // No auth for Ollama
        }
    }

    // Set Content-Type and Accept headers
    reqwest_headers.insert(
        reqwest::header::CONTENT_TYPE,
        ReqwestHeaderValue::from_static("application/json"),
    );
    if is_streaming_request {
        reqwest_headers.insert(
            reqwest::header::ACCEPT,
            ReqwestHeaderValue::from_static("text/event-stream"),
        );
    } else {
        reqwest_headers.insert(
            reqwest::header::ACCEPT,
            ReqwestHeaderValue::from_static("application/json"),
        );
    }

    // Modify the request body to replace the model alias
    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        warp::reject::custom(CCProxyError::InternalError(format!(
            "Failed to deserialize request body: {}",
            e
        )))
    })?;

    if let Some(model_field) = body_json.get_mut("model") {
        *model_field = Value::String(proxy_model.model.clone());
    }

    let modified_body = serde_json::to_vec(&body_json).map_err(|e| {
        warp::reject::custom(CCProxyError::InternalError(format!(
            "Failed to serialize modified body: {}",
            e
        )))
    })?;

    let onward_request_builder = http_client
        .post(&full_url)
        .headers(reqwest_headers)
        .body(modified_body);

    // Send request
    let target_response = onward_request_builder.send().await.map_err(|e| {
        warp::reject::custom(CCProxyError::InternalError(format!(
            "Request to backend failed: {}",
            e
        )))
    })?;

    // Handle response
    let status_code = target_response.status();
    let response_headers = target_response.headers().clone();

    if !status_code.is_success() {
        let error_body_bytes = target_response.bytes().await.map_err(|e| {
            CCProxyError::InternalError(
                t!("network.response_read_error", error = e.to_string()).to_string(),
            )
        })?;
        let mut response = warp::reply::Response::new(Body::from(error_body_bytes));
        *response.status_mut() = warp::http::StatusCode::from_u16(status_code.as_u16()).unwrap();

        for (name, value) in response_headers.iter() {
            if let Ok(warp_name) = warp::http::header::HeaderName::from_bytes(name.as_ref()) {
                if let Ok(warp_value) = warp::http::header::HeaderValue::from_bytes(value.as_ref())
                {
                    response.headers_mut().insert(warp_name, warp_value);
                }
            }
        }
        return Ok(response.into_response());
    }

    let mut response_builder = warp::http::Response::builder()
        .status(warp::http::StatusCode::from_u16(status_code.as_u16()).unwrap());
    for (name, value) in response_headers.iter() {
        response_builder = response_builder.header(name.as_str(), value.as_bytes());
    }

    if is_streaming_request {
        let log_to_file_clone = log_to_file.clone();
        let stream = target_response.bytes_stream().map(move |chunk| {
            chunk.map(|bytes| {
                if log_to_file_clone.clone(){
                    log::info!(target: "ccproxy_logger", "Direct Stream Response Chunk: {}", String::from_utf8_lossy(&bytes));
                }
                bytes
            })
        });
        let body = Body::wrap_stream(stream);
        let response = response_builder.body(body).unwrap();
        Ok(response.into_response())
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;
        if log_to_file {
            log::info!(target: "ccproxy_logger", "Direct Response Body: {}\n---", String::from_utf8_lossy(&body_bytes));
        }
        let response = response_builder.body(Body::from(body_bytes)).unwrap();
        Ok(response.into_response())
    }
}
