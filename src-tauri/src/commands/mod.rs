pub mod agent;
pub mod capability;
pub mod ccproxy;
pub mod chat;
pub mod chat_hub;
pub mod clipboard;
pub mod config_transfer;
pub mod constants;
pub mod dev_tool;
pub mod env;
pub mod fs;
pub mod mcp;
pub mod message;
pub mod model_catalog;
pub mod note;
pub mod plugin;
pub mod proxy_group;
pub mod sandbox;
pub mod sensitive;
pub mod setting;
pub mod terminal;
pub mod types;
pub mod updater;
pub mod window;
// The workflow Tauri wire is a desktop adapter: the shared cores and wire types
// are re-exported from the runtime backend, and this file holds only the
// `#[tauri::command]` wrappers that forward to `crate::runtime_workflow`.
pub mod workflow;
pub mod workflow_automation;
