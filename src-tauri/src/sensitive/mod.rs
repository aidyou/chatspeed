pub mod error;
pub mod filters;
// Localized replacement text is only needed by the runtime filter engine; the
// desktop never renders replacements and does not link the catalog lookups.
#[cfg(not(feature = "desktop"))]
pub mod i18n_support;
pub mod manager;
pub mod traits;
