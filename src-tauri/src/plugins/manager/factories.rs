use std::sync::{Arc, Mutex};

use tauri::Manager;

use crate::{
    db::MainStore,
    plugins::{
        core::{
            http::HttpPluginFactory, selector::SelectorPluginFactory, store::StorePluginFactory,
        },
        manager::PluginManager,
        runtime::{deno::DenoRuntimeFactory, python::PythonRuntimeFactory},
        traits::RuntimePluginInfo,
        PluginError,
    },
    CORE_PLUGIN_HTTP_CLIENT, CORE_PLUGIN_SELECTOR, CORE_PLUGIN_STORE, DENO_RUNTIME, PYTHON_RUNTIME,
};

impl PluginManager {
    /// Register core plugin factories for the application
    ///
    /// This includes:
    /// - HTTP client plugin for making HTTP requests
    /// - Store plugin for data persistence
    /// - Selector plugin for text selection
    /// - Runtime plugins (Python and Deno) for executing plugin code
    ///
    /// # Errors
    ///
    /// Returns `PluginError` if any plugin registration fails
    pub fn register_core_factories(&self) -> Result<(), PluginError> {
        // Register HTTP plugin factory
        self.register_native_plugin(CORE_PLUGIN_HTTP_CLIENT, Arc::new(HttpPluginFactory))?;

        // Register Store plugin factory
        self.register_native_plugin(CORE_PLUGIN_STORE, Arc::new(StorePluginFactory))?;

        // Register Selector plugin factory
        self.register_native_plugin(CORE_PLUGIN_SELECTOR, Arc::new(SelectorPluginFactory))?;

        // Register Python runtime plugin factory
        self.register_native_plugin(PYTHON_RUNTIME, Arc::new(PythonRuntimeFactory))?;

        // Register Deno runtime plugin factories
        self.register_native_plugin(DENO_RUNTIME, Arc::new(DenoRuntimeFactory))?;

        Ok(())
    }

    /// register all available runtime plugins
    fn register_runtime_factories(&self, app_handle: &tauri::AppHandle) -> Result<(), PluginError> {
        let store = app_handle.state::<Arc<Mutex<MainStore>>>();
        if let Ok(store) = store.clone().lock() {
            if let Ok(plugins) = store.get_plugin_list() {
                for plugin in plugins {
                    self.register_runtime_plugin(RuntimePluginInfo {
                        id: plugin.uuid.clone(),
                        name: plugin.name.clone(),
                        version: plugin.version.clone(),
                        plugin_type: plugin.runtime_type.into(),
                    })?;
                }
            }
        }
        Ok(())
    }
}
