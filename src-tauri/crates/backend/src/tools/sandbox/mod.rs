// The command analyzer, executor-plan resolver and sandbox process runner are
// runtime-only: the desktop reaches sandbox execution through the runtime and
// only compiles the shared scheme DTOs plus the local device detector.
#[cfg(not(feature = "desktop"))]
pub mod analyzer;
pub mod detector;
#[cfg(not(feature = "desktop"))]
pub mod resolver;
#[cfg(not(feature = "desktop"))]
pub mod runner;
pub(crate) mod types;

pub use detector::*;
#[cfg(not(feature = "desktop"))]
pub use resolver::*;
#[cfg(not(feature = "desktop"))]
pub use runner::*;
pub use types::*;
