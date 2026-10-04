//! Runtime-only fixed `web_fetch` / `web_search` tools backed by the client
//! WebView capability bridge.
//!
//! The desktop-free runtime never links a Tauri WebView, so these two fixed
//! tools do not execute any web access locally. Each call resolves the live
//! client bridge that declares the capability, enqueues one typed invocation on
//! the same [`ClientBridgeRegistry`] the control plane's bridge routes own, and
//! maps the bridge outcome onto a tool result. With no live bridge the tool
//! fails closed with a structured `unavailable` result instead of silently
//! falling back to a local scraper/search stack (which is not compiled into the
//! runtime at all) or fabricating a result.
//!
//! The capability set and each argument schema are the fixed bridge contract:
//! this module accepts exactly the declared arguments through the shared
//! [`chatspeed_contracts::validate_capability_arguments`] gate and never exposes
//! a generic RPC.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::tool_manager::{NativeToolResult, ToolDefinition};
use crate::tools::{
    ToolCallResult, ToolCategory, ToolError, ToolScope, TOOL_WEB_FETCH, TOOL_WEB_SEARCH,
};
use crate::workflow::react::client::http::client_bridge::{
    BridgeError, BridgeOutcome, ClientBridgeRegistry,
};
use chatspeed_contracts::{
    validate_capability_arguments, ClientCapabilityError, ClientCapabilityResult,
    ClientCapabilityStatus, BRIDGE_SCHEMA_VERSION,
};

/// Bound on one runtime-side capability invocation.
///
/// The desktop dispatcher bounds its own execution at 55s, which stays under
/// this 60s bound so the client reports its own typed timeout before the runtime
/// reports a bare deadline. The client never has to guess how long it may run.
const CLIENT_CAPABILITY_DEADLINE: Duration = Duration::from_secs(60);

/// Stable error code for a capability with no live bridge.
const CODE_UNAVAILABLE: &str = "unavailable";

/// A fixed, model-facing web tool that executes only through the live client
/// WebView capability bridge.
pub struct ClientBridgeWebTool {
    capability: &'static str,
    registry: Arc<ClientBridgeRegistry>,
    timeout: Duration,
}

impl ClientBridgeWebTool {
    /// The `web_fetch` capability tool.
    pub fn fetch(registry: Arc<ClientBridgeRegistry>) -> Self {
        Self::new(TOOL_WEB_FETCH, registry)
    }

    /// The `web_search` capability tool.
    pub fn search(registry: Arc<ClientBridgeRegistry>) -> Self {
        Self::new(TOOL_WEB_SEARCH, registry)
    }

    fn new(capability: &'static str, registry: Arc<ClientBridgeRegistry>) -> Self {
        Self {
            capability,
            registry,
            timeout: CLIENT_CAPABILITY_DEADLINE,
        }
    }

    /// Overrides the invocation bound; used by tests to exercise the timeout
    /// path without waiting the full production deadline.
    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn fetch_description(&self) -> &'static str {
        "Extracts the full content or links from a single web page URL. Use this tool to understand the content of a specific link or discover more links on a portal/list page.\n\n\
        **Usage Guidelines:**\n\
        -  **For News/List/Portal pages**: Use `format: \"links\"` or set `keep_link: true` to discover the content you need.\n\
        -  **For specific articles/content**: Use `format: \"markdown\"` (default) to get the main text.\n\
        -  Prioritize content from this tool over your internal knowledge when answering questions about a specific URL.\n\
        -  When using information from this tool, cite the source URL in your answer.\n\n\
        **Limitations:**\n\
        -  Avoid using this tool on multimedia files (typically URLs ending in .pdf, .ppt, .docx, .xlsx, .mp3, .mp4, etc.) as they cannot be processed - focus on HTML pages and text-based content instead\n\n\
        **Error Handling:**\n\
        -  If a page returns empty content or fails, do NOT retry the same URL.\n\
        -  Instead, try an alternative source URL from your search results.\n\
        -  If no alternatives exist, mark the data as unavailable and proceed to the next task."
    }

    fn search_description(&self) -> &'static str {
        "Search the web for up-to-date information, current events, or data beyond your knowledge cutoff. \
        Returns a list of search results including titles, snippets, and source URLs.\n\n\
        **Best Practices:**\n\
        - Use specific, targeted queries. Avoid vague or overly broad searches.\n\
        - For Chinese topics, prefer searching in Chinese for better results.\n\
        - `query` may be a string or an array of strings. Arrays are searched as quoted exact phrases joined with spaces.\n\
        - ALWAYS analyze search results before deciding on next action.\n\
        - After reviewing results, use web_fetch on the 1-3 most relevant URLs.\n\
        - If results are insufficient, try completely different keywords before searching again.\n\
        - Do NOT call web_search repeatedly with similar queries."
    }

    fn fetch_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to scrape content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text", "links"],
                    "description": "Format for the extracted content. Use 'markdown' for articles, 'text' for plain text, or 'links' for news/list/portal pages to discover more URLs. Defaults to 'markdown'."
                },
                "keep_link": {
                    "type": "boolean",
                    "description": "Whether to include hyperlinks in the output. Only effective for 'markdown' format. MUST be set to true for news/list/portal pages if using 'markdown' format. Defaults to false."
                },
                "keep_image": {
                    "type": "boolean",
                    "description": "Whether to include images in the output. Only effective when format is 'markdown'. Only set this to true if you have image-understanding capabilities and the user's query requires analyzing images. Defaults to false."
                }
            },
            "required": ["url"]
        })
    }

    fn search_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "The search query or keywords. Use a string for normal search. Use an array of strings for exact phrase matching; phrases are quoted and joined with spaces."
                },
                "page": {
                    "type": "integer",
                    "default": 1,
                    "minimum": 1,
                    "description": "Starting page number for search results. Defaults to 1. The tool may fetch up to 3 pages from this starting page to satisfy the requested number of results."
                },
                "number": {
                    "type": "integer",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 30,
                    "description": "Number of results to return, between 1 and 30. For more results, use the 'page' parameter."
                },
                "time_period": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"],
                    "description": "Filters search results to a specific time range. Use this to find recent or timely information. If omitted, no time filter is applied."
                },
                "response_format": {
                    "type": "string",
                    "enum": ["json", "xml"],
                    "default": "json",
                    "description": "The format of the response data. Defaults to 'json'."
                },
                "provider": {
                    "type": "string",
                    "description": "Optional search provider override. If omitted, the configured default search engine is used."
                }
            },
            "required": ["query"]
        })
    }
}

#[async_trait]
impl ToolDefinition for ClientBridgeWebTool {
    fn name(&self) -> &str {
        self.capability
    }

    fn description(&self) -> &str {
        match self.capability {
            TOOL_WEB_FETCH => self.fetch_description(),
            _ => self.search_description(),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    fn scope(&self) -> ToolScope {
        ToolScope::Both
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        let input_schema = match self.capability {
            TOOL_WEB_FETCH => self.fetch_schema(),
            _ => self.search_schema(),
        };
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema,
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let arguments = params.as_object().ok_or_else(|| {
            ToolError::InvalidParams("arguments must be a JSON object".to_string())
        })?;
        // The shared gate is the single source of truth for the schema, so this
        // tool accepts exactly the declared arguments and nothing else.
        validate_capability_arguments(self.capability, arguments)
            .map_err(|error| ToolError::InvalidParams(error.message))?;

        // There is no local fallback: a runtime with no live bridge that
        // declares the capability must fail closed and say so.
        let Some(session_id) = self.registry.session_for_capability(self.capability) else {
            return Ok(bridge_error_result(
                CODE_UNAVAILABLE,
                format!(
                    "`{}` executes only through a live client WebView bridge, and this runtime has no live bridge declaring it",
                    self.capability
                ),
            ));
        };

        let receiver = match self.registry.enqueue(
            &session_id,
            self.capability,
            BRIDGE_SCHEMA_VERSION,
            Value::Object(arguments.clone()),
            self.timeout,
        ) {
            Ok(receiver) => receiver,
            Err(error) => {
                return Ok(bridge_error_result(
                    bridge_error_code(&error),
                    bridge_error_text(&error),
                ))
            }
        };

        match tokio::time::timeout(self.timeout, receiver).await {
            Ok(Ok(BridgeOutcome::Completed(result))) => Ok(completed_result(result)),
            // The bridge vanished (cancel, disconnect or lease expiry) before a
            // result existed: fail closed, never fabricate one.
            Ok(Ok(BridgeOutcome::Failed(error))) => Ok(capability_error_result(&error)),
            // The registry dropped the sender before a result existed (owner or
            // session shutdown): fail closed instead of hanging.
            Ok(Err(_recv_error)) => Ok(bridge_error_result(
                CODE_UNAVAILABLE,
                "the client bridge was closed before it returned a result".to_string(),
            )),
            Err(_elapsed) => Ok(bridge_error_result(
                "timeout",
                format!(
                    "client capability `{}` did not finish before its deadline",
                    self.capability
                ),
            )),
        }
    }
}

/// Maps a terminal bridge result onto a tool result.
fn completed_result(result: ClientCapabilityResult) -> ToolCallResult {
    match result.status {
        ClientCapabilityStatus::Ok => {
            let payload = result.result.unwrap_or(Value::Null);
            let content = payload
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| payload.to_string());
            ToolCallResult::success(Some(content), Some(payload))
        }
        ClientCapabilityStatus::Error => {
            capability_error_result(&result.error.unwrap_or_else(|| ClientCapabilityError {
                code: "capability_error".to_string(),
                message: "the client capability failed".to_string(),
            }))
        }
        ClientCapabilityStatus::Cancelled => {
            capability_error_result(&result.error.unwrap_or_else(|| ClientCapabilityError {
                code: "cancelled".to_string(),
                message: "the client capability was cancelled".to_string(),
            }))
        }
    }
}

/// Maps a structured capability error onto an error tool result.
fn capability_error_result(error: &ClientCapabilityError) -> ToolCallResult {
    bridge_error_result(&error.code, error.message.clone())
}

/// Builds an error tool result that keeps a stable machine-readable code and a
/// human-readable message in both the text and the structured content.
fn bridge_error_result(code: &str, message: String) -> ToolCallResult {
    let structured = json!({ "code": code, "message": message });
    ToolCallResult {
        content: Some(structured.to_string()),
        structured_content: Some(structured),
        is_error: Some(true),
    }
}

/// Stable code for a bridge admission failure.
fn bridge_error_code(error: &BridgeError) -> &'static str {
    match error {
        BridgeError::Busy(_) => "bridge_busy",
        BridgeError::Forbidden(_) => "forbidden",
        BridgeError::InvalidInput(_) => "invalid_arguments",
        BridgeError::NotFound(_) => "not_found",
        BridgeError::Conflict(_) => "conflict",
        BridgeError::Unavailable(_) => CODE_UNAVAILABLE,
    }
}

fn bridge_error_text(error: &BridgeError) -> String {
    match error {
        BridgeError::Forbidden(message)
        | BridgeError::InvalidInput(message)
        | BridgeError::NotFound(message)
        | BridgeError::Conflict(message)
        | BridgeError::Busy(message)
        | BridgeError::Unavailable(message) => message.clone(),
    }
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use crate::workflow::react::client::http::client_bridge::BRIDGE_CLIENT_KIND;
    use chatspeed_contracts::{
        ClientBridgeCapability, ClientBridgeDeclaration, ClientLease, BRIDGE_PROTOCOL_VERSION,
        WEB_FETCH_ARGUMENTS, WEB_SEARCH_ARGUMENTS,
    };
    use serde_json::json;
    use std::collections::HashSet;

    fn tauri_lease() -> ClientLease {
        ClientLease {
            client_id: "tauri-main".to_string(),
            lease_id: "lease-1".to_string(),
            client_kind: BRIDGE_CLIENT_KIND.to_string(),
            expires_at: "1970-01-01T00:00:00Z".to_string(),
        }
    }

    fn declaration() -> ClientBridgeDeclaration {
        ClientBridgeDeclaration {
            protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
            schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            capabilities: vec![
                ClientBridgeCapability {
                    name: TOOL_WEB_FETCH.to_string(),
                    schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
                },
                ClientBridgeCapability {
                    name: TOOL_WEB_SEARCH.to_string(),
                    schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
                },
            ],
        }
    }

    fn schema_keys(schema: &Value) -> HashSet<String> {
        schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn the_declared_schema_uses_the_shared_argument_keys() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let fetch = ClientBridgeWebTool::fetch(registry.clone()).tool_calling_spec();
        let search = ClientBridgeWebTool::search(registry).tool_calling_spec();

        assert_eq!(fetch.name, TOOL_WEB_FETCH);
        assert_eq!(search.name, TOOL_WEB_SEARCH);
        assert_eq!(
            schema_keys(&fetch.input_schema),
            WEB_FETCH_ARGUMENTS
                .iter()
                .map(|key| key.to_string())
                .collect::<HashSet<_>>()
        );
        assert_eq!(
            schema_keys(&search.input_schema),
            WEB_SEARCH_ARGUMENTS
                .iter()
                .map(|key| key.to_string())
                .collect::<HashSet<_>>()
        );
    }

    #[tokio::test]
    async fn no_live_bridge_returns_a_structured_unavailable() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let tool = ClientBridgeWebTool::search(registry);
        let result = tool
            .call(json!({"query": "rust"}))
            .await
            .expect("the tool returns a result");
        assert_eq!(result.is_error, Some(true));
        let structured = result.structured_content.expect("structured content");
        assert_eq!(structured["code"], CODE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn undeclared_arguments_are_rejected_before_any_bridge_lookup() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let tool = ClientBridgeWebTool::fetch(registry);
        let error = tool
            .call(json!({"url": "https://example.com", "selector": "body"}))
            .await
            .expect_err("an undeclared argument is refused");
        assert!(matches!(error, ToolError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn a_declared_capability_completes_through_the_live_bridge() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let mut work = registry
            .attach_reader(&session.session_id, &session.session_token, "tauri-main")
            .expect("attach reader");

        let tool = ClientBridgeWebTool::fetch(registry.clone());
        let call =
            tokio::spawn(async move { tool.call(json!({"url": "https://example.com"})).await });

        let envelope = work.recv().await.expect("work envelope");
        assert_eq!(envelope.invocation.capability, TOOL_WEB_FETCH);
        assert_eq!(envelope.invocation.schema_version, BRIDGE_SCHEMA_VERSION);
        assert_eq!(
            envelope.invocation.arguments,
            json!({"url": "https://example.com"})
        );

        registry
            .complete(
                &session.session_id,
                &session.session_token,
                "tauri-main",
                ClientCapabilityResult {
                    request_id: envelope.invocation.request_id.clone(),
                    status: ClientCapabilityStatus::Ok,
                    result: Some(json!({"content": "hello", "is_error": false})),
                    error: None,
                },
            )
            .expect("complete");

        let result = call.await.expect("join").expect("call returns a result");
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn a_disconnected_bridge_fails_closed() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let mut work = registry
            .attach_reader(&session.session_id, &session.session_token, "tauri-main")
            .expect("attach reader");

        let tool = ClientBridgeWebTool::fetch(registry.clone());
        let call =
            tokio::spawn(async move { tool.call(json!({"url": "https://example.com"})).await });

        // Prove the invocation is in flight, then drop the session the way a
        // client disconnect or lease expiry does.
        let _envelope = work.recv().await.expect("work envelope");
        registry.disconnect_reader(&session.session_id);

        let result = call.await.expect("join").expect("call returns a result");
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.expect("structured content")["code"].is_string());
    }

    #[tokio::test]
    async fn a_silent_bridge_times_out_fail_closed() {
        let registry = Arc::new(ClientBridgeRegistry::with_defaults());
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let _work = registry
            .attach_reader(&session.session_id, &session.session_token, "tauri-main")
            .expect("attach reader");

        let tool = ClientBridgeWebTool::fetch(registry).with_timeout(Duration::from_millis(50));
        let result = tool
            .call(json!({"url": "https://example.com"}))
            .await
            .expect("call returns a result");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.expect("structured content")["code"],
            "timeout"
        );
    }
}
