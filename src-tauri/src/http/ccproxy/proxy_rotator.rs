use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ProxyBackendRotator {
    // Key: Proxy Alias (e.g., "model-a")
    // Value: (Vec<BackendModelTarget>, AtomicUsize for round-robin index)
    counters: Arc<RwLock<HashMap<String, AtomicUsize>>>,
}

impl ProxyBackendRotator {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the next round-robin index for specified proxy alias and update the counter.
    ///
    /// # Arguments
    /// * `proxy_alias` - The alias of proxy model.
    /// * `num_targets` - The number of backend targets configured for this proxy alias.
    ///
    /// # Returns
    /// The index of next backend target to use. Returns 0 if `num_targets` is 0.
    pub async fn get_next_index(&self, proxy_alias: &str, num_targets: usize) -> usize {
        if num_targets == 0 {
            return 0;
        }

        let mut counters_map = self.counters.write().await;
        let counter = counters_map
            .entry(proxy_alias.to_string())
            .or_insert_with(|| AtomicUsize::new(0));

        let current_index = counter.fetch_add(1, Ordering::SeqCst);
        current_index % num_targets
    }
}

lazy_static! {
    pub static ref PROXY_BACKEND_ROTATOR: ProxyBackendRotator = ProxyBackendRotator::new();
}
