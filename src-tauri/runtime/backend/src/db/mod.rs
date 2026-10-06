pub mod agent;
// Runtime-only API-key encryption implementation. The desktop delegates every
// API-key/config command to the standalone runtime, so it never links the
// crypto or its key material handling; only the desktop-free crate compiles it.
#[cfg(not(feature = "desktop"))]
pub mod api_key_crypto;
pub mod automation;
// Runtime-only backup/restore machinery. The desktop delegates every backup
// command to the standalone runtime over the control plane and never links the
// file-level implementation, so only the desktop-free crate compiles it.
#[cfg(not(feature = "desktop"))]
pub mod backup;
#[cfg(not(feature = "desktop"))]
pub mod backup_crypto;
// Conversation/message CRUD is runtime-only; the desktop reads chat history
// through the control plane and only shares the `Conversation` DTO.
#[cfg(not(feature = "desktop"))]
pub mod chat;
pub mod chat_hub;
// Runtime-only configuration persistence. The desktop reads and writes the
// runtime-owned configuration through the control plane and keeps only its own
// `RuntimeConfigCache`, so it compiles no config implementation.
#[cfg(not(feature = "desktop"))]
pub mod config;
pub mod config_transfer;
pub mod error;
// Runtime-only database facade. `MainStore`/`DbRuntime` own the single writer
// thread and the reader pool; the desktop reaches the database only through the
// control plane, so the whole owner is compiled out of the desktop crate.
#[cfg(not(feature = "desktop"))]
pub mod main_store;
#[cfg(not(feature = "desktop"))]
pub mod runtime;
// pub mod plugin;
// Runtime-only proxy statistics implementation; the desktop reads them through
// the control plane.
#[cfg(not(feature = "desktop"))]
mod ccproxy;
mod mcp;
mod note;
mod proxy_group;
pub mod sandbox_scheme;
// Runtime-only schema/migrations implementation. `sql::migrations` opens and
// upgrades the database file, which only the runtime owner does.
#[cfg(not(feature = "desktop"))]
mod sql;
mod types;
mod workflow;
// Workflow usage aggregation is runtime-only. The desktop reads the finalized
// summaries through the control plane, so it never links this implementation.
#[cfg(not(feature = "desktop"))]
pub mod workflow_usage;

#[cfg(not(feature = "desktop"))]
pub use agent::AgentConfig;
pub use agent::{Agent, McpToolConfig};
#[cfg(not(feature = "desktop"))]
pub use automation::WorkflowAutomationUpsert;
pub use automation::{WorkflowAutomation, WorkflowAutomationRun};
#[cfg(not(feature = "desktop"))]
pub use backup::{BackupConfig, DbBackup};
pub use chat_hub::ChatHub;
pub use error::StoreError;
#[cfg(not(feature = "desktop"))]
pub use main_store::MainStore;
pub use mcp::Mcp;
pub use note::{Note, NoteTag};
pub use proxy_group::ProxyGroup;
pub use sandbox_scheme::SandboxScheme;
pub use types::{AiModel, AiSkill, Conversation, ModelConfig, PricingConfig, ThinkingConfig};
// ccproxy statistics and pricing tiers are produced by the runtime proxy and
// read by the runtime statistics/configuration paths only.
#[cfg(not(feature = "desktop"))]
pub use types::{CcproxyStat, PricingTier};
pub use workflow::{Workflow, WorkflowEfficiencyReport, WorkflowMessage};
#[cfg(not(feature = "desktop"))]
pub use workflow::{WorkflowAiContextMessage, WorkflowSnapshot};
