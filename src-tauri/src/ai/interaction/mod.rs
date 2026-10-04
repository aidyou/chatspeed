// The interaction chat state machine runs only in the desktop-free runtime; the
// constants module stays shared for the token keys the wire DTOs still use.
#[cfg(not(feature = "desktop"))]
pub mod chat_completion;
pub mod constants;
