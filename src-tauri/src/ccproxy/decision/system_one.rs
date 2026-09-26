//! HTTP adapter for System One evaluation and same-origin model discovery.
use super::{catalog, types::{Answer, DecisionError, DecisionRequest, DecisionResponse}};
use crate::{ai::{network::ProxyType, util::get_proxy_type_for_key}, ccproxy::helper::CC_PROXY_ROTATOR, db::MainStore};
use bytes::Bytes;
use reqwest::{header::HeaderMap, Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{sync::Arc, time::{Duration, Instant}};

const TIMEOUT: Duration = Duration::from_secs(4);
const MAX_BODY: usize = 256 * 1024;

/// Upstream response kept raw so callers can forward status, headers and body untouched.
pub(crate) struct RawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

fn client(store: Arc<MainStore>, mut metadata: Option<Value>, key_index: Option<usize>) -> Result<Client, DecisionError> {
    crate::commands::chat::setup_chat_proxy(store, &mut metadata)
        .map_err(|_| DecisionError::Transport("proxy configuration"))?;
    let mut builder = Client::builder().timeout(TIMEOUT).redirect(reqwest::redirect::Policy::none());
    builder = match get_proxy_type_for_key(metadata, key_index) {
        ProxyType::None => builder.no_proxy(),
        ProxyType::System => builder,
        ProxyType::Http(server, username, password) => {
            let mut proxy = reqwest::Proxy::all(&server).map_err(|_| DecisionError::Transport("invalid proxy"))?;
            if let (Some(username), Some(password)) = (username, password) {
                proxy = proxy.basic_auth(&username, &password);
            }
            builder.proxy(proxy)
        }
    };
    builder.build().map_err(|_| DecisionError::Transport("client construction"))
}

async fn send_raw(client: &Client, url: Url, key: &str, payload: Option<&Value>) -> Result<RawResponse, DecisionError> {
    if key.trim().is_empty() { return Err(DecisionError::Unavailable); }
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    for attempt in 0..=1 {
        let request = match payload {
            Some(body) => client.post(url.clone()).bearer_auth(key.trim()).json(body),
            None => client.get(url.clone()).bearer_auth(key.trim()),
        };
        let response = tokio::time::timeout_at(deadline, request.send()).await.map_err(|_| DecisionError::Transport("timeout"))?.map_err(|_| DecisionError::Transport("request"))?;
        let status = response.status();
        if matches!(status.as_u16(), 429 | 529) && attempt == 0 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            continue;
        }
        if response.content_length().is_some_and(|length| length > MAX_BODY as u64) { return Err(DecisionError::InvalidResponse("oversized body")); }
        let headers = response.headers().clone();
        let body = tokio::time::timeout_at(deadline, response.bytes()).await.map_err(|_| DecisionError::Transport("timeout"))?.map_err(|_| DecisionError::Transport("response read"))?;
        if body.len() > MAX_BODY { return Err(DecisionError::InvalidResponse("oversized body")); }
        return Ok(RawResponse { status, headers, body });
    }
    Err(DecisionError::Transport("retry exhausted"))
}

async fn send(client: &Client, url: Url, key: &str, payload: Option<&DecisionRequest>) -> Result<Value, DecisionError> {
    let payload = payload.map(|request| serde_json::to_value(request).map_err(|_| DecisionError::InvalidRequest("payload encoding"))).transpose()?;
    let response = send_raw(client, url, key, payload.as_ref()).await?;
    if !response.status.is_success() { return Err(DecisionError::Http(response.status.as_u16())); }
    serde_json::from_slice(&response.body).map_err(|_| DecisionError::InvalidResponse("JSON"))
}

/// Forwards a decision payload with a credential resolved by the caller, keeping the upstream status intact.
pub(crate) async fn forward(store: Arc<MainStore>, endpoint: &str, key: &str, key_index: Option<usize>, metadata: Option<Value>, payload: &Value) -> Result<RawResponse, DecisionError> {
    let url = catalog::endpoint(endpoint)?;
    let (adapter, rule) = catalog::resolve(&url)?;
    log::debug!("decision forward: rule={}, adapter={:?}", rule, adapter);
    match adapter {
        catalog::Adapter::SystemOne => {}
    }
    let client = client(store, metadata, key_index)?;
    send_raw(&client, url, key, Some(payload)).await
}

fn result_summary(response: &DecisionResponse) -> String {
    let answers: serde_json::Map<String, Value> = response.answers.iter().map(|(id, answer)| {
        let summary = match answer {
            Answer::Choice { choice, probabilities, confidence } => serde_json::json!({
                "type": "choice",
                "choice": choice,
                "probability": probabilities.get(choice),
                "confidence": confidence,
            }),
            Answer::Noul { noul } => serde_json::json!({ "type": "noul", "noul": noul }),
            Answer::Score { score, confidence, .. } => serde_json::json!({
                "type": "score",
                "score": score,
                "confidence": confidence,
            }),
        };
        (id.clone(), summary)
    }).collect();
    Value::Object(answers).to_string()
}

/// Evaluate typed questions using only the configured endpoint and credential.
pub(crate) async fn evaluate(store: Arc<MainStore>, provider_id: i64, request: DecisionRequest) -> Result<DecisionResponse, DecisionError> {
    request.validate()?;
    let provider = store.config.get_ai_model_by_id(provider_id).map_err(|_| DecisionError::Unavailable)?;
    if provider.disabled || provider.api_protocol != "decision" || !provider.models.iter().any(|model| model.id == request.model) { return Err(DecisionError::Unavailable); }
    let url = catalog::endpoint(&provider.base_url)?;
    let (adapter, rule) = catalog::resolve(&url)?;
    let keys: Vec<&str> = provider.api_key.lines().map(str::trim).filter(|key| !key.is_empty()).collect();
    if keys.is_empty() { return Err(DecisionError::Unavailable); }
    let key_index = if keys.len() == 1 { 0 } else { CC_PROXY_ROTATOR.get_next_target_index(&format!("decision/{provider_id}"), keys.len()) };
    let started = Instant::now();
    let result: Result<DecisionResponse, DecisionError> = async {
        match adapter {
            catalog::Adapter::SystemOne => {
                let client = client(store, provider.metadata, Some(key_index))?;
                let value = send(&client, url, keys[key_index], Some(&request)).await?;
                let response: DecisionResponse = serde_json::from_value(value).map_err(|_| DecisionError::InvalidResponse("answer schema"))?;
                response.validate(&request)?;
                Ok(response)
            }
        }
    }.await;
    match &result {
        Ok(response) => log::info!("decision provider={provider_id} rule={rule} model={} elapsed_ms={} input_tokens={} output_tokens={} answers={}", response.model, started.elapsed().as_millis(), response.usage.input_tokens, response.usage.output_tokens, result_summary(response)),
        Err(error) => log::warn!("decision provider={provider_id} rule={rule} elapsed_ms={} failed: {error}", started.elapsed().as_millis()),
    }
    result
}

#[derive(Deserialize)]
struct ListResponse { models: Option<Vec<ListModel>>, data: Option<Vec<ListModel>> }
#[derive(Deserialize)]
struct ListModel { name: Option<String>, id: Option<String> }

/// Discover models only when the complete evaluation URL has a known same-origin mapping.
pub(crate) async fn list_models(store: Arc<MainStore>, endpoint: &str, key: &str, metadata: Option<Value>) -> Result<Vec<(String, String)>, DecisionError> {
    let evaluation = catalog::endpoint(endpoint)?;
    let (_adapter, _rule) = catalog::resolve(&evaluation)?;
    let url = catalog::models_endpoint(endpoint)?;
    let client = client(store, metadata, None)?;
    let value = send(&client, url, key, None).await?;
    parse_model_list(value)
}

fn parse_model_list(value: Value) -> Result<Vec<(String, String)>, DecisionError> {
    let list: ListResponse = serde_json::from_value(value).map_err(|_| DecisionError::InvalidResponse("model list schema"))?;
    let models = list.models.filter(|models| !models.is_empty()).or(list.data)
        .ok_or(DecisionError::InvalidResponse("missing model list"))?;
    if models.is_empty() || models.len() > 1000 { return Err(DecisionError::InvalidResponse("model list entries")); }
    models.into_iter().map(|model| {
        let name = model.name.as_deref().filter(|value| !value.trim().is_empty())
            .or_else(|| model.id.as_deref().filter(|value| !value.trim().is_empty()))
            .ok_or(DecisionError::InvalidResponse("model name or id"))?.to_string();
        Ok((name.clone(), name))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decision_log_summary_contains_outcomes_but_not_input_or_full_distribution() {
        let response = DecisionResponse {
            model: "jev-latest".into(),
            answers: std::collections::BTreeMap::from([
                ("language".into(), Answer::Choice {
                    choice: "zh".into(),
                    probabilities: std::collections::BTreeMap::from([("zh".into(), 0.96), ("en".into(), 0.04)]),
                    confidence: 0.93,
                }),
                ("urgency".into(), Answer::Noul { noul: 0.8 }),
                ("risk".into(), Answer::Score {
                    score: 1.2,
                    confidence: 0.95,
                    legend: std::collections::BTreeMap::from([("0".into(), "private input".into())]),
                    probabilities: std::collections::BTreeMap::from([("0".into(), 1.0)]),
                }),
            ]),
            usage: super::super::types::Usage { input_tokens: 1, output_tokens: 2 },
        };
        let summary: Value = serde_json::from_str(&result_summary(&response)).unwrap();
        assert_eq!(summary["language"]["choice"], "zh");
        assert_eq!(summary["language"]["probability"], 0.96);
        assert_eq!(summary["language"]["confidence"], 0.93);
        assert_eq!(summary["urgency"]["noul"], 0.8);
        assert_eq!(summary["risk"]["score"], 1.2);
        assert!(!result_summary(&response).contains("private input"));
        assert!(!result_summary(&response).contains("\"en\""));
    }

    #[test]
    fn model_list_prefers_name_then_id_across_official_and_chat_shapes() {
        assert_eq!(parse_model_list(json!({"models":[{"name":"jev-latest","description":"Stable Jev","release_date":"2026-01-01"}]})).unwrap(), vec![("jev-latest".to_string(), "jev-latest".to_string())]);
        assert_eq!(parse_model_list(json!({"data":[{"id":"siliconflow-chat-model","object":"model"}]})).unwrap()[0].0, "siliconflow-chat-model");
        assert_eq!(parse_model_list(json!({"models":[{"name":"preferred","id":"fallback"},{"id":"second"}]})).unwrap(), vec![("preferred".into(), "preferred".into()), ("second".into(), "second".into())]);
        assert_eq!(parse_model_list(json!({"models":[{"id":"official"}],"data":[{"id":"chat"}]})).unwrap()[0].0, "official");
        assert_eq!(parse_model_list(json!({"models":[],"data":[{"id":"fallback"}]})).unwrap()[0].0, "fallback");
        assert!(parse_model_list(json!({"models":[]})).is_err());
        assert!(parse_model_list(json!({"data":[{}]})).is_err());
        assert!(parse_model_list(json!({"models":[{"name":"  ","id":""}]})).is_err());
        assert!(parse_model_list(json!({"unknown":[{"id":"ignored"}]})).is_err());
    }

    #[tokio::test]
    async fn post_uses_exact_endpoint_and_bearer_without_redirect() {
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/v1/systemone", listener.local_addr().unwrap().port())).unwrap();
        assert_eq!(catalog::resolve(&url).unwrap().1, "default_system_one");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let length = socket.read(&mut buffer).await.unwrap();
            let raw = String::from_utf8_lossy(&buffer[..length]);
            assert!(raw.starts_with("POST /v1/systemone HTTP/1.1"));
            assert!(raw.to_ascii_lowercase().contains("authorization: bearer secret"));
            assert!(raw.contains("\"state\":\"我的提现已经连续三天失败了，客服一直没回复，麻烦尽快处理！\""));
            assert!(raw.contains("\"model\":\"Kev-4b\""));
            assert!(raw.contains("\"is_urgent\""));
            let body = b"{\"model\":\"Kev-4b\",\"answers\":{\"is_urgent\":{\"type\":\"noul\",\"noul\":0.9}},\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });
        let request = DecisionRequest { state: "我的提现已经连续三天失败了，客服一直没回复，麻烦尽快处理！".into(), model: "Kev-4b".into(), questions: std::collections::BTreeMap::from([("is_urgent".into(), super::super::types::Question::Noul { instructions: "这条内容是否表达了紧迫性？".into() })]) };
        let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let value = send(&client, url, "secret", Some(&request)).await.unwrap();
        assert_eq!(value["usage"]["input_tokens"], 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn same_origin_models_request_uses_official_shape_and_bearer() {
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let evaluation = format!("http://127.0.0.1:{}/v1/systemone", listener.local_addr().unwrap().port());
        let listing = catalog::models_endpoint(&evaluation).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 2048];
            let length = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            assert!(request.starts_with("GET /v1/models HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("authorization: bearer secret"));
            let body = br#"{"models":[{"name":"jev-latest","description":"Stable Jev","release_date":"2026-01-01"}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        assert_eq!(parse_model_list(send(&client, listing, "secret", None).await.unwrap()).unwrap()[0].0, "jev-latest");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn same_origin_chat_shape_can_be_imported_as_fallback() {
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://127.0.0.1:{}/v1/systemone", listener.local_addr().unwrap().port());
        let listing = catalog::models_endpoint(&endpoint).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 2048];
            let length = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            assert!(request.starts_with("GET /v1/models HTTP/1.1"));
            assert!(request.to_ascii_lowercase().contains("authorization: bearer secret"));
            let body = br#"{"object":"list","data":[{"id":"chat-model","object":"model","created":1}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });
        let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        assert_eq!(parse_model_list(send(&client, listing, "secret", None).await.unwrap()).unwrap()[0].0, "chat-model");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn errors_and_retries_fail_closed() {
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/v1/systemone", listener.local_addr().unwrap().port())).unwrap();
        let server = tokio::spawn(async move {
            for (index, status) in [429, 529].into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 2048];
                let _ = socket.read(&mut buffer).await.unwrap();
                assert!(index < 2);
                socket.write_all(format!("HTTP/1.1 {status} Error\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
            }
        });
        let client = Client::builder().no_proxy().build().unwrap();
        assert!(matches!(send(&client, url, "secret", None).await, Err(DecisionError::Http(529))));
        server.await.unwrap();
        for status in [401, 422] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = Url::parse(&format!("http://127.0.0.1:{}/v1/models", listener.local_addr().unwrap().port())).unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 2048];
                let _ = socket.read(&mut buffer).await.unwrap();
                socket.write_all(format!("HTTP/1.1 {status} Error\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
            });
            assert!(matches!(send(&client, url, "secret", None).await, Err(DecisionError::Http(code)) if code == status));
            server.await.unwrap();
        }
    }
}
