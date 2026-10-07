//! Windowless chat stream registry for the runtime process.
//!
//! The desktop implementation of this module pushes batched chat output into
//! Tauri windows. A runtime process owns no window, so it exposes the same
//! registry shape — `new` plus `get_sender` — and lets a control-plane route
//! register the one stream a chat turn needs.
//!
//! The canonical chat dispatcher resolves the destination through
//! `get_sender(window_label)`; registering a channel here is therefore the only
//! way a runtime session can observe the dispatcher's UI-bound chunks (including
//! the tool-call round trips), without forking the dispatcher or feeding the chat
//! callback directly (which would bypass tool execution).
//!
//! Registrations are per window label and replace any previous one: the desktop
//! registry also keys by window label, so a runtime and a desktop client agree on
//! the routing key. The owning turn keeps the [`ChannelHandle`] and closes it when
//! the turn ends, so an aborted or cancelled turn never leaves a dead channel
//! behind for the dispatcher to write into.

use crate::ai::traits::chat::ChatResponse;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Bounded capacity of a registered chat stream. The dispatcher uses `try_send`
/// and logs (never blocks) when a slow observer falls behind.
const CHANNEL_CAPACITY: usize = 256;

/// The registry of per-window chat stream senders.
///
/// A runtime process registers exactly the channels its active chat turns need,
/// so a lookup for a label with no turn reports "no window".
pub struct WindowChannels {
    channels: Arc<Mutex<HashMap<String, RegisteredChannel>>>,
    next_token: AtomicU64,
}

struct RegisteredChannel {
    token: u64,
    sender: mpsc::Sender<Arc<ChatResponse>>,
}

/// A registered chat stream: the receiver the forwarder drains plus the handle
/// that unregisters it.
pub struct ChannelRegistration {
    /// Chunks the canonical dispatcher routed to this label.
    pub receiver: mpsc::Receiver<Arc<ChatResponse>>,
    /// Removes the registration when the owning turn ends.
    pub handle: ChannelHandle,
}

/// Removes one registration, guarding against clobbering a newer one.
pub struct ChannelHandle {
    channels: Arc<Mutex<HashMap<String, RegisteredChannel>>>,
    label: String,
    token: u64,
}

impl ChannelHandle {
    /// Removes the registration if it is still the one this handle created.
    ///
    /// A newer turn for the same label installs a new token, so a late close
    /// from an earlier turn cannot drop the newer turn's stream.
    pub async fn close(self) {
        let mut channels = self.channels.lock().await;
        if channels
            .get(&self.label)
            .is_some_and(|entry| entry.token == self.token)
        {
            channels.remove(&self.label);
        }
    }
}

impl WindowChannels {
    /// Creates the empty registry.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
            next_token: AtomicU64::new(1),
        }
    }

    /// Returns the sender the dispatcher should route `window_label` chunks to.
    pub async fn get_sender(&self, window_label: &str) -> Option<mpsc::Sender<Arc<ChatResponse>>> {
        self.channels
            .lock()
            .await
            .get(window_label)
            .map(|entry| entry.sender.clone())
    }

    /// Registers (replacing any previous) a chat stream for `window_label`.
    pub async fn register_channel(&self, window_label: &str) -> ChannelRegistration {
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        self.channels.lock().await.insert(
            window_label.to_string(),
            RegisteredChannel { token, sender },
        );
        ChannelRegistration {
            receiver,
            handle: ChannelHandle {
                channels: self.channels.clone(),
                label: window_label.to_string(),
                token,
            },
        }
    }
}

impl Default for WindowChannels {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::traits::chat::{ChatResponse, MessageType};

    fn chunk(chat_id: &str, text: &str) -> Arc<ChatResponse> {
        ChatResponse::new_with_arc(
            chat_id.to_string(),
            text.to_string(),
            MessageType::Text,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn an_unregistered_label_has_no_sender() {
        let channels = WindowChannels::new();
        assert!(channels.get_sender("main").await.is_none());
    }

    #[tokio::test]
    async fn a_registered_channel_reaches_the_dispatcher_and_the_forwarder() {
        let channels = WindowChannels::new();
        let mut registration = channels.register_channel("main").await;

        let sender = channels
            .get_sender("main")
            .await
            .expect("registered sender");
        sender.try_send(chunk("chat-1", "hello")).expect("send");

        let received = registration
            .receiver
            .recv()
            .await
            .expect("forwarder receives the chunk");
        assert_eq!(received.chunk, "hello");

        registration.handle.close().await;
        assert!(channels.get_sender("main").await.is_none());
    }

    #[tokio::test]
    async fn closing_an_old_registration_does_not_drop_a_newer_turn() {
        let channels = WindowChannels::new();
        let old = channels.register_channel("main").await;
        let _new = channels.register_channel("main").await;

        // The old turn ends after the new one started: the newer stream survives.
        old.handle.close().await;
        assert!(channels.get_sender("main").await.is_some());

        // The replaced receiver is closed, so the old forwarder cannot observe
        // the newer turn's chunks.
        let mut old_receiver = old.receiver;
        assert!(old_receiver.recv().await.is_none());
    }
}
