//! Desktop-free re-export of the runtime-owned Models.dev catalog service.
//!
//! The catalog snapshot owner lives in the desktop-free runtime backend
//! (`chatspeed_runtime_backend::model_catalog_service`), which is the only
//! process that loads, serves and refreshes it. This shared module keeps the
//! canonical `ai::model_catalog_updater` path working for the runtime sources
//! while the desktop reaches the same owner over the control plane, so there is
//! exactly one catalog service implementation and no desktop copy.
//!
//! The desktop compiles this module as empty: it must not construct a second
//! catalog service.

#[cfg(not(feature = "desktop"))]
pub use crate::model_catalog_service::{proxy_type_for_store, ModelsDevCatalogService};