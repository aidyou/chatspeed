use chatspeed_contracts::UserErrorPresentation;
use chatspeed_runtime_backend::error_diagnostics::{
    ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage,
};
use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// HTTP module error types
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum HttpError {
    /// Request error
    #[error("{0}")]
    Request(FailureMessage),
    /// Response error
    #[error("{0}")]
    Response(FailureMessage),
    /// Configuration error
    #[error("{0}")]
    Config(FailureMessage),
    /// IO error
    #[error("{0}")]
    Io(FailureMessage),
    /// Other error
    #[error("{}", t!("http.server_startup_failed", error = _0).to_string())]
    StartUp(FailureMessage),
}

impl HttpError {
    pub fn error_origin(&self) -> ErrorOrigin {
        let message = match self {
            Self::Request(message)
            | Self::Response(message)
            | Self::Config(message)
            | Self::Io(message)
            | Self::StartUp(message) => message,
        };
        message
            .origin()
            .cloned()
            .unwrap_or_else(|| ErrorOrigin::leaf(message.as_str(), None, None, None))
    }
}

impl ErrorProjection for HttpError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }
    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .diagnostic();
        diagnostic.module = "http".into();
        diagnostic
    }
}

impl From<std::io::Error> for HttpError {
    fn from(err: std::io::Error) -> Self {
        HttpError::Io(FailureMessage::from_error(&err))
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        let origin = ErrorOrigin::from_error(&err);
        if err.is_timeout() {
            HttpError::Request(
                FailureMessage::leaf(t!("http.request_timeout").to_string()).with_origin(origin),
            )
        } else if err.is_connect() {
            HttpError::Request(
                FailureMessage::leaf(
                    t!("http.connection_failed", error = err.to_string()).to_string(),
                )
                .with_origin(origin),
            )
        } else if err.is_builder() {
            HttpError::Config(
                FailureMessage::leaf(
                    t!("http.client_build_failed", error = err.to_string()).to_string(),
                )
                .with_origin(origin),
            )
        } else {
            HttpError::Request(FailureMessage::from_error(&err))
        }
    }
}

pub type HttpResult<T> = Result<T, HttpError>;
