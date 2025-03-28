//! A thread-safe API key rotator for managing and retrieving API keys in a round-robin fashion.
//!
//! This module provides the ApiKeyRotator struct, which allows multiple API keys to be managed
//! for different base URLs. Keys are retrieved in a round-robin manner to distribute usage evenly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ApiKeyRotator {
    data: Arc<RwLock<HashMap<String, (Vec<String>, AtomicUsize)>>>,
}

impl ApiKeyRotator {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Retrieves the next API key for the specified base URL in a round-robin fashion.
    ///
    /// If no keys are available for the base URL, the provided `new_keys` will be added.
    ///
    /// # Arguments
    /// * `base_url` - The base URL associated with the API keys.
    /// * `new_keys` - A vector of API keys to add if no keys are available.
    ///
    /// # Returns
    /// An `Option<String>` containing the next API key, or `None` if no keys are available.
    pub async fn get_next_key(
        &self,
        base_url: &str,
        new_keys: impl IntoIterator<Item = String>,
    ) -> Option<String> {
        {
            let data_map = self.data.read().await;
            let entry = data_map.get(base_url);
            if let Some((keys, index)) = entry {
                if !keys.is_empty() {
                    let idx = index.fetch_add(1, Ordering::SeqCst);
                    return keys.get(idx % keys.len()).cloned();
                }
            }
        }

        let mut data_map = self.data.write().await;
        let (keys, index) = data_map
            .entry(base_url.to_string())
            .or_insert_with(|| (Vec::new(), AtomicUsize::new(0)));

        if keys.is_empty() {
            keys.extend(new_keys);
            if keys.is_empty() {
                return None;
            }
        }

        let idx = index.fetch_add(1, Ordering::SeqCst);
        keys.get(idx % keys.len()).cloned()
    }
}
