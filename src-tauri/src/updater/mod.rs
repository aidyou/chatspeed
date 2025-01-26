//! Update system module
//!
//! Provides functionality for checking and installing application updates.
//! Built on top of Tauri's update system with additional support for
//! multi-source downloads and speed testing.

mod error;
mod manager;
mod types;

pub use error::*;
pub use manager::UpdateManager;
pub use types::*;
