//! Type definitions for the update system
//!
//! Defines the core data structures used in the update process.

use serde::{Deserialize, Serialize};

/// Version information returned by the update server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Version number in semver format
    pub version: String,

    /// Update description and release notes
    pub notes: String,
}
