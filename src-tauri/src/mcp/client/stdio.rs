//! A client implementation for MCP (Model Context Protocol) using Stdio transport
//!
//! # Type Parameters
//! - `T`: A type that can be converted to an OS string reference and cloned
//!
//! # Examples
//! ```no_run
//! use crate::mcp::client::{StdioClient, McpServerConfig, McpProtocolType};
//!
//! let config = McpServerConfig {
//!     protocol_type: McpProtocolType::Stdio,
//!     command: Some("upx".into()),
//!     args: Some(vec!["cstoolbox"]),
//!     env: None,
//!     url: None,
//! };
//!
//! let mut client = StdioClient::new(config)?;
//! client.start().await?;
//! let tools = client.list_tools().await?;
//! println!("{:?}", tools);
//!
//! let result = client.call("web_crawler", serde_json::json!({ "url": "https://github.com/aidyou/chatspeed" })).await?;
//! println!("{}", result);

//! client.stop().await?;
//! Ok(())
//! ```
//!

use std::sync::Arc;

use rmcp::{service::RunningService, transport::TokioChildProcess, RoleClient, ServiceExt as _};
use rust_i18n::t;
use tokio::{process::Command, sync::RwLock};

use super::{
    types::McpStatus, McpClient, McpClientError, McpClientResult, McpProtocolType, McpServerConfig,
};

/// A client implementation for MCP (Model Context Protocol) using Stdio transport
pub struct StdioClient {
    /// Configuration for the MCP server
    config: McpServerConfig,
    /// Running service instance
    client: Arc<RwLock<Option<RunningService<RoleClient, ()>>>>,

    status: RwLock<McpStatus>,
}

impl StdioClient {
    /// Creates a new StdioClient instance
    ///
    /// # Arguments
    /// * `config` - Configuration for the MCP server
    ///
    /// # Returns
    /// A new StdioClient instance or an error if:
    /// - Protocol type is not Stdio
    /// - Command or args are missing
    /// - Args is empty
    pub fn new(config: McpServerConfig) -> McpClientResult<Self> {
        if config.command.as_deref().unwrap_or_default().is_empty() {
            return Err(McpClientError::ConfigError(
                t!("mcp.client.stdio_command_cant_be_empty").to_string(),
            ));
        }
        if config.protocol_type != McpProtocolType::Stdio {
            return Err(McpClientError::ConfigError(
                t!(
                    "mcp.client.config_mismatch",
                    client = "StdioClient",
                    protocol_type = config.protocol_type.to_string()
                )
                .to_string(),
            ));
        }
        if config.args.as_ref().map(Vec::is_empty).unwrap_or(true) {
            return Err(McpClientError::ConfigError(
                t!("mcp.client.stdio_args_cant_be_empty").to_string(),
            ));
        }
        Ok(StdioClient {
            config,
            client: Arc::new(RwLock::new(None)),
            status: RwLock::new(McpStatus::Stopped),
        })
    }
}

/// Implementation of McpClient trait for Stdio transport
#[async_trait::async_trait]
impl McpClient for StdioClient {
    async fn status(&self) -> McpStatus {
        self.status.read().await.clone()
    }

    async fn set_status(&self, status: McpStatus) {
        let mut s = self.status.write().await;
        *s = status;
    }

    /// Performs the actual Stdio process starting logic.
    /// This method is called by the default `start` implementation in the `McpClient` trait.
    ///
    /// # Returns
    /// `McpClientResult<RunningService<RoleClient, ()>>` - The running service instance
    /// on success, or an error.
    async fn perform_connect(&self) -> McpClientResult<RunningService<RoleClient, ()>> {
        let cmd_str = self
            .config
            .command
            .as_ref()
            .ok_or(McpClientError::ConfigError(
                t!("mcp.client.stdio_command_cant_be_empty").to_string(),
            ))?;
        let mut cmd = Command::new(cmd_str.clone());

        let args = self
            .config
            .args
            .as_ref()
            .ok_or_else(|| {
                McpClientError::ConfigError(t!("mcp.client.stdio_args_cant_be_empty").into())
            })?
            .iter()
            .filter_map(|s| {
                let s = s.trim();
                (!s.is_empty()).then_some(s)
            });
        cmd.args(args);

        if let Some(env) = &self.config.env {
            cmd.envs(env.iter().filter_map(|(k, v)| {
                let k = k.trim();
                let v = v.trim();
                (!k.is_empty() && !v.is_empty()).then_some((k, v))
            }));
        }

        let process = TokioChildProcess::new(&mut cmd).map_err(|e| {
            return McpClientError::StartError(e.to_string());
        })?;

        ().serve(process).await.map_err(|e| {
            log::error!("Start StdioClient error: {}", e);
            McpClientError::StartError(e.to_string())
        })
    }

    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, ()>>>> {
        self.client.clone()
    }

    fn name(&self) -> String {
        self.config.name.clone()
    }

    fn config(&self) -> McpServerConfig {
        self.config.clone()
    }
}

mod test {
    use crate::mcp::client::{McpClient as _, McpClientError};

    #[tokio::test]
    async fn stdio_test() -> Result<(), McpClientError> {
        let config = crate::mcp::client::McpServerConfig {
            protocol_type: crate::mcp::client::McpProtocolType::Stdio,
            command: Some("uvx".into()),
            args: Some(vec![
                "-i".into(),
                "https://mirrors.aliyun.com/pypi/simple/".into(),
                "cstoolbox@1.0.5".into(),
            ]),
            env: Some(vec![
                ("CS_HEADLESS".into(), "true".into()),
                // ("CS_PROXY", "http://localhost:15154"),
                (
                    "CS_EXECUTABLE_PATH".into(),
                    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
                ),
                (
                    "CS_USER_DATA_DIR".into(),
                    "~/Library/Application Support/Google/Chrome".into(),
                ),
            ]),
            ..Default::default()
        };
        let client = crate::mcp::client::stdio::StdioClient::new(config)
            .expect("Should create client with valid config");
        client.start().await?;
        let tools = client.list_tools().await?;
        log::info!("{}", serde_json::to_string_pretty(&tools).unwrap());

        let result = client
            .call(
                "web_crawler",
                serde_json::json!({ "url": "https://github.com/aidyou/chatspeed" }),
            )
            .await?;
        log::info!("{}", result);

        client.stop().await?;
        Ok(())
    }
}
