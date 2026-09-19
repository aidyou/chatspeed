//! Agent Skill sub-modules: source resolution, staging, archive safety,
//! deterministic checking, installation planning and ownership.
//!
//! [`source`], [`staging`], [`archive`] and [`checker`] are the fail-closed
//! safety gate shared by install and the standalone check. [`manifest`] is the
//! ownership proof behind drift detection and uninstall. Installation planning
//! and ownership are added by the installation unit.

pub mod archive;
pub mod checker;
pub mod installer;
pub mod manifest;
pub mod orchestrator;
pub mod ownership;
pub mod plan;
pub mod source;
pub mod staging;
pub mod uninstaller;
