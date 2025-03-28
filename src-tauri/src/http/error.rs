use rust_i18n::t;
use thiserror::Error;

/// HTTP module error types
#[derive(Debug, Error)]
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
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
            HttpError::Request(
                t!(
                    "http.request_failed",
                    error = err.to_string(),
                    status = err.status().unwrap_or_default()
                )
                .to_string(),
            )
        }
    }
}

pub type HttpResult<T> = Result<T, HttpError>;
