//! Test for the proxy rotator to verify model target and global key rotation

#[cfg(test)]
mod tests {
    use super::super::proxy_rotator::{GlobalApiKey, ProxyRotator};
    use std::collections::HashMap;

    // [Gemini] The old `update_global_key_pool` helper is removed as tests now use the new public API directly.

    /// Test model target rotation (This test remains unchanged as the underlying method is the same)
    #[test]
    fn test_model_target_rotation() {
        let rotator = ProxyRotator::new();
        let proxy_alias = "test-models";
        let num_targets = 3;

        // Test that rotation works correctly
        let mut indices = Vec::new();
        for _ in 0..9 {
            let index = rotator.get_next_target_index(proxy_alias, num_targets);
            indices.push(index);
        }

        // Should cycle through 0, 1, 2, 0, 1, 2, 0, 1, 2
        assert_eq!(indices, vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
        println!("Model target rotation test passed!");
    }

    /// Test global API key rotation across multiple providers using the new atomic update method
    #[tokio::test]
    async fn test_global_key_rotation() {
        let rotator = ProxyRotator::new();
        let proxy_alias = "test-global-rotation";

        // Create global keys from multiple providers
        let global_keys = vec![
            GlobalApiKey::new(
                "openai-key1".to_string(),
                1,
                "https://api.openai.com".to_string(),
                "gpt-4".to_string(),
            ),
            GlobalApiKey::new(
                "claude-key1".to_string(),
                2,
                "https://api.anthropic.com".to_string(),
                "claude-3".to_string(),
            ),
            GlobalApiKey::new(
                "gemini-key1".to_string(),
                3,
                "https://api.google.com".to_string(),
                "gemini-pro".to_string(),
            ),
            GlobalApiKey::new(
                "openai-key2".to_string(),
                4,
                "https://api.openai.com".to_string(),
                "gpt-4".to_string(),
            ),
        ];

        // [Gemini] Update: Call the new atomic method once with the full list of keys.
        rotator
            .replace_pool_for_composite_key(proxy_alias, global_keys)
            .await;

        let mut selected_keys = Vec::new();
        for _ in 0..8 {
            if let Some(key) = rotator.get_next_global_key(proxy_alias).await {
                selected_keys.push(key.key);
            }
        }

        // [Gemini] Update: The new method sorts keys internally for predictable rotation.
        // The expected order is now the sorted order of the keys.
        assert_eq!(
            selected_keys,
            vec![
                "claude-key1",
                "gemini-key1",
                "openai-key1",
                "openai-key2",
                "claude-key1",
                "gemini-key1",
                "openai-key1",
                "openai-key2",
            ]
        );
        println!("Global key rotation test passed!");
    }

    /// Test empty global key pool using the new atomic update method
    #[tokio::test]
    async fn test_empty_global_pool() {
        let rotator = ProxyRotator::new();
        let proxy_alias = "empty-pool";

        // [Gemini] Update: Call the new atomic method with an empty vector.
        rotator
            .replace_pool_for_composite_key(proxy_alias, vec![])
            .await;

        // Should return None
        let key = rotator.get_next_global_key(proxy_alias).await;
        assert!(key.is_none());
        println!("Empty global pool test passed!");
    }

    /// Test single provider with multiple keys using the new atomic update method
    #[tokio::test]
    async fn test_single_provider_multiple_keys() {
        let rotator = ProxyRotator::new();
        let proxy_alias = "single-provider";

        // [Gemini] Update: Build the full Vec<GlobalApiKey> first.
        let keys = vec!["key1".to_string(), "key2".to_string(), "key3".to_string()];
        let global_keys: Vec<GlobalApiKey> = keys
            .into_iter()
            .map(|k| {
                GlobalApiKey::new(
                    k,
                    1,
                    "https://api.test.com".to_string(),
                    "model1".to_string(),
                )
            })
            .collect();

        rotator
            .replace_pool_for_composite_key(proxy_alias, global_keys)
            .await;

        let mut selected_keys = Vec::new();
        for _ in 0..6 {
            if let Some(key) = rotator.get_next_global_key(proxy_alias).await {
                selected_keys.push(key.key);
            }
        }

        assert_eq!(
            selected_keys,
            vec!["key1", "key2", "key3", "key1", "key2", "key3"]
        );
        println!("Single provider multiple keys test passed!");
    }

    /// Test balanced rotation across providers with different key counts using the new atomic update method
    #[tokio::test]
    async fn test_balanced_global_rotation() {
        let rotator = ProxyRotator::new();
        let proxy_alias = "balanced-test";

        // [Gemini] Update: Build a single, flat Vec<GlobalApiKey> with all keys.
        let mut all_keys = Vec::new();
        all_keys.extend(vec![
            GlobalApiKey::new(
                "p1-key1".to_string(),
                1,
                "https://api.p1.com".to_string(),
                "model1".to_string(),
            ),
            GlobalApiKey::new(
                "p1-key2".to_string(),
                1,
                "https://api.p1.com".to_string(),
                "model1".to_string(),
            ),
        ]);
        all_keys.push(GlobalApiKey::new(
            "p2-key1".to_string(),
            2,
            "https://api.p2.com".to_string(),
            "model2".to_string(),
        ));
        all_keys.extend(vec![
            GlobalApiKey::new(
                "p3-key1".to_string(),
                3,
                "https://api.p3.com".to_string(),
                "model3".to_string(),
            ),
            GlobalApiKey::new(
                "p3-key2".to_string(),
                3,
                "https://api.p3.com".to_string(),
                "model3".to_string(),
            ),
            GlobalApiKey::new(
                "p3-key3".to_string(),
                3,
                "https://api.p3.com".to_string(),
                "model3".to_string(),
            ),
        ]);

        rotator
            .replace_pool_for_composite_key(proxy_alias, all_keys)
            .await;

        let mut key_usage: HashMap<String, usize> = HashMap::new();

        // Perform 12 rotations (2 full cycles)
        for _ in 0..12 {
            if let Some(key) = rotator.get_next_global_key(proxy_alias).await {
                *key_usage.entry(key.key).or_insert(0) += 1;
            }
        }

        // Each key should be used exactly 2 times (12 rotations / 6 keys)
        assert_eq!(key_usage.len(), 6, "Should have 6 different keys");
        for (key, count) in &key_usage {
            assert_eq!(
                *count, 2,
                "Key {} should be used 2 times, but was used {} times",
                key, count
            );
        }

        println!("Balanced global rotation test passed!");
    }
}
