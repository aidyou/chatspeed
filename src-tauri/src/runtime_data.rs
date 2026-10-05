//! Canonical runtime-owned data-command surface.
//!
//! These are the desktop command domains whose data lives in the runtime-owned
//! `MainStore`/`ChatState`: agent ordering and tool metadata, proxy groups,
//! notes, conversations and messages, sandbox schemes, the sensitive-filter
//! configuration, ChatHubs, ccproxy statistics, configuration transfer and the
//! configuration/model/skill/backup settings.
//!
//! The logic that used to run inside the Tauri command wrappers is extracted
//! here as transport-neutral cores. The Tauri wrappers now only translate their
//! wire into one `/control/v1/data-commands/{command}` call and re-apply
//! desktop-only side effects (locale, tray, shortcuts, window events, file
//! uploads) after a successful reply.
//!
//! The module is compiled by both the desktop crate and the desktop-free
//! runtime backend, so it must not reference Tauri, an `AppHandle` or any
//! desktop resource. Every command is an explicit entry in [`data_command_kind`];
//! there is deliberately no generic RPC, SQL, filesystem or plugin surface.
//! Request bodies are strongly typed `serde` structures and responses keep the
//! exact historical (camelCase) Tauri shapes, including opaque `Value` payloads
//! so arbitrary metadata is never re-cased.
//!
//! Desktop-only code (the thin HTTP adapters) is gated behind
//! `#[cfg(feature = "desktop")]`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(not(feature = "desktop"))]
use std::path::Path;
#[cfg(not(feature = "desktop"))]
use std::sync::{Arc, OnceLock};

#[cfg(feature = "desktop")]
use crate::ai::model_catalog::ResolvedModelProfile;
#[cfg(feature = "desktop")]
use crate::ai::traits::chat::ModelDetails;
#[cfg(not(feature = "desktop"))]
use crate::ai::transport::resolve as resolve_transport;
#[cfg(not(feature = "desktop"))]
use crate::ai::util::{
    get_family_from_model_id, is_function_call_supported, is_image_input_supported,
    is_reasoning_supported,
};
#[cfg(not(feature = "desktop"))]
use crate::constants::{CFG_ACTIVE_PROXY_GROUP, CFG_INTERFACE_LANGUAGE};
#[cfg(not(feature = "desktop"))]
use crate::db::config_transfer;
use crate::db::config_transfer::ConfigCategory;
#[cfg(not(feature = "desktop"))]
use crate::db::runtime::DbRuntime;
#[cfg(not(feature = "desktop"))]
use crate::db::MainStore;
use crate::db::{ModelConfig, ProxyGroup, SandboxScheme};
#[cfg(not(feature = "desktop"))]
use crate::model_catalog_engine::resolve_model_profile_from_catalog_with_context;
#[cfg(not(feature = "desktop"))]
use crate::sensitive::manager::FilterManager;
use crate::sensitive::manager::SensitiveConfig;
#[cfg(not(feature = "desktop"))]
use crate::workflow::react::application::{ApplicationError, WorkflowApplicationService};
#[cfg(not(feature = "desktop"))]
use chatspeed_contracts::ChatProtocolDto;
#[cfg(feature = "desktop")]
use chatspeed_contracts::ModelsDevPresetProviderDto;
use chatspeed_contracts::{
    ModelDetailsDto, ModelsDevProviderModelsRequest, ResolveModelProfileRequest,
};

#[cfg(not(feature = "desktop"))]
use crate::db::{BackupConfig, DbBackup};
#[cfg(not(feature = "desktop"))]
use rust_i18n::t;

#[cfg(feature = "desktop")]
use crate::db::{ChatHub, Conversation, Note, NoteTag};
#[cfg(feature = "desktop")]
use crate::runtime_client::RuntimeSupervisor;
#[cfg(feature = "desktop")]
use chatspeed_runtime_client::{ClientError, RuntimeClient};

/// Machine-specific configuration keys that a restored backup must never
/// overwrite: local paths, network bindings and proxy credentials.
///
/// Public so both crates can reference the same list without a dead-code
/// warning when the desktop-free backup path is compiled out.
#[cfg(not(feature = "desktop"))]
pub const MACHINE_SPECIFIC_CONFIG_KEYS: &[&str] = &[
    "backup_dir",
    crate::db::api_key_crypto::API_KEY_FILE_CONFIG_KEY,
    crate::constants::CFG_WINDOW_POSITION,
    crate::constants::CFG_WINDOW_SIZE,
    crate::constants::CFG_ASSISTANT_WINDOW_SIZE,
    crate::constants::CFG_WORKFLOW_WINDOW_SIZE,
    crate::constants::CFG_WORKFLOW_WINDOW_POSITION,
    crate::constants::CFG_CCPROXY_PORT,
    crate::constants::CFG_CCPROXY_LISTEN,
    "proxy_type",
    "proxy_server",
    "proxy_username",
    "proxy_password",
];

/// Response for [`dispatch_data_command`]'s `restore_setting` command.
///
/// Serialized with the historical Tauri field names so the frontend keeps the
/// same JSON shape.
#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreSettingResponse {
    /// Whether the restoration succeeded.
    pub success: bool,
    /// Warning when some files were skipped (e.g. locked MCP sessions).
    pub warning: Option<String>,
    /// Whether an application restart is recommended.
    pub restart_recommended: bool,
}

/// Whether a data command reads or mutates runtime-owned state.
///
/// The transport uses this to require an idempotency key only for mutations;
/// every command is listed explicitly so the allowlist is a complete table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(feature = "desktop"))]
pub enum DataCommandKind {
    /// A read that performs no durable mutation.
    Read,
    /// A mutation that must be retried through the idempotency journal.
    Mutation,
}

/// Returns the kind of a known data command, or `None` for an unknown command.
#[cfg(not(feature = "desktop"))]
pub fn data_command_kind(command: &str) -> Option<DataCommandKind> {
    use DataCommandKind::{Mutation, Read};
    let kind = match command {
        // Agent ordering and tool introspection.
        "get_available_tools" => Read,
        "update_agent_order" => Mutation,
        // Proxy groups.
        "proxy_group_list" => Read,
        "proxy_group_add" => Mutation,
        "proxy_group_update" => Mutation,
        "proxy_group_batch_update" => Mutation,
        "proxy_group_delete" => Mutation,
        "get_active_proxy_group" => Read,
        "set_active_proxy_group" => Mutation,
        // Notes and tags.
        "get_tags" => Read,
        "get_notes" => Read,
        "get_note" => Read,
        "search_notes" => Read,
        "add_note" => Mutation,
        "delete_note" => Mutation,
        // Conversations and messages.
        "get_all_conversations" => Read,
        "get_conversation_by_id" => Read,
        "get_messages_for_conversation" => Read,
        "add_conversation" => Mutation,
        "update_conversation" => Mutation,
        "delete_conversation" => Mutation,
        "add_message" => Mutation,
        "delete_message" => Mutation,
        "update_message_metadata" => Mutation,
        // Sandbox schemes.
        "get_sandbox_schemes" => Read,
        "add_sandbox_scheme" => Mutation,
        "update_sandbox_scheme" => Mutation,
        "delete_sandbox_scheme" => Mutation,
        // Sensitive-filter configuration.
        "get_sensitive_config" => Read,
        "update_sensitive_config" => Mutation,
        // ChatHub entries.
        "get_all_chat_hubs" => Read,
        "add_chat_hub" => Mutation,
        "update_chat_hub" => Mutation,
        "delete_chat_hub" => Mutation,
        "update_chat_hub_order" => Mutation,
        // ccproxy statistics.
        "get_ccproxy_daily_stats" => Read,
        "get_ccproxy_grouped_stats" => Read,
        "get_ccproxy_grouped_stats_by_date_range" => Read,
        "get_ccproxy_today_cost_stats" => Read,
        "get_ccproxy_provider_stats_by_date" => Read,
        "get_ccproxy_error_stats_by_date" => Read,
        "get_ccproxy_model_usage_stats" => Read,
        "get_ccproxy_model_token_usage_stats" => Read,
        "get_ccproxy_error_distribution_stats" => Read,
        "get_ccproxy_provider_token_usage_stats" => Read,
        "delete_ccproxy_stats" => Mutation,
        // Models.dev catalog (runtime-owned snapshot).
        "get_models_dev_providers" => Read,
        "get_models_dev_provider_models" => Read,
        "resolve_model_profile" => Read,
        // Configuration transfer.
        "export_config_package" => Mutation,
        "import_config_package" => Mutation,
        // Configuration, models, skills and backup maintenance.
        "get_all_config" => Read,
        "set_config" => Mutation,
        "reload_config" => Mutation,
        "get_api_key_encryption_status" => Read,
        "activate_api_key_file" => Mutation,
        "generate_api_key_file" => Mutation,
        "get_ai_model_by_id" => Read,
        "get_all_ai_models" => Read,
        "add_ai_model" => Mutation,
        "update_ai_model" => Mutation,
        "update_ai_model_order" => Mutation,
        "delete_ai_model" => Mutation,
        "get_ai_skill_by_id" => Read,
        "get_all_ai_skills" => Read,
        "add_ai_skill" => Mutation,
        "update_ai_skill" => Mutation,
        "update_ai_skill_order" => Mutation,
        "delete_ai_skill" => Mutation,
        "backup_setting" => Mutation,
        "restore_setting" => Mutation,
        "get_all_backups" => Read,
        _ => return None,
    };
    Some(kind)
}

/// Dispatches one allowlisted data command against the canonical runtime owner.
///
/// `body` is the typed command request; unknown commands are rejected, and a
/// malformed body is a stable `invalid_input` error. Responses keep the exact
/// historical Tauri shapes (camelCase, opaque `Value` payloads preserved) so the
/// desktop adapter can decode them back into its original Rust types.
#[cfg(not(feature = "desktop"))]
pub async fn dispatch_data_command(
    svc: &WorkflowApplicationService,
    command: &str,
    body: Value,
) -> Result<Value, ApplicationError> {
    match command {
        // Agent ordering and tool introspection.
        "get_available_tools" => get_available_tools_core(svc).await,
        "update_agent_order" => update_agent_order_core(svc, body).await,

        // Proxy groups.
        "proxy_group_list" => Ok(to_value(svc.main_store.config.get_proxy_groups())?),
        "proxy_group_add" => proxy_group_add_core(svc, body),
        "proxy_group_update" => proxy_group_update_core(svc, body),
        "proxy_group_batch_update" => proxy_group_batch_update_core(svc, body),
        "proxy_group_delete" => proxy_group_delete_core(svc, body),
        "get_active_proxy_group" => get_active_proxy_group_core(svc),
        "set_active_proxy_group" => set_active_proxy_group_core(svc, body),

        // Notes and tags.
        "get_tags" => note_get_tags_core(svc).await,
        "get_notes" => note_get_notes_core(svc, body).await,
        "get_note" => note_get_note_core(svc, body).await,
        "search_notes" => note_search_core(svc, body).await,
        "add_note" => note_add_core(svc, body).await,
        "delete_note" => note_delete_core(svc, body).await,

        // Conversations and messages.
        "get_all_conversations" => conversation_list_core(svc).await,
        "get_conversation_by_id" => conversation_get_core(svc, body).await,
        "get_messages_for_conversation" => messages_for_conversation_core(svc, body).await,
        "add_conversation" => conversation_add_core(svc, body).await,
        "update_conversation" => conversation_update_core(svc, body).await,
        "delete_conversation" => conversation_delete_core(svc, body).await,
        "add_message" => message_add_core(svc, body).await,
        "delete_message" => message_delete_core(svc, body).await,
        "update_message_metadata" => message_update_metadata_core(svc, body).await,

        // Sandbox schemes.
        "get_sandbox_schemes" => sandbox_list_core(svc),
        "add_sandbox_scheme" => sandbox_add_core(svc, body),
        "update_sandbox_scheme" => sandbox_update_core(svc, body),
        "delete_sandbox_scheme" => sandbox_delete_core(svc, body),

        // Sensitive-filter configuration.
        "get_sensitive_config" => Ok(to_value(
            svc.main_store
                .get_config("sensitive_config", SensitiveConfig::default()),
        )?),
        "update_sensitive_config" => sensitive_update_core(svc, body),

        // ChatHub entries.
        "get_all_chat_hubs" => Ok(to_value(
            svc.main_store.get_all_chat_hubs().map_err(store_error)?,
        )?),
        "add_chat_hub" => chat_hub_add_core(svc, body),
        "update_chat_hub" => chat_hub_update_core(svc, body),
        "delete_chat_hub" => chat_hub_delete_core(svc, body),
        "update_chat_hub_order" => chat_hub_order_core(svc, body),

        // ccproxy statistics.
        "get_ccproxy_daily_stats" => ccproxy_daily_core(svc, body).await,
        "get_ccproxy_grouped_stats" => ccproxy_grouped_core(svc, body).await,
        "get_ccproxy_grouped_stats_by_date_range" => ccproxy_grouped_range_core(svc, body).await,
        "get_ccproxy_today_cost_stats" => ccproxy_today_cost_core(svc).await,
        "get_ccproxy_provider_stats_by_date" => ccproxy_provider_by_date_core(svc, body).await,
        "get_ccproxy_error_stats_by_date" => ccproxy_error_by_date_core(svc, body).await,
        "get_ccproxy_model_usage_stats" => ccproxy_model_usage_core(svc, body).await,
        "get_ccproxy_model_token_usage_stats" => ccproxy_model_token_usage_core(svc, body).await,
        "get_ccproxy_error_distribution_stats" => ccproxy_error_distribution_core(svc, body).await,
        "get_ccproxy_provider_token_usage_stats" => {
            ccproxy_provider_token_usage_core(svc, body).await
        }
        "delete_ccproxy_stats" => ccproxy_delete_core(svc, body).await,

        // Models.dev catalog (runtime-owned snapshot).
        "get_models_dev_providers" => get_models_dev_providers_core(svc),
        "get_models_dev_provider_models" => get_models_dev_provider_models_core(svc, body),
        "resolve_model_profile" => resolve_model_profile_core(svc, body),

        // Configuration transfer.
        "export_config_package" => config_transfer_export_core(svc, body),
        "import_config_package" => config_transfer_import_core(svc, body),

        // Configuration, models, skills and backup maintenance.
        "get_all_config" => Ok(to_value(svc.main_store.config.settings())?),
        "set_config" => set_config_core(svc, body),
        "reload_config" => reload_config_core(svc),
        "get_api_key_encryption_status" => Ok(to_value(
            svc.main_store
                .api_key_encryption_status()
                .map_err(store_error)?,
        )?),
        "activate_api_key_file" => activate_api_key_file_core(svc, body),
        "generate_api_key_file" => generate_api_key_file_core(svc, body),
        "get_ai_model_by_id" => get_ai_model_by_id_core(svc, body),
        "get_all_ai_models" => Ok(to_value(
            svc.main_store.config.get_ai_models().map_err(store_error)?,
        )?),
        "add_ai_model" => add_ai_model_core(svc, body),
        "update_ai_model" => update_ai_model_core(svc, body),
        "update_ai_model_order" => update_ai_model_order_core(svc, body),
        "delete_ai_model" => delete_ai_model_core(svc, body),
        "get_ai_skill_by_id" => get_ai_skill_by_id_core(svc, body),
        "get_all_ai_skills" => Ok(to_value(svc.main_store.config.get_ai_skills())?),
        "add_ai_skill" => add_ai_skill_core(svc, body),
        "update_ai_skill" => update_ai_skill_core(svc, body),
        "update_ai_skill_order" => update_ai_skill_order_core(svc, body),
        "delete_ai_skill" => delete_ai_skill_core(svc, body),
        #[cfg(not(feature = "desktop"))]
        "backup_setting" => backup_setting_core(svc, body).await,
        #[cfg(not(feature = "desktop"))]
        "restore_setting" => restore_setting_core(svc, body).await,
        #[cfg(not(feature = "desktop"))]
        "get_all_backups" => get_all_backups_core(svc, body),
        #[cfg(feature = "desktop")]
        "backup_setting" | "restore_setting" | "get_all_backups" => Err(
            ApplicationError::internal("backup and restore run only in the desktop-free runtime"),
        ),

        other => Err(ApplicationError::not_found(format!(
            "unknown data command `{other}`"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Deserializes a typed request body, rejecting unknown fields.
#[cfg(not(feature = "desktop"))]
fn parse_body<T: DeserializeOwned>(body: Value) -> Result<T, ApplicationError> {
    serde_json::from_value(body).map_err(|error| {
        ApplicationError::invalid_input(format!("invalid data command body: {error}"))
    })
}

/// Serializes a successful response into the exact historical wire shape.
#[cfg(not(feature = "desktop"))]
fn to_value<T: Serialize>(value: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(value).map_err(|error| ApplicationError::internal(error.to_string()))
}

/// Maps a store failure to a stable internal application error.
#[cfg(not(feature = "desktop"))]
fn store_error(error: impl std::fmt::Display) -> ApplicationError {
    ApplicationError::internal(error.to_string())
}

/// Resolves the writer/reader runtime the store owns.
#[cfg(not(feature = "desktop"))]
fn db_runtime(svc: &WorkflowApplicationService) -> Result<Arc<DbRuntime>, ApplicationError> {
    svc.main_store.db_runtime().map_err(store_error)
}

/// The single in-process sensitive filter used by the message core.
#[cfg(not(feature = "desktop"))]
fn sensitive_filter_manager() -> &'static FilterManager {
    static MANAGER: OnceLock<FilterManager> = OnceLock::new();
    MANAGER.get_or_init(FilterManager::new)
}

/// Git review tools are instantiated with a session `PathGuard` only for child
/// workflows. Metadata is exposed for agent configuration without registering
/// executable instances globally.
#[cfg(not(feature = "desktop"))]
fn git_review_tool_metadata() -> Vec<Value> {
    vec![
        json!({
            "id": crate::tools::TOOL_GIT_DIFF,
            "name": crate::tools::TOOL_GIT_DIFF,
            "category": "FileSystem",
            "scope": "workflow",
            "child_only": true
        }),
        json!({
            "id": crate::tools::TOOL_GIT_INSPECT,
            "name": crate::tools::TOOL_GIT_INSPECT,
            "category": "FileSystem",
            "scope": "workflow",
            "child_only": true
        }),
    ]
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

/// Typed request for `update_agent_order`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentOrderBody {
    pub agent_ids: Vec<String>,
}

#[cfg(not(feature = "desktop"))]
async fn get_available_tools_core(
    svc: &WorkflowApplicationService,
) -> Result<Value, ApplicationError> {
    let mut native_meta = svc
        .chat_state
        .tool_manager
        .get_all_native_tool_metadata()
        .await;
    let mcp_meta = svc
        .chat_state
        .tool_manager
        .get_mcp_tool_specs(None)
        .await
        .into_iter()
        .map(|tool| {
            json!({
                "id": tool.canonical_name,
                "name": tool.declaration.name,
                "category": "MCP",
                "scope": tool.declaration.scope.unwrap_or(crate::tools::ToolScope::Both),
            })
        });
    native_meta.extend(mcp_meta);
    native_meta.extend(git_review_tool_metadata());
    native_meta.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });
    Ok(json!(native_meta))
}

#[cfg(not(feature = "desktop"))]
async fn update_agent_order_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let UpdateAgentOrderBody { agent_ids } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::update_agent_order_with_runtime(runtime, agent_ids)
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Proxy groups
// ---------------------------------------------------------------------------

/// Typed request carrying one proxy group.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyGroupBody {
    pub item: ProxyGroup,
}

/// Typed request for `proxy_group_batch_update`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyGroupBatchUpdateBody {
    pub ids: Vec<i64>,
    pub prompt_injection: Option<String>,
    pub prompt_text: Option<String>,
    pub tool_filter: Option<String>,
    pub injection_position: Option<String>,
    pub injection_condition: Option<String>,
    pub prompt_replace: Option<Value>,
}

/// Typed request for `proxy_group_delete`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyGroupDeleteBody {
    pub id: i64,
}

/// Typed request for `set_active_proxy_group`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetActiveProxyGroupBody {
    pub name: String,
}

#[cfg(not(feature = "desktop"))]
fn proxy_group_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ProxyGroupBody { item } = parse_body(body)?;
    let id = svc.main_store.proxy_group_add(&item).map_err(store_error)?;
    Ok(json!(id))
}

#[cfg(not(feature = "desktop"))]
fn proxy_group_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ProxyGroupBody { item } = parse_body(body)?;
    svc.main_store
        .proxy_group_update(&item)
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn proxy_group_batch_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: ProxyGroupBatchUpdateBody = parse_body(body)?;
    svc.main_store
        .proxy_group_batch_update(
            request.ids,
            request.prompt_injection,
            request.prompt_text,
            request.tool_filter,
            request.injection_position,
            request.injection_condition,
            request.prompt_replace,
        )
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn proxy_group_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ProxyGroupDeleteBody { id } = parse_body(body)?;
    svc.main_store.proxy_group_delete(id).map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn get_active_proxy_group_core(
    svc: &WorkflowApplicationService,
) -> Result<Value, ApplicationError> {
    let name = svc
        .main_store
        .config
        .get_setting(CFG_ACTIVE_PROXY_GROUP)
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "default".to_string());
    Ok(Value::String(name))
}

#[cfg(not(feature = "desktop"))]
fn set_active_proxy_group_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SetActiveProxyGroupBody { name } = parse_body(body)?;
    svc.main_store
        .set_config(CFG_ACTIVE_PROXY_GROUP, &json!(name))
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

/// Typed request for `add_note`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddNoteBody {
    pub title: String,
    pub content: String,
    pub conversation_id: Option<i64>,
    pub message_id: Option<i64>,
    pub tags: Vec<String>,
    pub metadata: Option<Value>,
}

/// Typed request for `get_notes`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetNotesBody {
    pub tag_id: Option<i64>,
}

/// Typed request carrying a numeric id.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdBody {
    pub id: i64,
}

/// Typed request for `search_notes`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchNotesBody {
    pub kw: String,
}

#[cfg(not(feature = "desktop"))]
async fn note_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: AddNoteBody = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::add_note_with_runtime(
        runtime,
        request.title,
        request.content,
        request.conversation_id,
        request.message_id,
        request.tags,
        request.metadata,
    )
    .await
    .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
async fn note_get_tags_core(svc: &WorkflowApplicationService) -> Result<Value, ApplicationError> {
    let runtime = db_runtime(svc)?;
    let tags = MainStore::get_tags_with_runtime(runtime)
        .await
        .map_err(store_error)?;
    to_value(tags)
}

#[cfg(not(feature = "desktop"))]
async fn note_get_notes_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let GetNotesBody { tag_id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let notes = MainStore::get_notes_with_runtime(runtime, tag_id)
        .await
        .map_err(store_error)?;
    to_value(notes)
}

#[cfg(not(feature = "desktop"))]
async fn note_get_note_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let note = MainStore::get_note_with_runtime(runtime, id)
        .await
        .map_err(store_error)?;
    to_value(note)
}

#[cfg(not(feature = "desktop"))]
async fn note_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::delete_note_with_runtime(runtime, id)
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
async fn note_search_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SearchNotesBody { kw } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let notes = MainStore::search_notes_with_runtime(runtime, kw)
        .await
        .map_err(store_error)?;
    to_value(notes)
}

// ---------------------------------------------------------------------------
// Conversations and messages
// ---------------------------------------------------------------------------

/// Typed request for `add_conversation`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddConversationBody {
    pub title: String,
}

/// Typed request for `update_conversation`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationBody {
    pub id: i64,
    pub title: Option<String>,
    pub is_favorite: Option<bool>,
}

/// Typed request for `get_messages_for_conversation`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationIdBody {
    pub conversation_id: i64,
}

/// Typed request for `add_message`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddMessageBody {
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub metadata: Option<Value>,
}

/// Typed request for `delete_message`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteMessageBody {
    pub id: Vec<i64>,
}

/// Typed request for `update_message_metadata`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateMessageMetadataBody {
    pub id: i64,
    pub metadata: Value,
}

#[cfg(not(feature = "desktop"))]
async fn conversation_list_core(
    svc: &WorkflowApplicationService,
) -> Result<Value, ApplicationError> {
    let runtime = db_runtime(svc)?;
    let conversations = MainStore::get_all_conversations_with_runtime(runtime)
        .await
        .map_err(store_error)?;
    to_value(conversations)
}

#[cfg(not(feature = "desktop"))]
async fn conversation_get_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let conversation = MainStore::get_conversation_by_id_with_runtime(runtime, id)
        .await
        .map_err(store_error)?;
    to_value(conversation)
}

#[cfg(not(feature = "desktop"))]
async fn messages_for_conversation_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ConversationIdBody { conversation_id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let messages = MainStore::get_messages_for_conversation_with_runtime(runtime, conversation_id)
        .await
        .map_err(store_error)?;
    to_value(messages)
}

#[cfg(not(feature = "desktop"))]
async fn conversation_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let AddConversationBody { title } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let id = MainStore::add_conversation_with_runtime(runtime, title)
        .await
        .map_err(store_error)?;
    Ok(json!(id))
}

#[cfg(not(feature = "desktop"))]
async fn conversation_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: UpdateConversationBody = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::update_conversation_with_runtime(
        runtime,
        request.id,
        request.title,
        request.is_favorite,
    )
    .await
    .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
async fn conversation_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::delete_conversation_with_runtime(runtime, id)
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
async fn message_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: AddMessageBody = parse_body(body)?;
    let mut final_content = request.content;
    if request.role == "user" {
        let sensitive_config: SensitiveConfig = svc
            .main_store
            .get_config("sensitive_config", SensitiveConfig::default());
        if sensitive_config.enabled {
            let interface_lang: String = svc
                .main_store
                .get_config(CFG_INTERFACE_LANGUAGE, "en".to_string());
            let detected_code = match whatlang::detect(&final_content) {
                Some(info) => {
                    crate::libs::lang::lang_to_iso_639_1(&info.lang().code()).unwrap_or("en")
                }
                None => "en",
            };
            let languages = vec![detected_code, interface_lang.as_str()];
            final_content = sensitive_filter_manager().filter_text(
                &final_content,
                &languages,
                &sensitive_config,
            );
        }
    }
    let runtime = db_runtime(svc)?;
    let id = MainStore::add_message_with_runtime(
        runtime,
        request.conversation_id,
        request.role,
        final_content.clone(),
        request.metadata,
    )
    .await
    .map_err(store_error)?;
    // The historical Tauri command returned `(i64, String)`, i.e. a JSON array.
    Ok(json!([id, final_content]))
}

#[cfg(not(feature = "desktop"))]
async fn message_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DeleteMessageBody { id } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::delete_message_with_runtime(runtime, id)
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
async fn message_update_metadata_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let UpdateMessageMetadataBody { id, metadata } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::update_message_metadata_with_runtime(runtime, id, Some(metadata))
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Sandbox schemes
// ---------------------------------------------------------------------------

/// Typed request carrying one sandbox scheme.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSchemeBody {
    pub scheme: SandboxScheme,
}

/// Typed request for `delete_sandbox_scheme`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSchemeIdBody {
    pub id: String,
}

/// Assigns a fresh TSID to every scheme item that has no client-supplied id.
#[cfg(not(feature = "desktop"))]
pub(crate) fn assign_missing_scheme_item_ids(
    scheme: &mut SandboxScheme,
    tsid_generator: &crate::libs::tsid::TsidGenerator,
) -> Result<(), String> {
    for profile in &mut scheme.config.profiles {
        if profile.id.trim().is_empty() {
            profile.id = tsid_generator
                .generate()
                .map_err(|error| error.to_string())?;
        }
    }
    for rule in &mut scheme.config.host_rules {
        if rule.id.trim().is_empty() {
            rule.id = tsid_generator
                .generate()
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "desktop"))]
fn sandbox_list_core(svc: &WorkflowApplicationService) -> Result<Value, ApplicationError> {
    let schemes = svc
        .main_store
        .get_all_sandbox_schemes()
        .map_err(store_error)?;
    to_value(schemes)
}

#[cfg(not(feature = "desktop"))]
fn sandbox_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SandboxSchemeBody { scheme } = parse_body(body)?;
    let mut scheme = scheme;
    scheme.id = svc.tsid_generator.generate().map_err(store_error)?;
    assign_missing_scheme_item_ids(&mut scheme, &svc.tsid_generator).map_err(store_error)?;
    svc.main_store
        .add_sandbox_scheme(&scheme)
        .map_err(store_error)?;
    Ok(Value::String(scheme.id))
}

#[cfg(not(feature = "desktop"))]
fn sandbox_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SandboxSchemeBody { scheme } = parse_body(body)?;
    let mut scheme = scheme;
    assign_missing_scheme_item_ids(&mut scheme, &svc.tsid_generator).map_err(store_error)?;
    svc.main_store
        .update_sandbox_scheme(&scheme)
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn sandbox_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SandboxSchemeIdBody { id } = parse_body(body)?;
    svc.main_store
        .delete_sandbox_scheme(&id)
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Sensitive configuration
// ---------------------------------------------------------------------------

/// Typed request carrying the sensitive-filter configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitiveConfigBody {
    pub config: SensitiveConfig,
}

#[cfg(not(feature = "desktop"))]
fn sensitive_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SensitiveConfigBody { config } = parse_body(body)?;
    let config_value = serde_json::to_value(&config).map_err(store_error)?;
    svc.main_store
        .set_config("sensitive_config", &config_value)
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// ChatHubs
// ---------------------------------------------------------------------------

/// Typed request for `add_chat_hub`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatHubAddBody {
    pub name: String,
    pub logo: String,
    pub url: String,
}

/// Typed request for `update_chat_hub`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatHubUpdateBody {
    pub id: i64,
    pub name: String,
    pub logo: String,
    pub url: String,
}

/// Typed request for `update_chat_hub_order`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatHubOrderBody {
    pub hub_ids: Vec<i64>,
}

#[cfg(not(feature = "desktop"))]
fn chat_hub_add_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ChatHubAddBody { name, logo, url } = parse_body(body)?;
    let hub = svc
        .main_store
        .add_chat_hub(&name, &logo, &url)
        .map_err(store_error)?;
    to_value(hub)
}

#[cfg(not(feature = "desktop"))]
fn chat_hub_update_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ChatHubUpdateBody {
        id,
        name,
        logo,
        url,
    } = parse_body(body)?;
    let hub = svc
        .main_store
        .update_chat_hub(id, &name, &logo, &url)
        .map_err(store_error)?;
    to_value(hub)
}

#[cfg(not(feature = "desktop"))]
fn chat_hub_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    svc.main_store.delete_chat_hub(id).map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn chat_hub_order_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ChatHubOrderBody { hub_ids } = parse_body(body)?;
    svc.main_store
        .update_chat_hub_order(hub_ids)
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// ccproxy statistics
// ---------------------------------------------------------------------------

/// Typed request with a day window.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaysBody {
    pub days: i32,
}

/// Typed request with an explicit date range.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateRangeBody {
    pub start_date: String,
    pub end_date: String,
}

/// Typed request with one date.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateBody {
    pub date: String,
}

/// Typed request for `get_ccproxy_error_stats_by_date`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorStatsBody {
    pub date: String,
    pub client_model: Option<String>,
    pub backend_model: Option<String>,
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_daily_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_daily_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_grouped_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_grouped_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_grouped_range_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DateRangeBody {
        start_date,
        end_date,
    } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_grouped_stats_by_date_range_with_runtime(
        runtime,
        &start_date,
        &end_date,
    )
    .await
    .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_today_cost_core(
    svc: &WorkflowApplicationService,
) -> Result<Value, ApplicationError> {
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_today_cost_stats_with_runtime(runtime)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_provider_by_date_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DateBody { date } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_provider_stats_by_date_with_runtime(runtime, date)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_error_by_date_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ErrorStatsBody {
        date,
        client_model,
        backend_model,
    } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_error_stats_by_date_with_runtime(
        runtime,
        date,
        client_model,
        backend_model,
    )
    .await
    .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_model_usage_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_model_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_model_token_usage_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_model_token_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_error_distribution_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_error_distribution_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_provider_token_usage_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let stats = MainStore::get_ccproxy_provider_token_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    to_value(stats)
}

#[cfg(not(feature = "desktop"))]
async fn ccproxy_delete_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let DaysBody { days } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    MainStore::delete_ccproxy_stats_with_runtime(runtime, days)
        .await
        .map_err(store_error)?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------
// Configuration transfer
// ---------------------------------------------------------------------------

/// Typed request for the configuration package export/import commands.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigTransferBody {
    pub path: String,
    pub categories: Vec<ConfigCategory>,
}

#[cfg(not(feature = "desktop"))]
fn config_transfer_export_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ConfigTransferBody { path, categories } = parse_body(body)?;
    let runtime = db_runtime(svc)?;
    let preview = runtime
        .read_blocking(move |conn| config_transfer::export_config_package(conn, path, categories))
        .map_err(store_error)?;
    to_value(preview)
}

#[cfg(not(feature = "desktop"))]
fn config_transfer_import_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ConfigTransferBody { path, categories } = parse_body(body)?;
    let result = config_transfer::import_config_package(svc.main_store.as_ref(), path, categories)
        .map_err(store_error)?;
    to_value(result)
}

// ---------------------------------------------------------------------------
// Configuration, models, skills and backups
// ---------------------------------------------------------------------------

/// Typed request for `set_config`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetConfigBody {
    pub key: String,
    pub value: Value,
}

/// Typed request carrying a filesystem path.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathBody {
    pub path: String,
}

/// Typed request for the backup commands.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupBody {
    pub backup_dir: Option<String>,
}

/// Typed request for `restore_setting`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreBody {
    pub backup_dir: String,
}

/// Typed request for `add_ai_model`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddAiModelBody {
    pub name: String,
    pub models: Vec<ModelConfig>,
    pub default_model: String,
    pub api_protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub max_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub disabled: bool,
    pub metadata: Option<Value>,
}

/// Typed request for `update_ai_model`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAiModelBody {
    pub id: i64,
    pub name: String,
    pub models: Vec<ModelConfig>,
    pub default_model: String,
    pub api_protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub max_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub disabled: bool,
    pub metadata: Option<Value>,
}

/// Typed request for the AI model order command.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOrderBody {
    pub model_ids: Vec<i64>,
}

/// Typed request for `add_ai_skill`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddAiSkillBody {
    pub name: String,
    pub icon: Option<String>,
    pub logo: Option<String>,
    pub prompt: String,
    pub disabled: bool,
    pub metadata: Option<Value>,
}

/// Typed request for `update_ai_skill`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAiSkillBody {
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub logo: Option<String>,
    pub prompt: String,
    pub disabled: bool,
    pub metadata: Option<Value>,
}

/// Typed request for the AI skill order command.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillOrderBody {
    pub skill_ids: Vec<i64>,
}

#[cfg(not(feature = "desktop"))]
fn set_config_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SetConfigBody { key, value } = parse_body(body)?;
    // The interface language is normalized by the runtime, so the desktop
    // wrapper can re-derive the same value for its local locale side effect.
    let value = if key == CFG_INTERFACE_LANGUAGE {
        Value::String(
            crate::libs::lang::normalize_interface_locale(value.as_str().unwrap_or_default())
                .to_string(),
        )
    } else {
        value
    };

    if value.is_null() {
        svc.main_store.delete_config(&key).map_err(store_error)?;
    } else {
        svc.main_store
            .set_config(&key, &value)
            .map_err(store_error)?;
    }
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn reload_config_core(svc: &WorkflowApplicationService) -> Result<Value, ApplicationError> {
    svc.main_store.reload_config().map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn activate_api_key_file_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let PathBody { path } = parse_body(body)?;
    svc.main_store
        .activate_api_key_file(Path::new(&path))
        .map_err(store_error)?;
    to_value(
        svc.main_store
            .api_key_encryption_status()
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn generate_api_key_file_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let PathBody { path } = parse_body(body)?;
    svc.main_store
        .generate_and_activate_api_key_file(Path::new(&path))
        .map_err(store_error)?;
    to_value(
        svc.main_store
            .api_key_encryption_status()
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn get_ai_model_by_id_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    to_value(
        svc.main_store
            .config
            .get_ai_model_by_id(id)
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn add_ai_model_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: AddAiModelBody = parse_body(body)?;
    let id = svc
        .main_store
        .add_ai_model(
            request.name,
            request.models,
            request.default_model,
            request.api_protocol,
            request.base_url,
            request.api_key,
            request.max_tokens,
            request.temperature,
            request.top_p,
            request.top_k,
            request.disabled,
            request.metadata,
        )
        .map_err(store_error)?;
    to_value(
        svc.main_store
            .config
            .get_ai_model_by_id(id)
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn update_ai_model_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: UpdateAiModelBody = parse_body(body)?;
    svc.main_store
        .update_ai_model(
            request.id,
            request.name,
            request.models,
            request.default_model,
            request.api_protocol,
            request.base_url,
            request.api_key,
            request.max_tokens,
            request.temperature,
            request.top_p,
            request.top_k,
            request.disabled,
            request.metadata,
        )
        .map_err(store_error)?;
    to_value(
        svc.main_store
            .config
            .get_ai_model_by_id(request.id)
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn update_ai_model_order_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let ModelOrderBody { model_ids } = parse_body(body)?;
    svc.main_store
        .update_ai_model_order(model_ids)
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn delete_ai_model_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    svc.main_store.delete_ai_model(id).map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn get_ai_skill_by_id_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    to_value(
        svc.main_store
            .config
            .get_ai_skill_by_id(id)
            .map_err(store_error)?,
    )
}

#[cfg(not(feature = "desktop"))]
fn add_ai_skill_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: AddAiSkillBody = parse_body(body)?;
    // The logo is already an uploaded relative url, or empty: the desktop
    // wrapper performs the file upload before the remote call.
    let logo_url = request.logo.unwrap_or_default();
    let skill = svc
        .main_store
        .add_ai_skill(
            request.name,
            request.icon,
            Some(logo_url),
            request.prompt,
            request.disabled,
            request.metadata,
        )
        .map_err(store_error)?;
    to_value(skill)
}

#[cfg(not(feature = "desktop"))]
fn update_ai_skill_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: UpdateAiSkillBody = parse_body(body)?;
    let logo_url = request.logo.unwrap_or_default();
    let skill = svc
        .main_store
        .update_ai_skill(
            request.id,
            request.name,
            request.icon,
            Some(logo_url),
            request.prompt,
            request.disabled,
            request.metadata,
        )
        .map_err(store_error)?;
    to_value(skill)
}

#[cfg(not(feature = "desktop"))]
fn update_ai_skill_order_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let SkillOrderBody { skill_ids } = parse_body(body)?;
    svc.main_store
        .update_ai_skill_order(skill_ids)
        .map_err(store_error)?;
    Ok(Value::Null)
}

#[cfg(not(feature = "desktop"))]
fn delete_ai_skill_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let IdBody { id } = parse_body(body)?;
    svc.main_store.delete_ai_skill(id).map_err(store_error)?;
    Ok(Value::Null)
}

/// Flushes WAL and writes a full backup under the runtime-owned data directory.
#[cfg(not(feature = "desktop"))]
async fn backup_setting_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let BackupBody { backup_dir } = parse_body(body)?;
    svc.main_store.checkpoint().map_err(store_error)?;
    let app_dir = svc.app_data_dir.clone();
    tokio::task::spawn_blocking(move || {
        DbBackup::new(
            app_dir,
            BackupConfig {
                backup_dir,
                read_only: false,
            },
        )
        .and_then(|mut backup| backup.backup_to_directory())
    })
    .await
    .map_err(store_error)?
    .map_err(store_error)?;
    Ok(Value::Null)
}

/// Restores a full backup over the runtime-owned data directory.
///
/// The user-file directories mirror the canonical desktop layout
/// (`<data>/static/{theme,upload}`, `<data>/schema`, `<data>/shared`,
/// `<data>/mcp_sessions`), rooted at the runtime's own data directory.
#[cfg(not(feature = "desktop"))]
async fn restore_setting_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let RestoreBody { backup_dir } = parse_body(body)?;
    let app_dir = svc.app_data_dir.clone();
    let static_dir = app_dir.join("static");
    let theme_dir = static_dir.join("theme");
    let upload_dir = static_dir.join("upload");
    let schema_dir = app_dir.join("schema");
    let shared_dir = app_dir.join("shared");
    let mcp_sessions_dir = app_dir.join("mcp_sessions");
    let main_db_path = app_dir.join("chatspeed.db");

    let db_backup = DbBackup::new(
        &app_dir,
        BackupConfig {
            backup_dir: Some(backup_dir.clone()),
            read_only: true,
        },
    )
    .map_err(store_error)?;

    // Prefer compressed backups while retaining restore support for backups
    // created before database ZIP compression.
    let compressed_backup_db_file = Path::new(&backup_dir).join("chatspeed.db.zip");
    let backup_db_file = if compressed_backup_db_file.exists() {
        compressed_backup_db_file
    } else {
        Path::new(&backup_dir).join("chatspeed.db")
    };
    let temp_db_file = db_backup
        .decrypt_to_temp(&backup_db_file, &main_db_path)
        .map_err(store_error)?;

    svc.main_store
        .atomic_restore(&temp_db_file, &main_db_path, MACHINE_SPECIFIC_CONFIG_KEYS)
        .map_err(store_error)?;

    let files_skipped = db_backup
        .restore_user_files(
            &Path::new(&backup_dir).join("user_files.zip"),
            &theme_dir,
            &upload_dir,
            &mcp_sessions_dir,
            &schema_dir,
            &shared_dir,
            &static_dir,
        )
        .map_err(store_error)?;

    let response = if files_skipped {
        RestoreSettingResponse {
            success: true,
            warning: Some(t!("db.backup.some_files_skipped_restart_required").to_string()),
            restart_recommended: true,
        }
    } else {
        RestoreSettingResponse {
            success: true,
            warning: None,
            restart_recommended: false,
        }
    };
    to_value(response)
}

/// Lists available backups under the runtime-owned data directory.
#[cfg(not(feature = "desktop"))]
fn get_all_backups_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let BackupBody { backup_dir } = parse_body(body)?;
    let db_backup = DbBackup::new(
        &svc.app_data_dir,
        BackupConfig {
            backup_dir,
            read_only: true,
        },
    )
    .map_err(store_error)?;
    let backups = db_backup.list_backups().map_err(store_error)?;
    to_value(
        backups
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
    )
}

// ---------------------------------------------------------------------------
// Models.dev catalog cores (runtime-owned snapshot)
// ---------------------------------------------------------------------------

/// `get_models_dev_providers` — the runtime-owned provider presets.
#[cfg(not(feature = "desktop"))]
fn get_models_dev_providers_core(
    svc: &WorkflowApplicationService,
) -> Result<Value, ApplicationError> {
    to_value(svc.catalog().preset_providers().providers.clone())
}

/// `get_models_dev_provider_models` — one provider's embedded catalog models.
///
/// The response keeps the historical camelCase `ModelDetails` shape so the
/// desktop adapter can decode it back into its own model descriptor.
#[cfg(not(feature = "desktop"))]
fn get_models_dev_provider_models_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: ModelsDevProviderModelsRequest = parse_body(body)?;
    to_value(provider_models_from_catalog(
        svc.catalog().snapshot().as_ref(),
        &request.provider_id,
    ))
}

/// Maps one provider's embedded catalog models to the historical wire shape.
///
/// Split from the command core so the mapping can be tested against an embedded
/// catalog without assembling a full application service.
#[cfg(not(feature = "desktop"))]
fn provider_models_from_catalog(
    catalog: &crate::model_catalog_engine::ModelsDevCatalog,
    provider_id: &str,
) -> Vec<ModelDetailsDto> {
    catalog
        .providers
        .get(provider_id)
        .map(|provider| {
            provider
                .models
                .values()
                .map(|model| ModelDetailsDto {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    protocol: ChatProtocolDto::OpenAI,
                    max_input_tokens: model
                        .limit
                        .as_ref()
                        .and_then(|limit| limit.input.map(|value| value as u32)),
                    max_output_tokens: model
                        .limit
                        .as_ref()
                        .and_then(|limit| limit.output.map(|value| value as u32)),
                    description: None,
                    last_updated: model
                        .extra
                        .get("last_updated")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    family: model.family.clone(),
                    reasoning: model.reasoning,
                    function_call: model.tool_call,
                    image_input: model
                        .modalities
                        .as_ref()
                        .map(|modalities| modalities.input.iter().any(|input| input == "image")),
                    recommended_temperature: None,
                    metadata: None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// `resolve_model_profile` — catalog profile plus transport resolution.
///
/// The fallback heuristics that used to run inside the desktop command now run
/// here, so the runtime is the single implementation of profile resolution and
/// the desktop only forwards the resolved camelCase profile.
#[cfg(not(feature = "desktop"))]
fn resolve_model_profile_core(
    svc: &WorkflowApplicationService,
    body: Value,
) -> Result<Value, ApplicationError> {
    let request: ResolveModelProfileRequest = parse_body(body)?;
    let metadata_map = request.metadata.as_ref().and_then(|value| {
        value.as_object().map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
    });
    let provider_id = request
        .metadata
        .as_ref()
        .and_then(|value| value.get("modelsDevProviderId").and_then(Value::as_str));
    let mut profile = resolve_model_profile_from_catalog_with_context(
        &svc.catalog().snapshot(),
        &request.model_id,
        provider_id,
        request.base_url.as_deref(),
    )
    .map_err(|error| ApplicationError::internal(error.to_string()))?;
    let normalized_model_id = request.model_id.trim().to_ascii_lowercase();
    if profile.family.is_none() {
        profile.family = get_family_from_model_id(&normalized_model_id);
    }
    if profile.capabilities.reasoning.is_none() && is_reasoning_supported(&normalized_model_id) {
        profile.capabilities.reasoning = Some(true);
    }
    if profile.capabilities.function_call.is_none()
        && is_function_call_supported(&normalized_model_id)
    {
        profile.capabilities.function_call = Some(true);
    }
    if profile.capabilities.image_input.is_none() && is_image_input_supported(&normalized_model_id)
    {
        profile.capabilities.image_input = Some(true);
    }
    if let Some((adapter, transport_id)) = resolve_transport(
        &request.model_id,
        request.base_url.as_deref(),
        request.backend_protocol.as_deref(),
        metadata_map.as_ref(),
    )
    .map_err(|error| ApplicationError::internal(error.to_string()))?
    {
        profile.thinking_adapter = Some(adapter);
        profile.matched_transport_id = Some(transport_id);
    }
    to_value(profile)
}

// ---------------------------------------------------------------------------
// Desktop transport adapters
// ---------------------------------------------------------------------------

/// Canonical control-plane route for data commands.
#[cfg(feature = "desktop")]
const DATA_COMMAND_ROUTE: &str = chatspeed_runtime_client::DATA_COMMANDS_PATH;

/// Resolves the connected control-plane client, or fails when no lease is held.
#[cfg(feature = "desktop")]
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(|error| error.to_string())
}

/// Fresh idempotency key for one mutating command invocation.
#[cfg(feature = "desktop")]
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Maps a transport/domain client error to the Tauri string error.
#[cfg(feature = "desktop")]
fn map_client_error(error: ClientError) -> String {
    match error {
        ClientError::Server { message, .. } => message,
        other => other.to_string(),
    }
}

/// Encodes a typed request body for the wire.
#[cfg(feature = "desktop")]
fn encode<T: Serialize>(body: &T) -> Result<Value, String> {
    serde_json::to_value(body).map_err(|error| error.to_string())
}

/// Decodes a runtime response back into its original Tauri-wire type.
#[cfg(feature = "desktop")]
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Sends a read-only data command.
#[cfg(feature = "desktop")]
async fn read_command(
    supervisor: &RuntimeSupervisor,
    command: &str,
    body: Value,
) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{DATA_COMMAND_ROUTE}/{command}");
    client.post(&path, &body).await.map_err(map_client_error)
}

/// Sends a mutating data command with an idempotency key.
#[cfg(feature = "desktop")]
async fn mutate_command(
    supervisor: &RuntimeSupervisor,
    command: &str,
    body: Value,
) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{DATA_COMMAND_ROUTE}/{command}");
    client
        .post_with_idempotency(&path, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)
}

/// `get_available_tools` — native tool metadata for agent configuration.
#[cfg(feature = "desktop")]
pub async fn get_available_tools(supervisor: &RuntimeSupervisor) -> Result<Value, String> {
    read_command(supervisor, "get_available_tools", json!({})).await
}

/// `update_agent_order` — persists the agent sort order.
#[cfg(feature = "desktop")]
pub async fn update_agent_order(
    supervisor: &RuntimeSupervisor,
    agent_ids: Vec<String>,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_agent_order",
        encode(&UpdateAgentOrderBody { agent_ids })?,
    )
    .await?;
    Ok(())
}

/// `proxy_group_list` — all proxy groups.
#[cfg(feature = "desktop")]
pub async fn proxy_group_list(supervisor: &RuntimeSupervisor) -> Result<Vec<ProxyGroup>, String> {
    decode(read_command(supervisor, "proxy_group_list", json!({})).await?)
}

/// `proxy_group_add` — inserts one proxy group and returns its id.
#[cfg(feature = "desktop")]
pub async fn proxy_group_add(
    supervisor: &RuntimeSupervisor,
    item: ProxyGroup,
) -> Result<i64, String> {
    decode(
        mutate_command(
            supervisor,
            "proxy_group_add",
            encode(&ProxyGroupBody { item })?,
        )
        .await?,
    )
}

/// `proxy_group_update` — updates one proxy group.
#[cfg(feature = "desktop")]
pub async fn proxy_group_update(
    supervisor: &RuntimeSupervisor,
    item: ProxyGroup,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "proxy_group_update",
        encode(&ProxyGroupBody { item })?,
    )
    .await?;
    Ok(())
}

/// `proxy_group_batch_update` — applies prompt-injection edits to many groups.
#[cfg(feature = "desktop")]
#[allow(clippy::too_many_arguments)]
pub async fn proxy_group_batch_update(
    supervisor: &RuntimeSupervisor,
    body: ProxyGroupBatchUpdateBody,
) -> Result<(), String> {
    mutate_command(supervisor, "proxy_group_batch_update", encode(&body)?).await?;
    Ok(())
}

/// `proxy_group_delete` — removes one proxy group.
#[cfg(feature = "desktop")]
pub async fn proxy_group_delete(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(
        supervisor,
        "proxy_group_delete",
        encode(&ProxyGroupDeleteBody { id })?,
    )
    .await?;
    Ok(())
}

/// `get_active_proxy_group` — the active proxy group name.
#[cfg(feature = "desktop")]
pub async fn get_active_proxy_group(supervisor: &RuntimeSupervisor) -> Result<String, String> {
    decode(read_command(supervisor, "get_active_proxy_group", json!({})).await?)
}

/// `set_active_proxy_group` — sets the active proxy group name.
#[cfg(feature = "desktop")]
pub async fn set_active_proxy_group(
    supervisor: &RuntimeSupervisor,
    name: String,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "set_active_proxy_group",
        encode(&SetActiveProxyGroupBody { name })?,
    )
    .await?;
    Ok(())
}

/// `add_note` — creates a note.
#[cfg(feature = "desktop")]
pub async fn add_note(supervisor: &RuntimeSupervisor, body: AddNoteBody) -> Result<(), String> {
    mutate_command(supervisor, "add_note", encode(&body)?).await?;
    Ok(())
}

/// `get_tags` — all note tags.
#[cfg(feature = "desktop")]
pub async fn get_tags(supervisor: &RuntimeSupervisor) -> Result<Vec<NoteTag>, String> {
    decode(read_command(supervisor, "get_tags", json!({})).await?)
}

/// `get_notes` — notes, optionally filtered by tag.
#[cfg(feature = "desktop")]
pub async fn get_notes(
    supervisor: &RuntimeSupervisor,
    tag_id: Option<i64>,
) -> Result<Vec<Note>, String> {
    decode(read_command(supervisor, "get_notes", encode(&GetNotesBody { tag_id })?).await?)
}

/// `get_note` — one note by id.
#[cfg(feature = "desktop")]
pub async fn get_note(supervisor: &RuntimeSupervisor, id: i64) -> Result<Note, String> {
    decode(read_command(supervisor, "get_note", encode(&IdBody { id })?).await?)
}

/// `search_notes` — notes matching a keyword.
#[cfg(feature = "desktop")]
pub async fn search_notes(supervisor: &RuntimeSupervisor, kw: String) -> Result<Vec<Note>, String> {
    decode(read_command(supervisor, "search_notes", encode(&SearchNotesBody { kw })?).await?)
}

/// `delete_note` — removes a note.
#[cfg(feature = "desktop")]
pub async fn delete_note(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_note", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `get_all_conversations` — all conversations.
#[cfg(feature = "desktop")]
pub async fn get_all_conversations(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<Conversation>, String> {
    decode(read_command(supervisor, "get_all_conversations", json!({})).await?)
}

/// `get_conversation_by_id` — one conversation.
#[cfg(feature = "desktop")]
pub async fn get_conversation_by_id(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> Result<Conversation, String> {
    decode(
        read_command(
            supervisor,
            "get_conversation_by_id",
            encode(&IdBody { id })?,
        )
        .await?,
    )
}

/// `get_messages_for_conversation` — messages of one conversation.
#[cfg(feature = "desktop")]
pub async fn get_messages_for_conversation(
    supervisor: &RuntimeSupervisor,
    conversation_id: i64,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_messages_for_conversation",
            encode(&ConversationIdBody { conversation_id })?,
        )
        .await?,
    )
}

/// `add_conversation` — creates a conversation and returns its id.
#[cfg(feature = "desktop")]
pub async fn add_conversation(
    supervisor: &RuntimeSupervisor,
    title: String,
) -> Result<i64, String> {
    decode(
        mutate_command(
            supervisor,
            "add_conversation",
            encode(&AddConversationBody { title })?,
        )
        .await?,
    )
}

/// `update_conversation` — updates a conversation's title/favorite flag.
#[cfg(feature = "desktop")]
pub async fn update_conversation(
    supervisor: &RuntimeSupervisor,
    body: UpdateConversationBody,
) -> Result<(), String> {
    mutate_command(supervisor, "update_conversation", encode(&body)?).await?;
    Ok(())
}

/// `delete_conversation` — removes a conversation.
#[cfg(feature = "desktop")]
pub async fn delete_conversation(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_conversation", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `add_message` — stores a message after the runtime sensitive filter and
/// returns `(id, final_content)`.
#[cfg(feature = "desktop")]
pub async fn add_message(
    supervisor: &RuntimeSupervisor,
    body: AddMessageBody,
) -> Result<(i64, String), String> {
    decode(mutate_command(supervisor, "add_message", encode(&body)?).await?)
}

/// `delete_message` — removes one or more messages.
#[cfg(feature = "desktop")]
pub async fn delete_message(supervisor: &RuntimeSupervisor, id: Vec<i64>) -> Result<(), String> {
    mutate_command(
        supervisor,
        "delete_message",
        encode(&DeleteMessageBody { id })?,
    )
    .await?;
    Ok(())
}

/// `update_message_metadata` — replaces a message's metadata.
#[cfg(feature = "desktop")]
pub async fn update_message_metadata(
    supervisor: &RuntimeSupervisor,
    body: UpdateMessageMetadataBody,
) -> Result<(), String> {
    mutate_command(supervisor, "update_message_metadata", encode(&body)?).await?;
    Ok(())
}

/// `get_sandbox_schemes` — all sandbox schemes.
#[cfg(feature = "desktop")]
pub async fn get_sandbox_schemes(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<SandboxScheme>, String> {
    decode(read_command(supervisor, "get_sandbox_schemes", json!({})).await?)
}

/// `add_sandbox_scheme` — creates a sandbox scheme and returns its id.
#[cfg(feature = "desktop")]
pub async fn add_sandbox_scheme(
    supervisor: &RuntimeSupervisor,
    scheme: SandboxScheme,
) -> Result<String, String> {
    decode(
        mutate_command(
            supervisor,
            "add_sandbox_scheme",
            encode(&SandboxSchemeBody { scheme })?,
        )
        .await?,
    )
}

/// `update_sandbox_scheme` — updates a sandbox scheme.
#[cfg(feature = "desktop")]
pub async fn update_sandbox_scheme(
    supervisor: &RuntimeSupervisor,
    scheme: SandboxScheme,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_sandbox_scheme",
        encode(&SandboxSchemeBody { scheme })?,
    )
    .await?;
    Ok(())
}

/// `delete_sandbox_scheme` — removes a sandbox scheme.
#[cfg(feature = "desktop")]
pub async fn delete_sandbox_scheme(
    supervisor: &RuntimeSupervisor,
    id: String,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "delete_sandbox_scheme",
        encode(&SandboxSchemeIdBody { id })?,
    )
    .await?;
    Ok(())
}

/// `get_sensitive_config` — the sensitive-filter configuration.
#[cfg(feature = "desktop")]
pub async fn get_sensitive_config(
    supervisor: &RuntimeSupervisor,
) -> Result<SensitiveConfig, String> {
    decode(read_command(supervisor, "get_sensitive_config", json!({})).await?)
}

/// `update_sensitive_config` — replaces the sensitive-filter configuration.
#[cfg(feature = "desktop")]
pub async fn update_sensitive_config(
    supervisor: &RuntimeSupervisor,
    config: SensitiveConfig,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_sensitive_config",
        encode(&SensitiveConfigBody { config })?,
    )
    .await?;
    Ok(())
}

/// `get_all_chat_hubs` — all ChatHub entries.
#[cfg(feature = "desktop")]
pub async fn get_all_chat_hubs(supervisor: &RuntimeSupervisor) -> Result<Vec<ChatHub>, String> {
    decode(read_command(supervisor, "get_all_chat_hubs", json!({})).await?)
}

/// `add_chat_hub` — creates a ChatHub entry.
#[cfg(feature = "desktop")]
pub async fn add_chat_hub(
    supervisor: &RuntimeSupervisor,
    body: ChatHubAddBody,
) -> Result<ChatHub, String> {
    decode(mutate_command(supervisor, "add_chat_hub", encode(&body)?).await?)
}

/// `update_chat_hub` — updates a ChatHub entry.
#[cfg(feature = "desktop")]
pub async fn update_chat_hub(
    supervisor: &RuntimeSupervisor,
    body: ChatHubUpdateBody,
) -> Result<ChatHub, String> {
    decode(mutate_command(supervisor, "update_chat_hub", encode(&body)?).await?)
}

/// `delete_chat_hub` — removes a ChatHub entry.
#[cfg(feature = "desktop")]
pub async fn delete_chat_hub(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_chat_hub", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `update_chat_hub_order` — persists the ChatHub order.
#[cfg(feature = "desktop")]
pub async fn update_chat_hub_order(
    supervisor: &RuntimeSupervisor,
    hub_ids: Vec<i64>,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_chat_hub_order",
        encode(&ChatHubOrderBody { hub_ids })?,
    )
    .await?;
    Ok(())
}

/// `delete_ccproxy_stats` — deletes telemetry older than the window.
#[cfg(feature = "desktop")]
pub async fn delete_ccproxy_stats(supervisor: &RuntimeSupervisor, days: i32) -> Result<(), String> {
    mutate_command(
        supervisor,
        "delete_ccproxy_stats",
        encode(&DaysBody { days })?,
    )
    .await?;
    Ok(())
}

/// `get_ccproxy_daily_stats` — daily ccproxy aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_daily_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_daily_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `get_ccproxy_grouped_stats` — grouped ccproxy aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_grouped_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_grouped_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `get_ccproxy_grouped_stats_by_date_range` — grouped aggregates by range.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_grouped_stats_by_date_range(
    supervisor: &RuntimeSupervisor,
    start_date: String,
    end_date: String,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_grouped_stats_by_date_range",
            encode(&DateRangeBody {
                start_date,
                end_date,
            })?,
        )
        .await?,
    )
}

/// `get_ccproxy_today_cost_stats` — today's ccproxy cost aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_today_cost_stats(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<Value>, String> {
    decode(read_command(supervisor, "get_ccproxy_today_cost_stats", json!({})).await?)
}

/// `get_ccproxy_provider_stats_by_date` — provider aggregates for one date.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_provider_stats_by_date(
    supervisor: &RuntimeSupervisor,
    date: String,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_provider_stats_by_date",
            encode(&DateBody { date })?,
        )
        .await?,
    )
}

/// `get_ccproxy_error_stats_by_date` — error aggregates for one date.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_error_stats_by_date(
    supervisor: &RuntimeSupervisor,
    date: String,
    client_model: Option<String>,
    backend_model: Option<String>,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_error_stats_by_date",
            encode(&ErrorStatsBody {
                date,
                client_model,
                backend_model,
            })?,
        )
        .await?,
    )
}

/// `get_ccproxy_model_usage_stats` — model usage aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_model_usage_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_model_usage_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `get_ccproxy_model_token_usage_stats` — model token aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_model_token_usage_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_model_token_usage_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `get_ccproxy_error_distribution_stats` — error distribution aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_error_distribution_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_error_distribution_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `get_ccproxy_provider_token_usage_stats` — provider token aggregates.
#[cfg(feature = "desktop")]
pub async fn get_ccproxy_provider_token_usage_stats(
    supervisor: &RuntimeSupervisor,
    days: i32,
) -> Result<Vec<Value>, String> {
    decode(
        read_command(
            supervisor,
            "get_ccproxy_provider_token_usage_stats",
            encode(&DaysBody { days })?,
        )
        .await?,
    )
}

/// `export_config_package` — writes a configuration package to a chosen path.
#[cfg(feature = "desktop")]
pub async fn export_config_package(
    supervisor: &RuntimeSupervisor,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<crate::db::config_transfer::ConfigTransferPreview, String> {
    decode(
        mutate_command(
            supervisor,
            "export_config_package",
            encode(&ConfigTransferBody { path, categories })?,
        )
        .await?,
    )
}

/// `import_config_package` — imports a configuration package from a chosen path.
#[cfg(feature = "desktop")]
pub async fn import_config_package(
    supervisor: &RuntimeSupervisor,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<crate::db::config_transfer::ConfigImportResult, String> {
    decode(
        mutate_command(
            supervisor,
            "import_config_package",
            encode(&ConfigTransferBody { path, categories })?,
        )
        .await?,
    )
}

/// `get_all_config` — the runtime configuration map.
#[cfg(feature = "desktop")]
pub async fn get_all_config(supervisor: &RuntimeSupervisor) -> Result<Value, String> {
    read_command(supervisor, "get_all_config", json!({})).await
}

/// `set_config` — sets or deletes one configuration key.
#[cfg(feature = "desktop")]
pub async fn set_config(
    supervisor: &RuntimeSupervisor,
    key: String,
    value: Value,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "set_config",
        encode(&SetConfigBody { key, value })?,
    )
    .await?;
    Ok(())
}

/// `reload_config` — reloads the configuration cache from the database.
#[cfg(feature = "desktop")]
pub async fn reload_config(supervisor: &RuntimeSupervisor) -> Result<(), String> {
    mutate_command(supervisor, "reload_config", json!({})).await?;
    Ok(())
}

/// `get_api_key_encryption_status` — the API-key encryption status.
#[cfg(feature = "desktop")]
pub async fn get_api_key_encryption_status(
    supervisor: &RuntimeSupervisor,
) -> Result<Value, String> {
    read_command(supervisor, "get_api_key_encryption_status", json!({})).await
}

/// `activate_api_key_file` — activates a chosen key file and returns the status.
#[cfg(feature = "desktop")]
pub async fn activate_api_key_file(
    supervisor: &RuntimeSupervisor,
    path: String,
) -> Result<Value, String> {
    mutate_command(
        supervisor,
        "activate_api_key_file",
        encode(&PathBody { path })?,
    )
    .await
}

/// `generate_api_key_file` — generates and activates a key file at a chosen path.
#[cfg(feature = "desktop")]
pub async fn generate_api_key_file(
    supervisor: &RuntimeSupervisor,
    path: String,
) -> Result<Value, String> {
    mutate_command(
        supervisor,
        "generate_api_key_file",
        encode(&PathBody { path })?,
    )
    .await
}

/// `get_ai_model_by_id` — one AI model.
#[cfg(feature = "desktop")]
pub async fn get_ai_model_by_id(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> Result<crate::db::AiModel, String> {
    decode(read_command(supervisor, "get_ai_model_by_id", encode(&IdBody { id })?).await?)
}

/// `get_all_ai_models` — all AI models.
#[cfg(feature = "desktop")]
pub async fn get_all_ai_models(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<crate::db::AiModel>, String> {
    decode(read_command(supervisor, "get_all_ai_models", json!({})).await?)
}

/// `add_ai_model` — creates an AI model and returns the stored record.
#[cfg(feature = "desktop")]
pub async fn add_ai_model(
    supervisor: &RuntimeSupervisor,
    body: AddAiModelBody,
) -> Result<crate::db::AiModel, String> {
    decode(mutate_command(supervisor, "add_ai_model", encode(&body)?).await?)
}

/// `update_ai_model` — updates an AI model and returns the stored record.
#[cfg(feature = "desktop")]
pub async fn update_ai_model(
    supervisor: &RuntimeSupervisor,
    body: UpdateAiModelBody,
) -> Result<crate::db::AiModel, String> {
    decode(mutate_command(supervisor, "update_ai_model", encode(&body)?).await?)
}

/// `update_ai_model_order` — persists the AI model order.
#[cfg(feature = "desktop")]
pub async fn update_ai_model_order(
    supervisor: &RuntimeSupervisor,
    model_ids: Vec<i64>,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_ai_model_order",
        encode(&ModelOrderBody { model_ids })?,
    )
    .await?;
    Ok(())
}

/// `delete_ai_model` — removes an AI model.
#[cfg(feature = "desktop")]
pub async fn delete_ai_model(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_ai_model", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `get_ai_skill_by_id` — one AI skill.
#[cfg(feature = "desktop")]
pub async fn get_ai_skill_by_id(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> Result<crate::db::AiSkill, String> {
    decode(read_command(supervisor, "get_ai_skill_by_id", encode(&IdBody { id })?).await?)
}

/// `get_all_ai_skills` — all AI skills.
#[cfg(feature = "desktop")]
pub async fn get_all_ai_skills(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<crate::db::AiSkill>, String> {
    decode(read_command(supervisor, "get_all_ai_skills", json!({})).await?)
}

/// `add_ai_skill` — creates an AI skill (logo already uploaded) and returns it.
#[cfg(feature = "desktop")]
pub async fn add_ai_skill(
    supervisor: &RuntimeSupervisor,
    body: AddAiSkillBody,
) -> Result<crate::db::AiSkill, String> {
    decode(mutate_command(supervisor, "add_ai_skill", encode(&body)?).await?)
}

/// `update_ai_skill` — updates an AI skill and returns the stored record.
#[cfg(feature = "desktop")]
pub async fn update_ai_skill(
    supervisor: &RuntimeSupervisor,
    body: UpdateAiSkillBody,
) -> Result<crate::db::AiSkill, String> {
    decode(mutate_command(supervisor, "update_ai_skill", encode(&body)?).await?)
}

/// `update_ai_skill_order` — persists the AI skill order.
#[cfg(feature = "desktop")]
pub async fn update_ai_skill_order(
    supervisor: &RuntimeSupervisor,
    skill_ids: Vec<i64>,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "update_ai_skill_order",
        encode(&SkillOrderBody { skill_ids })?,
    )
    .await?;
    Ok(())
}

/// `delete_ai_skill` — removes an AI skill.
#[cfg(feature = "desktop")]
pub async fn delete_ai_skill(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_ai_skill", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `backup_setting` — flushes and writes a full backup.
#[cfg(feature = "desktop")]
pub async fn backup_setting(
    supervisor: &RuntimeSupervisor,
    backup_dir: Option<String>,
) -> Result<(), String> {
    mutate_command(
        supervisor,
        "backup_setting",
        encode(&BackupBody { backup_dir })?,
    )
    .await?;
    Ok(())
}

/// `restore_setting` — restores a full backup.
#[cfg(feature = "desktop")]
pub async fn restore_setting(
    supervisor: &RuntimeSupervisor,
    backup_dir: String,
) -> Result<RestoreSettingResponse, String> {
    decode(
        mutate_command(
            supervisor,
            "restore_setting",
            encode(&RestoreBody { backup_dir })?,
        )
        .await?,
    )
}

/// `get_all_backups` — lists available backups.
#[cfg(feature = "desktop")]
pub async fn get_all_backups(
    supervisor: &RuntimeSupervisor,
    backup_dir: Option<String>,
) -> Result<Vec<String>, String> {
    decode(
        read_command(
            supervisor,
            "get_all_backups",
            encode(&BackupBody { backup_dir })?,
        )
        .await?,
    )
}

/// `get_models_dev_providers` — the runtime-owned provider presets.
#[cfg(feature = "desktop")]
pub async fn list_models_dev_providers(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<ModelsDevPresetProviderDto>, String> {
    let client = control_plane_client(supervisor).await?;
    client
        .models_dev_providers()
        .await
        .map_err(map_client_error)
}

/// `get_models_dev_provider_models` — one provider's embedded catalog models.
#[cfg(feature = "desktop")]
pub async fn list_models_dev_provider_models(
    supervisor: &RuntimeSupervisor,
    provider_id: String,
) -> Result<Vec<ModelDetails>, String> {
    let client = control_plane_client(supervisor).await?;
    let models = client
        .models_dev_provider_models(&ModelsDevProviderModelsRequest { provider_id })
        .await
        .map_err(map_client_error)?;
    models.into_iter().map(model_details_from_wire).collect()
}

/// `resolve_model_profile` — resolves a profile against the runtime snapshot.
#[cfg(feature = "desktop")]
pub async fn resolve_model_profile(
    supervisor: &RuntimeSupervisor,
    request: ResolveModelProfileRequest,
) -> Result<ResolvedModelProfile, String> {
    let client = control_plane_client(supervisor).await?;
    let value = client
        .resolve_model_profile(&request)
        .await
        .map_err(map_client_error)?;
    decode(value)
}

/// Decodes the runtime wire model descriptor back into the desktop domain type.
#[cfg(feature = "desktop")]
fn model_details_from_wire(model: ModelDetailsDto) -> Result<ModelDetails, String> {
    let value = serde_json::to_value(&model).map_err(|error| error.to_string())?;
    decode(value)
}

// The data-command cores are runtime-only, so their tests compile only in the
// desktop-free runtime backend.
#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn allowlist_covers_known_commands_and_rejects_unknown() {
        assert_eq!(
            data_command_kind("get_available_tools"),
            Some(DataCommandKind::Read)
        );
        assert_eq!(
            data_command_kind("update_agent_order"),
            Some(DataCommandKind::Mutation)
        );
        assert_eq!(
            data_command_kind("add_message"),
            Some(DataCommandKind::Mutation)
        );
        assert_eq!(
            data_command_kind("get_all_backups"),
            Some(DataCommandKind::Read)
        );
        assert_eq!(
            data_command_kind("get_models_dev_providers"),
            Some(DataCommandKind::Read)
        );
        assert_eq!(
            data_command_kind("get_models_dev_provider_models"),
            Some(DataCommandKind::Read)
        );
        assert_eq!(
            data_command_kind("resolve_model_profile"),
            Some(DataCommandKind::Read)
        );
        assert_eq!(data_command_kind("not_a_command"), None);
    }

    #[test]
    fn embedded_catalog_provider_models_keep_the_canonical_shape() {
        let dir = tempdir().expect("temporary directory");
        let service = crate::model_catalog_service::ModelsDevCatalogService::load(dir.path())
            .expect("catalog service");
        let snapshot = service.snapshot();
        let (provider_id, provider) = snapshot
            .providers
            .iter()
            .find(|(_, provider)| !provider.models.is_empty())
            .expect("a provider with models");
        let expected_id = provider.models.keys().next().expect("a model id").clone();

        let models = provider_models_from_catalog(snapshot.as_ref(), provider_id);
        assert_eq!(models.len(), provider.models.len());

        let model = models
            .iter()
            .find(|model| model.id == expected_id)
            .expect("the provider model is present");
        let value = serde_json::to_value(model).expect("serialize");
        assert_eq!(value["id"], json!(expected_id));
        assert_eq!(value["protocol"], json!("OpenAI"));
        // The wire stays camelCase and omits optionals the core never fills.
        assert!(value.get("functionCall").is_some());
        assert!(value.get("recommendedTemperature").is_none());
    }

    #[tokio::test]
    async fn unknown_command_is_rejected() {
        let dir = tempdir().unwrap();
        let store = MainStore::new(dir.path().join("runtime_data_dispatch.db")).unwrap();
        // Building a full service is out of scope for this focused test; the
        // allowlist gate is verified through the public kind table above, and
        // the body parser is verified directly below.
        drop(store);
        assert!(data_command_kind("bogus").is_none());
    }

    #[test]
    fn typed_bodies_reject_unknown_fields() {
        let error = parse_body::<GetNotesBody>(json!({ "tag_id": 1, "unexpected": true }))
            .expect_err("unknown field must be rejected");
        assert!(matches!(
            error.kind,
            crate::workflow::react::application::ApplicationErrorKind::InvalidInput
        ));

        let parsed = parse_body::<GetNotesBody>(json!({ "tag_id": null }))
            .expect("null optional tag id is valid");
        assert_eq!(parsed.tag_id, None);
    }

    #[test]
    fn git_review_metadata_is_child_only_and_unique() {
        let metadata = git_review_tool_metadata();
        assert_eq!(metadata.len(), 2);
        for id in [crate::tools::TOOL_GIT_DIFF, crate::tools::TOOL_GIT_INSPECT] {
            let matches = metadata
                .iter()
                .filter(|tool| tool["id"].as_str() == Some(id))
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{id} metadata should appear once");
            assert_eq!(matches[0]["child_only"].as_bool(), Some(true));
            assert_eq!(matches[0]["scope"].as_str(), Some("workflow"));
        }
    }

    #[test]
    fn assigns_tsid_to_new_scheme_items_without_client_supplied_ids() {
        use crate::tools::{
            HostCommandRule, SandboxNetworkPolicy, SandboxProfileConfig, SandboxSchemeConfig,
            WorkspaceAccess,
        };
        let generator = crate::libs::tsid::TsidGenerator::new(1).expect("create TSID generator");
        let mut scheme = SandboxScheme {
            id: "scheme".to_string(),
            name: "Scheme".to_string(),
            description: String::new(),
            config: SandboxSchemeConfig {
                runtime_preference: Default::default(),
                profiles: vec![SandboxProfileConfig {
                    id: String::new(),
                    name: "Bash".to_string(),
                    enabled: true,
                    priority: 0,
                    command_patterns: vec!["^bash(?:\\s|$)".to_string()],
                    runtime_preference: Default::default(),
                    image: "bash:latest".to_string(),
                    instance_name: None,
                    image_size_bytes: None,
                    network: SandboxNetworkPolicy::default(),
                    resources: Default::default(),
                    workspace_access: WorkspaceAccess::ReadWrite,
                }],
                host_rules: vec![HostCommandRule {
                    id: String::new(),
                    name: "Tauri Host".to_string(),
                    enabled: true,
                    priority: 10,
                    command_patterns: vec![
                        "^(?:pnpm|npm|yarn|npx)(?:\\s+run)?\\s+tauri(?:\\s|$)".to_string()
                    ],
                }],
            },
            disabled: false,
            created_at: None,
            updated_at: None,
        };

        assign_missing_scheme_item_ids(&mut scheme, &generator).expect("assign scheme item IDs");

        assert_eq!(scheme.config.profiles[0].id.len(), 13);
        assert_eq!(scheme.config.host_rules[0].id.len(), 13);
        scheme
            .validate()
            .expect("generated IDs satisfy scheme validation");
    }

    #[test]
    fn restore_preserves_local_ccproxy_binding_settings() {
        let temp_dir = tempdir().unwrap();
        let main_path = temp_dir.path().join("main.db");
        let staged_path = temp_dir.path().join("staged.db");
        let main_store = MainStore::new(&main_path).unwrap();
        main_store
            .set_config(crate::constants::CFG_CCPROXY_PORT, &json!(11436))
            .unwrap();
        main_store
            .set_config(crate::constants::CFG_CCPROXY_LISTEN, &json!("127.0.0.1"))
            .unwrap();
        main_store
            .set_config("restore_test", &json!("local-value"))
            .unwrap();

        let staged_store = MainStore::new(&staged_path).unwrap();
        staged_store
            .set_config(crate::constants::CFG_CCPROXY_PORT, &json!(11435))
            .unwrap();
        staged_store
            .set_config(crate::constants::CFG_CCPROXY_LISTEN, &json!("0.0.0.0"))
            .unwrap();
        staged_store
            .set_config("restore_test", &json!("remote-value"))
            .unwrap();
        drop(staged_store);

        main_store
            .atomic_restore(&staged_path, &main_path, MACHINE_SPECIFIC_CONFIG_KEYS)
            .unwrap();

        assert_eq!(
            main_store.get_config(crate::constants::CFG_CCPROXY_PORT, 0),
            11436
        );
        assert_eq!(
            main_store.get_config(crate::constants::CFG_CCPROXY_LISTEN, String::new()),
            "127.0.0.1"
        );
        assert_eq!(
            main_store.get_config("restore_test", String::new()),
            "remote-value"
        );
    }
}
