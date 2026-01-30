//! A thread-safe proxy rotator for managing model targets and global API key rotation.
//!
//! This module provides the ProxyRotator struct, which manages:
//! 1. Model target rotation for proxy aliases within a specific group.
//! 2. Global API key rotation across ALL providers for a proxy alias within a specific group.
//! 3. Ensures even distribution of key usage across all providers.

use dashmap::DashMap;
use lazy_static::lazy_static;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Represents a single API key with its associated provider information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalApiKey {
    pub key: String,
    pub provider_id: i64,
    pub base_url: String,
    pub model_name: String,
}

impl GlobalApiKey {
    #[cfg(test)]
    pub fn new(key: String, provider_id: i64, base_url: String, model_name: String) -> Self {
        Self {
            key,
            provider_id,
            base_url,
            model_name,
        }
    }
}

#[derive(Clone, Default)]
pub struct ProxyRotator {
    /// Counter for model target rotation per composite key.
    /// Key: Composite Key (e.g., "group_name/model-a")
    /// Value: AtomicUsize for round-robin index
    model_counters: Arc<DashMap<String, AtomicUsize>>,

    /// Global key pool for each composite key.
    /// Key: Composite Key
    /// Value: Vec<GlobalApiKey> - all keys from all providers for this composite key
    global_key_pools: Arc<DashMap<String, Vec<GlobalApiKey>>>,

    /// Global counter for key rotation per composite key.
    /// Key: Composite Key
    /// Value: AtomicUsize for round-robin index across all keys
    global_key_counters: Arc<DashMap<String, AtomicUsize>>,

    /// Mapping between provider ID and their keys for efficient update detection.
    /// Key: format!("{}:{}", composite_key, provider_id)
    /// Value: (Vec<String>, base_url, model_name) - keys and metadata for this provider
    provider_keys_mapping: Arc<DashMap<String, (Vec<String>, String, String)>>,
}

impl ProxyRotator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the next round-robin index for a specified composite key and update the counter.
    ///
    /// # Arguments
    /// * `composite_key` - The composite key ("group_name/proxy_alias").
    /// * `num_targets` - The number of backend targets configured for this proxy alias.
    ///
    /// # Returns
    /// The index of the next backend target to use. Returns 0 if `num_targets` is 0.
    pub fn get_next_target_index(&self, composite_key: &str, num_targets: usize) -> usize {
        if num_targets == 0 {
            return 0;
        }

        let counter = self
            .model_counters
            .entry(composite_key.to_string())
            .or_insert_with(|| AtomicUsize::new(0));

        let current_index = counter.fetch_add(1, Ordering::SeqCst);
        current_index % num_targets
    }

    /// Get the next API key from the global pool for a composite key.
    /// This ensures even distribution across ALL providers and ALL keys for the given group/alias.
    ///
    /// # Arguments
    /// * `composite_key` - The composite key ("group_name/proxy_alias") to get a key for.
    ///
    /// # Returns
    /// The next GlobalApiKey to use, or None if no keys are available.
    pub async fn get_next_global_key(&self, composite_key: &str) -> Option<GlobalApiKey> {
        let keys = self.global_key_pools.get(composite_key)?;

        if keys.is_empty() {
            return None;
        }

        // Get the next key using global round-robin
        let counter = self
            .global_key_counters
            .entry(composite_key.to_string())
            .or_insert_with(|| AtomicUsize::new(0));

        let current_index = counter.fetch_add(1, Ordering::SeqCst);
        let selected_key = &keys[current_index % keys.len()];

        Some(selected_key.clone())
    }

    // [Gemini] New method for atomic replacement of the key pool.
    /// Atomically replaces the entire key pool for a given composite key.
    pub async fn replace_pool_for_composite_key(
        &self,
        composite_key: &str,
        mut new_pool: Vec<GlobalApiKey>,
    ) {
        // Sort to ensure consistent order for round-robin, which is important for testing and predictability.
        new_pool.sort_by(|a, b| a.key.cmp(&b.key));

        // Atomically insert the new pool, replacing the old one.
        self.global_key_pools
            .insert(composite_key.to_string(), new_pool);

        // Since we are replacing the pool directly, we should also clear the old provider_keys_mapping
        // for this composite key to prevent stale data from being used if the old update path is ever called.
        self.provider_keys_mapping
            .retain(|key, _| !key.starts_with(&format!("{}:", composite_key)));

        #[cfg(debug_assertions)]
        log::debug!(
            "Atomically replaced key pool for composite key '{}' and cleaned up old mapping.",
            composite_key
        );
    }
}

lazy_static! {
    pub static ref CC_PROXY_ROTATOR: ProxyRotator = ProxyRotator::new();
}
