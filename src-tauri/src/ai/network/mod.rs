#[cfg(not(feature = "desktop"))]
mod client;
#[cfg(not(feature = "desktop"))]
mod stream;
pub mod types;

#[cfg(not(feature = "desktop"))]
pub use client::{ApiClient, DefaultApiClient};
#[cfg(not(feature = "desktop"))]
pub use stream::{StreamChunk, TokenUsage};
#[cfg(not(feature = "desktop"))]
pub use types::*;
