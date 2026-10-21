//! Internal error diagnostics declarations and re-exports.
mod logging;
mod projection;
mod traits;
mod types;

pub use logging::log_error_diagnostic;
pub use projection::{project_json_message, project_user_message, sanitize_user_message};
pub use traits::ErrorProjection;
#[cfg(not(feature = "desktop"))]
pub(crate) use types::ReconnectDisposition;
pub use types::{ErrorDiagnostic, ErrorFrame, ErrorOrigin, FailureMessage};
