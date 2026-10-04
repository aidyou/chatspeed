// The chat execution clients and the local transport policy run only in the
// desktop-free runtime; the desktop reaches them through the control plane.
#[cfg(not(feature = "desktop"))]
pub mod chat;
pub mod error;
pub mod interaction;
pub mod model_catalog;
pub mod model_catalog_updater;
pub mod network;
pub mod traits;
#[cfg(not(feature = "desktop"))]
pub mod transport;
pub mod util;
