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
use tokio::sync::RwLock;

/// Represents a single API key with its associated provider information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalApiKey {
    pub key: String,
    pub provider_id: i64,
    pub base_url: String,
    pub model_name: String,
}

impl GlobalApiKey {
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
    global_key_pools: Arc<RwLock<DashMap<String, Vec<GlobalApiKey>>>>,

    /// Global counter for key rotation per composite key.
    /// Key: Composite Key
    /// Value: AtomicUsize for round-robin index across all keys
    global_key_counters: Arc<DashMap<String, AtomicUsize>>,

    /// Mapping between provider ID and their keys for efficient update detection.
    /// Key: format!("{}:{}", composite_key, provider_id)
    /// Value: (Vec<String>, base_url, model_name) - keys and metadata for this provider
    provider_keys_mapping: Arc<RwLock<DashMap<String, (Vec<String>, String, String)>>>,
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

    /// Update keys for a specific provider with efficient change detection.
    /// Only updates the global pool if the keys actually changed.
    ///
    /// # Arguments
    /// * `composite_key` - The composite key ("group_name/proxy_alias").
    /// * `provider_id` - The provider ID
    /// * `base_url` - The provider's base URL
    /// * `model_name` - The model name
    /// * `new_keys` - The new keys for this provider (Vec<String>)
    pub async fn update_provider_keys_efficient(
        &self,
        composite_key: &str,
        provider_id: i64,
        base_url: &str,
        model_name: &str,
        new_keys: Vec<String>,
    ) {
        // 1. Exit directly if provider_id is empty (provider_id of 0 is considered invalid)
        if provider_id == 0 {
            log::debug!(
                "Provider ID is 0, skipping update for composite key '{}'",
                composite_key
            );
            return;
        }

        let mapping_key = format!("{}:{}", composite_key, provider_id);

        // Check if mapping relationship exists and if there are changes
        let needs_update = {
            let mapping = self.provider_keys_mapping.read().await;
            // Clone the necessary data inside the lock's scope to avoid lifetime issues.
            let existing_entry_opt = mapping.get(&mapping_key).map(|entry| {
                let (keys, base_url, model_name) = entry.value();
                (keys.clone(), base_url.clone(), model_name.clone())
            });
            // The read lock is released here as `mapping` goes out of scope.

            // Now, perform comparisons on the owned data.
            match existing_entry_opt {
                None => !new_keys.is_empty(), // If no mapping exists, update if there are new keys.
                Some((existing_keys, existing_base_url, existing_model_name)) => {
                    if new_keys.is_empty() {
                        true // Need to delete the existing entry.
                    } else {
                        // Sort keys for consistent comparison.
                        let mut existing_sorted = existing_keys;
                        existing_sorted.sort();
                        let mut new_sorted = new_keys.clone();
                        new_sorted.sort();

                        // Check for any changes in keys, base_url, or model_name.
                        existing_sorted != new_sorted
                            || existing_base_url != base_url
                            || existing_model_name != model_name
                    }
                }
            }
        };

        // 3. If exists but no update, exit directly
        if !needs_update {
            #[cfg(debug_assertions)]
            log::debug!(
                "No key changes for provider {} in composite key '{}', skipping update",
                provider_id,
                composite_key
            );
            return;
        }

        // Update mapping relationship
        {
            let mapping = self.provider_keys_mapping.write().await;
            if new_keys.is_empty() {
                // 5. If keys are empty, remove the mapping between id and keys
                mapping.remove(&mapping_key);
                #[cfg(debug_assertions)]
                log::debug!(
                    "Removed mapping for provider {} in composite key '{}'",
                    provider_id,
                    composite_key
                );
            } else {
                // 2. Write id to key mapping relationship or 4. Modify mapping to latest
                let mut sorted_keys = new_keys.clone();
                sorted_keys.sort(); // Keep keys order consistent for round-robin
                mapping.insert(
                    mapping_key.clone(),
                    (sorted_keys, base_url.to_string(), model_name.to_string()),
                );
                #[cfg(debug_assertions)]
                log::debug!(
                    "Updated mapping for provider {} in composite key '{}': {} keys",
                    provider_id,
                    composite_key,
                    new_keys.len()
                );
            }
        }

        // 6. Read all current keys from id-keys mapping and write to global keys
        self.rebuild_global_keys_from_mapping(composite_key).await;
    }

    /// Rebuild the global key pool from the provider keys mapping for a specific composite key.
    async fn rebuild_global_keys_from_mapping(&self, composite_key: &str) {
        let mapping = self.provider_keys_mapping.read().await;
        let mut global_keys = Vec::new();

        // Collect all provider keys belonging to this composite_key
        for entry in mapping.iter() {
            let key = entry.key();
            if key.starts_with(&format!("{}:", composite_key)) {
                // Parse provider_id
                if let Some(provider_id_str) = key.strip_prefix(&format!("{}:", composite_key)) {
                    if let Ok(provider_id) = provider_id_str.parse::<i64>() {
                        let (keys, base_url, model_name) = entry.value();

                        // Create GlobalApiKey for each key
                        for key_str in keys {
                            global_keys.push(GlobalApiKey::new(
                                key_str.clone(),
                                provider_id,
                                base_url.clone(),
                                model_name.clone(),
                            ));
                        }
                    }
                }
            }
        }

        // Sort to ensure consistent order
        global_keys.sort_by(|a, b| a.key.cmp(&b.key));

        // Update global key pool
        {
            let pools = self.global_key_pools.write().await;
            pools.insert(composite_key.to_string(), global_keys.clone());
        }

        #[cfg(debug_assertions)]
        log::debug!(
            "Rebuilt global key pool for composite key '{}': {} keys from mapping",
            composite_key,
            global_keys.len()
        );
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
        let pools = self.global_key_pools.read().await;
        let keys = pools.get(composite_key)?;

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
}

lazy_static! {
    pub static ref CC_PROXY_ROTATOR: ProxyRotator = ProxyRotator::new();
}
