use std::sync::Arc;

use super::types::{McpClientInternal, McpServerConfig, McpStatus, StatusChangeCallback};
use rmcp::{model::InitializeRequestParam, service::RunningService, RoleClient};
use tokio::sync::RwLock;

/// Core structure holding shared state and logic for McpClient implementations.
pub struct McpClientCore {
    pub config: Arc<RwLock<McpServerConfig>>,
    pub client_instance: Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParam>>>>,
    pub status: RwLock<McpStatus>,
    pub status_callback: Arc<RwLock<Option<StatusChangeCallback>>>,
}

impl McpClientCore {
    /// Creates a new McpClientCore instance.
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            client_instance: Arc::new(RwLock::new(None)),
            status: RwLock::new(McpStatus::Stopped),
            status_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Gets the client configuration (cloned).
    pub async fn get_config(&self) -> McpServerConfig {
        self.config.read().await.clone()
    }

    /// Gets the client name from the configuration.
    pub async fn get_name(&self) -> String {
        self.config.read().await.name.clone()
    }

    /// Gets the Arc for the client instance.
    pub fn get_client_instance_arc(
        &self,
    ) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParam>>>> {
        self.client_instance.clone()
    }

    /// Gets the current status of the client.
    pub async fn get_status(&self) -> McpStatus {
        self.status.read().await.clone()
    }

    /// Sets the callback for status changes.
    pub async fn set_on_status_change_callback(&self, callback: StatusChangeCallback) {
        let mut cb = self.status_callback.write().await;
        *cb = Some(callback);
    }

    /// Updates the disabled_tools list in the internal McpServerConfig.
    pub async fn update_disabled_tools(&self, tool_name: &str, is_disabled: bool) {
        let mut config_guard = self.config.write().await;
        if is_disabled {
            let list = config_guard.disabled_tools.get_or_insert_default();
            list.insert(tool_name.to_string());
        } else {
            if let Some(list) = config_guard.disabled_tools.as_mut() {
                list.remove(tool_name);
                // If the list becomes empty, set it to None to save space/clarity
                if list.is_empty() {
                    config_guard.disabled_tools = None;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl McpClientInternal for McpClientCore {
    async fn set_status(&self, status: McpStatus) {
        *self.status.write().await = status.clone();
        self.notify_status_change(self.config.read().await.name.clone(), status)
            .await;
    }

    async fn notify_status_change(&self, name: String, status: McpStatus) {
        if let Some(callback) = self.status_callback.read().await.as_ref() {
            callback(name, status);
        }
    }
}
