mod common;
mod proxy_rotator;
#[cfg(not(feature = "desktop"))]
pub mod retry;
#[cfg(not(feature = "desktop"))]
pub mod sse;
#[cfg(not(feature = "desktop"))]
pub mod stat_guard;
#[cfg(not(feature = "desktop"))]
pub mod stream_handler;
mod stream_processor;
pub mod thinking;
pub mod tool_use_xml;

pub use common::*;
pub use proxy_rotator::CC_PROXY_ROTATOR;
#[cfg(not(feature = "desktop"))]
pub use retry::{send_with_retry, RetryConfig};
#[cfg(not(feature = "desktop"))]
pub use sse::Event;
pub use stream_processor::StreamProcessor;

#[cfg(test)]
mod proxy_rotator_test;
