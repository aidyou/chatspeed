//! CLI error type with stable exit codes.
//!
//! Exit codes (documented in `cs --help`):
//! - 0: success
//! - 1: server-side or I/O error
//! - 2: CLI usage error
//! - 3: ChatSpeed not running / discovery missing or stale / connection failure
//! - 4: authentication failure
//! - 5: incompatible control-plane protocol version
//! - 9: budget admission rejection

use std::fmt;

#[derive(Debug)]
pub enum CliError {
    /// Discovery file missing, unreadable or stale (exit 3).
    Discovery(String),
    /// Transport-level failure talking to the control plane (exit 3).
    Transport(String),
    /// Authentication rejected (exit 4).
    Auth(String),
    /// Protocol/schema incompatibility (exit 5).
    Protocol(String),
    /// The server returned a structured error (exit 1).
    Server {
        status: u16,
        code: String,
        message: String,
    },
    /// A run was rejected by budget admission (exit 9).
    Budget(String),
    /// Local I/O or rendering failure (exit 1).
    Io(String),
    /// CLI usage error (exit 2).
    Usage(String),
}

impl CliError {
    pub fn discovery(message: impl Into<String>) -> Self {
        CliError::Discovery(message.into())
    }

    pub fn transport(message: impl Into<String>) -> Self {
        CliError::Transport(message.into())
    }

    pub fn auth(message: impl Into<String>) -> Self {
        CliError::Auth(message.into())
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        CliError::Protocol(message.into())
    }

    pub fn io(message: impl Into<String>) -> Self {
        CliError::Io(message.into())
    }

    pub fn usage(message: impl Into<String>) -> Self {
        CliError::Usage(message.into())
    }

    pub fn budget(message: impl Into<String>) -> Self {
        CliError::Budget(message.into())
    }

    /// Safe one-line text for terminal output.
    ///
    /// `map_client_error` materializes the runtime's optional presentation into
    /// the message field, so this never returns the shared client's classifying
    /// display (`server error ...`, `authentication failed: ...`).
    pub fn user_message(&self) -> &str {
        match self {
            CliError::Discovery(message)
            | CliError::Transport(message)
            | CliError::Auth(message)
            | CliError::Protocol(message)
            | CliError::Budget(message)
            | CliError::Io(message)
            | CliError::Usage(message)
            | CliError::Server { message, .. } => message,
        }
    }

    /// Stable process exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Discovery(_) | CliError::Transport(_) => 3,
            CliError::Auth(_) => 4,
            CliError::Protocol(_) => 5,
            CliError::Server { .. } | CliError::Io(_) => 1,
            CliError::Budget(_) => 9,
            CliError::Usage(_) => 2,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Discovery(message)
            | CliError::Transport(message)
            | CliError::Auth(message)
            | CliError::Protocol(message)
            | CliError::Budget(message)
            | CliError::Io(message)
            | CliError::Usage(message) => f.write_str(message),
            CliError::Server {
                status,
                code,
                message,
            } => {
                // Local callers may construct a server category without text;
                // keep the typed status/code visible instead of printing an
                // empty line. Normal wire errors always carry user-safe text.
                if message.is_empty() {
                    write!(f, "server error {} ({})", status, code)
                } else {
                    f.write_str(message)
                }
            }
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError::io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_rejection_exits_nine() {
        assert_eq!(CliError::budget("rejected").exit_code(), 9);
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::usage("bad").exit_code(), 2);
        assert_eq!(CliError::auth("no").exit_code(), 4);
        assert_eq!(CliError::protocol("stale").exit_code(), 5);
        assert_eq!(CliError::transport("down").exit_code(), 3);
        assert_eq!(
            CliError::Server {
                status: 500,
                code: "internal_error".into(),
                message: "boom".into()
            }
            .exit_code(),
            1
        );
    }

    #[test]
    fn budget_message_is_protocol_safe() {
        let error = CliError::budget("budget admission rejected (budget_exceeded)");
        assert!(error.to_string().contains("budget_exceeded"));
    }

    #[test]
    fn server_display_uses_the_safe_message() {
        let error = CliError::Server {
            status: 503,
            code: "runtime_unavailable".into(),
            message: "The runtime is stopping; retry shortly".into(),
        };
        assert_eq!(
            error.user_message(),
            "The runtime is stopping; retry shortly"
        );
        assert_eq!(error.to_string(), error.user_message());
        assert!(!error.to_string().contains("server error"));
        assert!(!error.to_string().contains("runtime_unavailable"));
    }

    #[test]
    fn empty_server_message_still_exposes_typed_status_and_code() {
        let error = CliError::Server {
            status: 502,
            code: "gateway_unavailable".into(),
            message: String::new(),
        };
        assert_eq!(error.to_string(), "server error 502 (gateway_unavailable)");
    }
}
