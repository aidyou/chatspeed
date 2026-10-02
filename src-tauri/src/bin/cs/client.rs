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
        ClientError::Auth(message) => CliError::auth(message),
        ClientError::Server { code, message, .. } if is_budget_code(&code) => {
            CliError::budget(format!("budget admission rejected ({}): {}", code, message))
        }
        ClientError::Server {
            status,
            code,
            message,
        } => CliError::Server {
            status,
            code,
            message,
        },
        ClientError::InvalidRequest(message) => CliError::usage(message),
        ClientError::Serialization(message) => CliError::protocol(message),
    }
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
    use chatspeed_runtime_client::ClientError;

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
        assert_eq!(
            map_client_error(ClientError::Auth("denied".to_string())).exit_code(),
            4
        );
        assert_eq!(
            map_client_error(ClientError::Protocol("major".to_string())).exit_code(),
            5
        );
        assert_eq!(
            map_client_error(ClientError::Transport("down".to_string())).exit_code(),
            3
        );
        assert_eq!(
            map_client_error(ClientError::Server {
                status: 500,
                code: "internal_error".to_string(),
                message: "boom".to_string(),
            })
            .exit_code(),
            1
        );
    }

    #[test]
    fn a_budget_code_maps_to_exit_nine_through_the_single_mapper() {
        assert_eq!(
            map_client_error(ClientError::Server {
                status: 409,
                code: "budget_exceeded".to_string(),
                message: "over budget".to_string(),
            })
            .exit_code(),
            9
        );
    }
}
