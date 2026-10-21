use crate::discovery::ControlPlaneDiscovery;
use crate::error::CliError;
use chatspeed_runtime_client::{ClientError, RuntimeClient};
use serde_json::Value;

/// HTTP client bound to one authenticated runtime instance.
///
/// This is a thin adapter: it adds only the CLI error categories on top of the
/// shared runtime client, so the exit-code mapping lives in exactly one place.
pub struct ControlPlaneClient {
    inner: RuntimeClient,
}

impl ControlPlaneClient {
    /// Builds a client without contacting the runtime.
    ///
    /// Prefer [`ControlPlaneClient::connect`]: readiness must be verified before
    /// any lease is registered.
    pub fn new(discovery: &ControlPlaneDiscovery) -> Result<Self, CliError> {
        RuntimeClient::new(discovery)
            .map(|inner| Self { inner })
            .map_err(map_client_error)
    }

    /// Builds a client and completes the readiness handshake against `/meta`.
    ///
    /// Every command reaches the control plane only after this succeeds, so a
    /// stale discovery document or the desktop in-process control plane fails
    /// here instead of after a lease has been registered.
    pub async fn connect(discovery: &ControlPlaneDiscovery) -> Result<Self, CliError> {
        RuntimeClient::connect(discovery)
            .await
            .map(|inner| Self { inner })
            .map_err(map_client_error)
    }

    /// Performs a GET request and decodes the JSON response.
    pub async fn get(&self, path: &str) -> Result<Value, CliError> {
        self.inner.get(path).await.map_err(map_client_error)
    }

    /// Performs a POST mutation. When `idempotency_key` is provided, a single
    /// transport-level retry reuses the same key so the server can deduplicate.
    pub async fn post(
        &self,
        path: &str,
        body: Value,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError> {
        match idempotency_key {
            Some(key) => self
                .inner
                .post_with_idempotency(path, &body, key)
                .await
                .map_err(map_client_error),
            None => self.inner.post(path, &body).await.map_err(map_client_error),
        }
    }

    pub async fn register_lease(
        &self,
        client_id: &str,
        client_kind: &str,
    ) -> Result<chatspeed_runtime_client::LeaseGuard, CliError> {
        chatspeed_runtime_client::LeaseGuard::register(&self.inner, client_id, client_kind)
            .await
            .map_err(map_client_error)
    }

    /// Opens an SSE stream; the caller consumes raw bytes incrementally.
    pub async fn stream(
        &self,
        path: &str,
        last_event_id: Option<&str>,
    ) -> Result<reqwest::Response, CliError> {
        self.inner
            .stream(path, last_event_id)
            .await
            .map_err(map_client_error)
    }
}

/// The single mapping from shared client errors to CLI exit categories.
///
/// Budget admission codes become exit 9, auth exit 4, protocol exit 5,
/// transport exit 3 and every other structured server error stays exit 1.
pub(crate) fn map_client_error(error: ClientError) -> CliError {
    match error {
        ClientError::Discovery(message) => CliError::discovery(message),
        ClientError::Protocol(message) => CliError::protocol(message),
        ClientError::Transport(message) => CliError::transport(message),
        ClientError::Auth(auth) => {
            CliError::auth(projected_message(auth.presentation.as_ref(), &auth.message))
        }
        ClientError::Server(response) if is_budget_code(&response.code) => {
            match response.presentation {
                Some(presentation) => CliError::budget(presentation.message),
                None => CliError::budget(format!(
                    "budget admission rejected ({}): {}",
                    response.code, response.message
                )),
            }
        }
        ClientError::Server(response) => CliError::Server {
            status: response.status,
            code: response.code,
            message: projected_message(response.presentation.as_ref(), &response.message)
                .to_string(),
        },
        ClientError::InvalidRequest(message) => CliError::usage(message),
        ClientError::Serialization(message) => CliError::protocol(message),
    }
}

/// Prefers the runtime's safe presentation over the diagnostic wire message.
fn projected_message<'a>(
    presentation: Option<&'a chatspeed_contracts::UserErrorPresentation>,
    message: &'a str,
) -> &'a str {
    presentation
        .map(|presentation| presentation.message.as_str())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(message)
}

/// Whether a control-plane error code represents a budget admission rejection.
fn is_budget_code(code: &str) -> bool {
    matches!(
        code,
        "budget_exceeded"
            | "unpriced_model"
            | "missing_bound"
            | "scope_paused"
            | "resource_unobservable"
            | "invalid_scope_chain"
            | "admission_persistence_failure"
    )
}

#[cfg(test)]
mod tests {
    use super::{is_budget_code, map_client_error};
    use chatspeed_runtime_client::{AuthError, ClientError, ClientResponseError};

    fn server_error(status: u16, code: &str, message: &str) -> ClientError {
        ClientError::Server(ClientResponseError {
            legacy_display: format!("server error {status}: {message} ({code})"),
            status,
            code: code.to_string(),
            message: message.to_string(),
            presentation: None,
        })
    }

    fn presentation(message: &str) -> chatspeed_contracts::UserErrorPresentation {
        chatspeed_contracts::UserErrorPresentation {
            schema_version: 1,
            message: message.to_string(),
            reason: None,
            upstream_status: None,
        }
    }

    fn presented_server_error(
        status: u16,
        code: &str,
        message: &str,
        user_message: &str,
    ) -> ClientError {
        ClientError::Server(ClientResponseError {
            legacy_display: format!("server error {status}: {message} ({code})"),
            status,
            code: code.to_string(),
            message: message.to_string(),
            presentation: Some(presentation(user_message)),
        })
    }

    fn auth_error(
        status: u16,
        code: &str,
        message: &str,
        user_message: Option<&str>,
    ) -> ClientError {
        ClientError::Auth(AuthError {
            status,
            code: code.to_string(),
            message: message.to_string(),
            presentation: user_message.map(presentation),
        })
    }

    #[test]
    fn budget_machine_codes_are_recognized() {
        for code in [
            "budget_exceeded",
            "unpriced_model",
            "missing_bound",
            "scope_paused",
            "resource_unobservable",
            "invalid_scope_chain",
            "admission_persistence_failure",
        ] {
            assert!(is_budget_code(code), "{code} must map to exit 9");
        }
    }

    #[test]
    fn non_budget_codes_are_not_exit_nine() {
        for code in [
            "invalid_input",
            "not_found",
            "idempotency_key_conflict",
            "internal_error",
            "missing_idempotency_key",
            "unauthorized",
        ] {
            assert!(!is_budget_code(code), "{code} must not map to exit 9");
        }
    }

    #[test]
    fn shared_client_errors_keep_cli_exit_categories() {
        let auth = ClientError::Auth(AuthError {
            status: 401,
            code: "unauthorized".to_string(),
            message: "denied".to_string(),
            presentation: None,
        });
        assert_eq!(map_client_error(auth).exit_code(), 4);
        assert_eq!(
            map_client_error(ClientError::Protocol("major".to_string())).exit_code(),
            5
        );
        assert_eq!(
            map_client_error(ClientError::Transport("down".to_string())).exit_code(),
            3
        );
        assert_eq!(
            map_client_error(server_error(500, "internal_error", "boom")).exit_code(),
            1
        );
    }

    #[test]
    fn a_budget_code_maps_to_exit_nine_through_the_single_mapper() {
        assert_eq!(
            map_client_error(server_error(409, "budget_exceeded", "over budget")).exit_code(),
            9
        );
    }

    #[test]
    fn a_budget_presentation_is_the_safe_message() {
        let mapped = map_client_error(presented_server_error(
            409,
            "budget_exceeded",
            "bucket detail",
            "Ask the operator to top up the budget",
        ));
        assert_eq!(mapped.exit_code(), 9);
        match mapped {
            crate::error::CliError::Budget(message) => {
                assert_eq!(message, "Ask the operator to top up the budget");
            }
            other => panic!("expected a budget error, got {other:?}"),
        }
    }

    #[test]
    fn a_budget_without_presentation_keeps_the_machine_code() {
        let mapped = map_client_error(server_error(409, "budget_exceeded", "over budget"));
        assert_eq!(mapped.exit_code(), 9);
        assert!(mapped.to_string().contains("budget_exceeded"));
    }

    #[test]
    fn auth_errors_keep_exit_four_and_prefer_the_safe_presentation() {
        for status in [401, 403] {
            let mapped = map_client_error(auth_error(
                status,
                "unauthorized",
                "wire detail",
                Some("Sign in again"),
            ));
            assert_eq!(mapped.exit_code(), 4, "status {status}");
            match mapped {
                crate::error::CliError::Auth(message) => {
                    assert_eq!(message, "Sign in again");
                }
                other => panic!("expected an auth error, got {other:?}"),
            }
        }
    }

    #[test]
    fn auth_without_presentation_uses_the_wire_message() {
        let mapped = map_client_error(auth_error(401, "unauthorized", "denied", None));
        assert_eq!(mapped.exit_code(), 4);
        match mapped {
            crate::error::CliError::Auth(message) => assert_eq!(message, "denied"),
            other => panic!("expected an auth error, got {other:?}"),
        }
    }

    #[test]
    fn a_blank_presentation_falls_back_to_the_wire_message() {
        let mapped = map_client_error(presented_server_error(
            500,
            "internal_error",
            "wire message",
            "   ",
        ));
        assert_eq!(mapped.exit_code(), 1);
        match mapped {
            crate::error::CliError::Server { message, .. } => {
                assert_eq!(message, "wire message")
            }
            other => panic!("expected a server error, got {other:?}"),
        }
    }

    #[test]
    fn a_structured_server_error_keeps_its_status_and_code() {
        let mapped = map_client_error(server_error(503, "runtime_unavailable", "quiescing"));
        match mapped {
            crate::error::CliError::Server {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 503);
                assert_eq!(code, "runtime_unavailable");
                assert_eq!(message, "quiescing");
            }
            other => panic!("expected a server error, got {other:?}"),
        }
    }

    #[test]
    fn a_server_presentation_is_the_safe_message() {
        let mapped = map_client_error(presented_server_error(
            503,
            "runtime_unavailable",
            "legacy diagnostic at /control/v1/models",
            "The runtime is stopping; retry shortly",
        ));
        match &mapped {
            crate::error::CliError::Server {
                status,
                code,
                message,
            } => {
                assert_eq!(*status, 503);
                assert_eq!(code, "runtime_unavailable");
                assert_eq!(message, "The runtime is stopping; retry shortly");
            }
            other => panic!("expected a server error, got {other:?}"),
        }
        assert_eq!(mapped.exit_code(), 1);
        assert!(!mapped.to_string().contains("legacy diagnostic"));
        // Compatibility Display keeps classifying the typed server category.
        assert!(mapped.to_string().contains("server error 503"));
        assert!(mapped.to_string().contains("runtime_unavailable"));
    }

    #[test]
    fn unknown_codes_do_not_infer_exit_categories_from_message_keywords() {
        let mapped = map_client_error(server_error(
            400,
            "unknown_error",
            "budget_exceeded unauthorized forbidden 401 403",
        ));
        assert_eq!(mapped.exit_code(), 1);
        match mapped {
            crate::error::CliError::Server { message, .. } => {
                assert_eq!(message, "budget_exceeded unauthorized forbidden 401 403")
            }
            other => panic!("expected a server error, got {other:?}"),
        }
    }
}
