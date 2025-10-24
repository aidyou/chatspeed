use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// HTTP module error types
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum HttpError {
    /// Request error
    #[error("{0}")]
    Request(String),
    /// Response error
    #[error("{0}")]
    Response(String),
    /// Configuration error
    #[error("{0}")]
    Config(String),
    /// IO error
    #[error("{0}")]
    Io(String),
    /// Other error
    #[error("{}", t!("http.server_startup_failed", error = _0).to_string())]
    StartUp(String),
}

impl From<std::io::Error> for HttpError {
    fn from(err: std::io::Error) -> Self {
        HttpError::Io(err.to_string())
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            HttpError::Request(t!("http.request_timeout").to_string())
        } else if err.is_connect() {
            HttpError::Request(t!("http.connection_failed", error = err.to_string()).to_string())
        } else if err.is_builder() {
            HttpError::Config(t!("http.client_build_failed", error = err.to_string()).to_string())
        } else {
            HttpError::Request(err.to_string())
        }
    }
}

pub type HttpResult<T> = Result<T, HttpError>;
