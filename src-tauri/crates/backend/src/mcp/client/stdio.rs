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

use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, InitializeRequestParams};
use rmcp::{service::RunningService, transport::TokioChildProcess, RoleClient, ServiceExt as _};
use rust_i18n::t;
use tokio::{process::Command, sync::RwLock};

#[allow(unused)]
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::mcp::client::types::McpClientInternal;
use crate::mcp::client::util::find_executable_in_common_paths;
use crate::mcp::McpError;

use super::core::McpClientCore;
use super::{
    McpClient, McpClientResult, McpProtocolType, McpServerConfig, McpStatus, StatusChangeCallback,
};

/// A client implementation for MCP (Model Context Protocol) using Stdio transport
pub struct StdioClient {
    core: McpClientCore,
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
            return Err(McpError::ClientConfigError(
                t!("mcp.client.stdio_command_cant_be_empty").to_string(),
            ));
        }
        if config.protocol_type != McpProtocolType::Stdio {
            return Err(McpError::ClientConfigError(
                t!(
                    "mcp.client.config_mismatch",
                    client = "StdioClient",
                    protocol_type = config.protocol_type.to_string()
                )
                .to_string(),
            ));
        }
        if config.args.as_ref().map(Vec::is_empty).unwrap_or(true) {
            return Err(McpError::ClientConfigError(
                t!("mcp.client.stdio_args_cant_be_empty").to_string(),
            ));
        }
        #[cfg(debug_assertions)]
        {
            log::debug!(
                "MCP original config: {}",
                serde_json::to_string_pretty(&config).unwrap_or_default(),
            );
        }
        Ok(StdioClient {
            // Pass the validated config to McpClientCore
            core: McpClientCore::new(config),
        })
    }

    // Only the runtime test helper `ToolManager::register_test_mcp_tool` drives
    // this hook, and that helper is itself runtime-only.
    #[cfg(all(test, not(feature = "desktop")))]
    pub(crate) async fn set_test_status(&self, status: McpStatus) {
        self.core.set_status(status).await;
    }
}

#[async_trait::async_trait]
impl McpClientInternal for StdioClient {
    async fn set_status(&self, status: McpStatus) {
        self.core.set_status(status).await;
    }

    async fn notify_status_change(&self, name: String, status: McpStatus) {
        self.core.notify_status_change(name, status).await;
    }
}

/// Implementation of McpClient trait for Stdio transport
#[async_trait::async_trait]
impl McpClient for StdioClient {
    /// Performs the actual Stdio process starting logic.
    /// This method is called by the default `start` implementation in the `McpClient` trait.
    ///
    /// # Returns
    /// `McpClientResult<RunningService<RoleClient, ()>>` - The running service instance
    /// on success, or an error.
    async fn perform_connect(
        &self,
    ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParams>> {
        let config = self.core.get_config().await;
        let original_cmd_str = config
            .command
            .as_ref()
            .cloned() // Clone the Option<String>
            .ok_or_else(|| {
                McpError::ClientConfigError(
                    // Use ok_or_else for lazy evaluation
                    t!("mcp.client.stdio_command_cant_be_empty").to_string(),
                )
            })?;

        // Try to find the executable, fallback to original_cmd_str if not found by helper
        let executable_to_run =
            if let Some(abs_path) = find_executable_in_common_paths(&original_cmd_str).await {
                log::info!(
                    "Found executable for command {}: {}",
                    original_cmd_str,
                    abs_path.display()
                );

                abs_path.to_string_lossy().into_owned()
            } else {
                log::info!(
                    "Can't find executable for command {}, falling back to original command string",
                    original_cmd_str
                );

                original_cmd_str.clone() // Fallback to original command string (relies on system PATH)
            };

        log::info!("Starting StdioClient with command: {}", executable_to_run);

        #[cfg(windows)]
        let mut cmd = {
            let path = std::path::Path::new(&executable_to_run);
            let extension = path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("");
            let is_script = extension.eq_ignore_ascii_case("cmd")
                || extension.eq_ignore_ascii_case("bat")
                || path.file_name().and_then(std::ffi::OsStr::to_str) == Some("npx")
                || path.file_name().and_then(std::ffi::OsStr::to_str) == Some("uv")
                || path.file_name().and_then(std::ffi::OsStr::to_str) == Some("uvx")
                || original_cmd_str == "npx"
                || original_cmd_str == "uv"
                || original_cmd_str == "uvx";

            let mut cmd = if is_script {
                let mut c = Command::new("cmd");
                c.arg("/c").arg(&executable_to_run);
                c
            } else {
                Command::new(&executable_to_run)
            };
            // Set CREATE_NO_WINDOW to hide the console window when starting the process.
            // Using only 0x08000000 (CREATE_NO_WINDOW) is usually sufficient and safer than DETACHED_PROCESS.
            cmd.creation_flags(0x08000000);
            cmd
        };

        #[cfg(not(windows))]
        let mut cmd = Command::new(&executable_to_run);

        let args = config
            .args
            .as_ref()
            .ok_or_else(|| {
                McpError::ClientConfigError(t!("mcp.client.stdio_args_cant_be_empty").into())
            })?
            .iter()
            .filter_map(|s| {
                let s = s.trim();
                (!s.is_empty()).then_some(s)
            });
        cmd.args(args);

        if let Some(env) = config.env {
            cmd.envs(env.iter().filter_map(|(k, v)| {
                let k = k.trim();
                let v = v.trim();
                (!k.is_empty() && !v.is_empty()).then_some((k, v))
            }));
        }

        // On Unix, create a new process group to prevent signals from propagating to the parent.
        // This is crucial to prevent the main application from crashing when the child process is terminated.
        #[cfg(unix)]
        cmd.process_group(0);

        let process = TokioChildProcess::new(cmd).map_err(|e| {
            let original_error_message = e.to_string();
            // Use original_cmd_str for user-facing messages about the command they configured
            let display_command_name = &original_cmd_str;

            if e.kind() == std::io::ErrorKind::NotFound {
                let specific_help = match display_command_name.as_str() {
                    "npx" => t!("mcp.client.stdio_npx_not_found_help").to_string(),
                    "uvx" | "uv" => t!(
                        "mcp.client.stdio_uvx_not_found_help",
                        command = display_command_name
                    )
                    .to_string(),
                    _ => "".to_string(),
                };
                log::error!(
                    "Command {} (tried as {}) not found: {}. OS error: {}",
                    display_command_name,
                    executable_to_run,
                    e.kind(),
                    original_error_message
                );
                let error_message = if !specific_help.is_empty() {
                    format!(
                        "{} {}",
                        t!(
                            "mcp.client.stdio_command_not_found_with_help",
                            command = original_cmd_str,
                            original_error = original_error_message // Keep original OS error for context
                        ),
                        specific_help
                    )
                } else {
                    t!(
                        "mcp.client.stdio_command_not_found_no_help",
                        command = original_cmd_str,
                        original_error = original_error_message // Keep original OS error for context
                    )
                    .to_string()
                };
                McpError::ClientStartError(error_message)
            } else {
                McpError::ClientStartError(
                    t!(
                        "mcp.client.stdio_process_creation_failed",
                        command = original_cmd_str,
                        error = original_error_message // Keep original OS error for context
                    )
                    .to_string(),
                )
            }
        })?;

        let mut client_info = ClientConfig::default();
        client_info.protocol_version = Default::default();
        client_info.capabilities = ClientCapabilities::default();
        client_info.client_info =
            Implementation::new("Chatspeed MCP Client", env!("CARGO_PKG_VERSION"))
                .with_title("Chatspeed")
                .with_website_url("https://chatspeed.aidyou.ai");
        client_info.serve(process).await.map_err(|e| {
            // Optional: Wrap with t!
            let detailed_error = e.to_string();
            log::error!("Start StdioClient error: {}", detailed_error);
            McpError::ClientStartError(
                t!(
                    "mcp.client.stdio_service_start_failed",
                    command = original_cmd_str, // Report based on original configured command
                    error = detailed_error
                )
                .to_string(),
            )
        })
    }

    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>> {
        self.core.get_client_instance_arc()
    }

    async fn name(&self) -> String {
        self.core.get_name().await
    }

    async fn config(&self) -> McpServerConfig {
        self.core.get_config().await
    }

    async fn update_disabled_tools(
        &self,
        tool_name: &str,
        is_disabled: bool,
    ) -> McpClientResult<()> {
        self.core
            .update_disabled_tools(tool_name, is_disabled)
            .await;
        Ok(())
    }

    async fn status(&self) -> McpStatus {
        self.core.get_status().await
    }

    async fn on_status_change(&self, callback: StatusChangeCallback) {
        self.core.set_on_status_change_callback(callback).await;
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use rmcp::{transport::TokioChildProcess, ServiceExt};
    use tokio::process::Command;

    use crate::mcp::{
        client::{McpClient as _, McpProtocolType, McpServerConfig, McpStatus, StdioClient},
        McpError,
    };

    /// Absolute path to the deterministic, network-free stdio fixture shipped
    /// with the crate. Resolving it from `CARGO_MANIFEST_DIR` keeps the test
    /// independent of the process' working directory.
    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|dir| dir.file_name() == Some(std::ffi::OsStr::new("src-tauri")))
            .expect("the backend crate lives under src-tauri")
            .join("fixtures/mcp_stdio.py")
    }

    fn fixture_client() -> StdioClient {
        let config = McpServerConfig {
            name: "local-stdio-fixture".into(),
            protocol_type: McpProtocolType::Stdio,
            command: Some("python3".into()),
            args: Some(vec![
                "-u".into(),
                fixture_path().to_string_lossy().into_owned(),
            ]),
            timeout: Some(10),
            ..Default::default()
        };
        StdioClient::new(config).expect("the fixture config is valid")
    }

    /// `start()` only returns once the `initialize` handshake completed, so the
    /// negotiated server identity is already available. The invoked tool name
    /// comes from `list_tools`, never a hard-coded third-party tool.
    #[tokio::test]
    #[cfg(unix)]
    async fn stdio_start_awaits_initialize_and_calls_a_declared_tool() -> Result<(), McpError> {
        let client = fixture_client();
        client.start().await?;

        // A known peer identity on a `Connected` client proves `start()` awaited
        // the `initialize` exchange instead of returning early.
        {
            let service = client.client();
            let guard = service.read().await;
            let running = guard
                .as_ref()
                .expect("start must store the running service");
            let peer_info = running.peer_info();
            let server_name = peer_info
                .as_ref()
                .and_then(|peer| peer.server_info.as_ref())
                .map(|info| info.name.as_str());
            assert_eq!(server_name, Some("local-fixture"));
        }
        assert_eq!(client.status().await, McpStatus::Connected);

        let tools = client.list_tools().await?;
        assert_eq!(client.status().await, McpStatus::Running);
        assert_eq!(tools.len(), 1, "the fixture declares exactly one tool");
        let declared_tool = tools[0].name.clone();
        assert_eq!(declared_tool, "fixture_echo");

        // The fixture refuses to execute tools so an accidental invocation can
        // never masquerade as a read; the client must surface that rejection as
        // a classified call error instead of fabricating a successful result.
        let error = client
            .call(&declared_tool, serde_json::json!({}))
            .await
            .expect_err("the fixture refuses to execute declared tools");
        assert!(
            matches!(error, McpError::ClientCallError(_)),
            "the declared-tool call must fail as a call error, got: {error:?}"
        );

        client.stop().await?;
        assert_eq!(client.status().await, McpStatus::Stopped);
        Ok(())
    }

    /// Smoke test against the external `tavily-mcp` npm provider. It needs
    /// `npx`, npm registry access and a live network, so it is ignored by
    /// default and only runs explicitly with `--ignored`.
    #[tokio::test]
    #[ignore = "requires npx and network access to the npm registry"]
    async fn stdio_external_npx_provider_smoke() -> Result<(), String> {
        let mut cmd = Command::new("npx");
        cmd.args(&["-y", "tavily-mcp"]);
        let service =
            ().serve(TokioChildProcess::new(cmd).map_err(|e| e.to_string())?)
                .await
                .map_err(|e| e.to_string())?;

        let server_info = service.peer_info();
        log::info!("Connected to server: {server_info:#?}");

        let tools = service
            .list_tools(Default::default())
            .await
            .map_err(|e| e.to_string())?;
        log::info!("Available tools: {tools:#?}");

        service.cancel().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
