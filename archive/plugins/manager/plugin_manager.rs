use dashmap::DashMap;
use rust_i18n::t;
use serde_json::Value;
use std::sync::Arc;

use crate::{
    plugins::{
        runtime::{deno::DenoRuntimeFactory, python::PythonRuntimeFactory},
        traits::{Plugin, PluginFactory, PluginInfo, PluginType, RuntimePluginInfo},
        PluginError,
    },
    DENO_RUNTIME, PYTHON_RUNTIME,
};

/// Plugin manager responsible for plugin lifecycle management
pub struct PluginManager {
    native_plugins: DashMap<String, Arc<dyn PluginFactory>>,
    runtime_plugins: DashMap<String, Arc<RuntimePluginInfo>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            native_plugins: DashMap::new(),
            runtime_plugins: DashMap::new(),
        }
    }

    /// Register a plugin factory
    pub fn register_native_plugin(
        &self,
        id: &str,
        factory: Arc<dyn PluginFactory>,
    ) -> Result<(), PluginError> {
        if self.native_plugins.contains_key(id) {
            return Err(PluginError::AlreadyExists(id.to_string()));
        }
        self.native_plugins.insert(id.to_string(), factory);
        Ok(())
    }

    /// Register a runtime plugin
    ///
    /// Runtime plugins consist of two parts:
    /// 1. The native runtime environment (e.g., Python)
    /// 2. The plugin information
    ///
    /// Since the runtime environment is shared, we only need to register
    /// the plugin information. This method handles the registration of
    /// the plugin information for runtime plugins.
    ///
    /// # Arguments
    /// * `plugin_info` - The plugin information
    ///
    /// # Returns
    /// * `Result<(), PluginError>`
    pub fn register_runtime_plugin(
        &self,
        plugin_info: RuntimePluginInfo,
    ) -> Result<(), PluginError> {
        if self.runtime_plugins.contains_key(&plugin_info.id) {
            return Err(PluginError::AlreadyExists(plugin_info.id.clone()));
        }

        // register rutime environment
        match plugin_info.plugin_type {
            PluginType::Python => {
                if !self.native_plugins.contains_key(PYTHON_RUNTIME) {
                    self.native_plugins.insert(
                        PYTHON_RUNTIME.to_string(),
                        Arc::new(PythonRuntimeFactory::new()),
                    );
                }
            }
            PluginType::JavaScript => {
                if !self.native_plugins.contains_key(DENO_RUNTIME) {
                    self.native_plugins.insert(
                        DENO_RUNTIME.to_string(),
                        Arc::new(DenoRuntimeFactory::new()),
                    );
                }
            }
            _ => {
                return Err(PluginError::PluginTypeError(
                    t!(
                        "workflow.plugin.unsupported_runtime_plugin_type",
                        plugin_type = plugin_info.plugin_type.to_string()
                    )
                    .to_string(),
                ));
            }
        }

        self.runtime_plugins
            .insert(plugin_info.id.clone(), Arc::new(plugin_info));
        Ok(())
    }

    /// Check if a plugin is available
    pub fn check_plugin_available(&self, id: &str, plugin_type: PluginType) -> bool {
        match plugin_type {
            PluginType::Native => self.native_plugins.contains_key(id),
            PluginType::Python | PluginType::JavaScript => self.runtime_plugins.contains_key(id),
        }
    }

    /// Get a runtime plugin
    pub fn get_runtime_plugin(&self, id: &str) -> Option<Arc<RuntimePluginInfo>> {
        self.runtime_plugins.get(id).map(|v| v.value().clone())
    }

    /// Get or create a plugin instance
    pub async fn get_plugin_instance(
        &self,
        id: &str,
        init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, PluginError> {
        let factory = self
            .native_plugins
            .get(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;

        factory
            .create_instance(init_options)
            .map_err(|e| PluginError::InitializationFailed(id.to_string(), e))
    }

    /// Execute a plugin with optional input
    pub async fn execute_plugin(
        &self,
        id: &str,
        plugin_type: PluginType,
        init_options: Option<&Value>,
        input: Option<Value>,
    ) -> Result<Value, PluginError> {
        if id.is_empty() {
            return Err(PluginError::InitializationFailed(
                "0".to_string(),
                t!("plugin.runtime.plugin_id_not_specified")
                    .to_string()
                    .into(),
            ));
        }
        // Create new instance for each execution
        let mut plugin = if plugin_type == PluginType::Native {
            self.get_plugin_instance(id, init_options).await?
        } else if plugin_type == PluginType::JavaScript {
            self.get_plugin_instance(DENO_RUNTIME, init_options).await?
        } else if plugin_type == PluginType::Python {
            self.get_plugin_instance(PYTHON_RUNTIME, init_options)
                .await?
        } else {
            return Err(PluginError::PluginTypeError(plugin_type.to_string()));
        };

        // Initialize with workflow context
        plugin
            .init()
            .await
            .map_err(|e| PluginError::InitializationFailed(id.to_string(), e))?;

        let plugin_info = if plugin_type == PluginType::Native {
            plugin.plugin_info().clone()
        } else {
            self.get_runtime_plugin(id)
                .as_ref()
                .map(|p| PluginInfo {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    version: p.version.clone(),
                })
                .ok_or_else(|| PluginError::NotFound(id.to_string()))?
        };

        // Execute the plugin
        let result = plugin
            .execute(input, Some(plugin_info))
            .await
            .map_err(|e| PluginError::ExecutionFailed(id.to_string(), e));

        // Cleanup
        plugin.destroy().await.map_err(|e| {
            log::warn!("Failed to destroy plugin {}: {}", id, e);
            PluginError::DestroyFailed(id.to_string(), e)
        })?;

        result
    }
}

#[cfg(test)]
mod tests {
    use crate::plugins::traits::PluginType;

    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockPlugin {
        initialized: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
        plugin_info: PluginInfo,
    }

    impl MockPlugin {
        fn new(initialized: Arc<AtomicBool>, destroyed: Arc<AtomicBool>) -> Self {
            Self {
                initialized,
                destroyed,
                plugin_info: Self::default_plugin_info(),
            }
        }

        fn default_plugin_info() -> PluginInfo {
            PluginInfo {
                id: "mock".to_string(),
                name: "Mock Plugin".to_string(),
                version: "1.0.0".to_string(),
            }
        }

        fn not_exists_plugin_info() -> PluginInfo {
            PluginInfo {
                id: "nonexistent".to_string(),
                name: "Nonexistent Plugin".to_string(),
                version: "1.0.0".to_string(),
            }
        }
    }

    struct MockPluginFactory {
        initialized: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
    }

    impl PluginFactory for MockPluginFactory {
        fn create_instance(
            &self,
            _init_options: Option<&Value>,
        ) -> Result<Box<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Box::new(MockPlugin::new(
                self.initialized.clone(),
                self.destroyed.clone(),
            )))
        }
    }

    #[async_trait]
    impl Plugin for MockPlugin {
        async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.initialized.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn destroy(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.destroyed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn plugin_info(&self) -> &PluginInfo {
            &self.plugin_info
        }

        fn plugin_type(&self) -> &PluginType {
            &PluginType::Native
        }

        async fn execute(
            &mut self,
            _input: Option<Value>,
            _plugin_info: Option<PluginInfo>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            Ok(serde_json::json!("mock result"))
        }
    }

    #[tokio::test]
    async fn test_plugin_lifecycle() {
        let manager = PluginManager::new();
        let initialized = Arc::new(AtomicBool::new(false));
        let destroyed = Arc::new(AtomicBool::new(false));

        let factory = MockPluginFactory {
            initialized: initialized.clone(),
            destroyed: destroyed.clone(),
        };

        // Register factory
        manager
            .register_native_plugin("mock", Arc::new(factory))
            .unwrap();

        // Execute plugin
        let result = manager
            .execute_plugin("mock", PluginType::Native, None, None)
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!("mock result"));

        // Verify lifecycle
        assert!(initialized.load(Ordering::SeqCst));
        assert!(destroyed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_plugin_not_found() {
        let manager = PluginManager::new();
        let result = manager
            .execute_plugin("nonexistent", PluginType::Native, None, None)
            .await;
        assert!(matches!(result, Err(PluginError::NotFound(_))));
    }
}
