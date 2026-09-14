//! CLI error type with stable exit codes.
//!
//! Exit codes (documented in `cs --help`):
//! - 0: success
//! - 1: server-side or I/O error
//! - 2: CLI usage error
//! - 3: ChatSpeed not running / discovery missing or stale / connection failure
//! - 4: authentication failure
//! - 5: incompatible control-plane protocol version

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

    /// Stable process exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Discovery(_) | CliError::Transport(_) => 3,
            CliError::Auth(_) => 4,
            CliError::Protocol(_) => 5,
            CliError::Server { .. } | CliError::Io(_) => 1,
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
            | CliError::Io(message)
            | CliError::Usage(message) => f.write_str(message),
            CliError::Server {
                status,
                code,
                message,
            } => write!(f, "server error {}: {} ({})", status, message, code),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError::io(error.to_string())
    }
}
