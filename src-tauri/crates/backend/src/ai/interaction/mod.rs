// The interaction chat state machine runs only in the desktop-free runtime; the
// constants it consumes are cfg-gated to the runtime as well, so the desktop
// compiles the module to nothing.
#[cfg(not(feature = "desktop"))]
pub mod chat_completion;
pub mod constants;
