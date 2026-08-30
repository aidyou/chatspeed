//! Local, endpoint-bound transport policy.
//!
//! Models.dev supplies model facts only. Wire-level thinking adaptation remains
//! owned by this module and is never derived from remote provider templates.

pub use super::model_catalog::ThinkingAdapter;

/// The transport policy is intentionally resolved by the existing endpoint-bound
/// resolver. This wrapper gives future callers a stable policy boundary without
/// changing the direct/unified request paths.
pub fn resolve(
    model_id: &str,
    base_url: Option<&str>,
    backend_protocol: Option<&str>,
    metadata: Option<&std::collections::HashMap<String, String>>,
) -> Result<Option<(ThinkingAdapter, String)>, super::model_catalog::CatalogError> {
    super::model_catalog::resolve_transport(model_id, base_url, backend_protocol, metadata)
}
