pub mod chat;
// Stopping an in-flight model call is a runtime-only concern; the desktop only
// consumes the wire `ChatResponse` shape from `chat`.
#[cfg(not(feature = "desktop"))]
pub mod stoppable;
