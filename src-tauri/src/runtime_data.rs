//! Desktop transport adapters for the runtime-owned data commands.
//!
//! The runtime owns the data (agents, proxy groups, notes, conversations,
//! sandbox schemes, the sensitive-filter configuration, ChatHubs, ccproxy
//! statistics and the configuration/model/skill/backup settings). These helpers
//! translate each desktop command's wire into one
//! `/control/v1/data-commands/{command}` call and decode the reply back into the
//! historical Tauri shape. The transport-neutral cores, the command allowlist
//! and the typed request bodies live in the runtime backend
//! (`chatspeed_runtime_backend::data::runtime_data`) and are re-exported here so
//! both sides keep one wire contract.

pub use chatspeed_runtime_backend::data::runtime_data::*;

use crate::ai::model_catalog::ResolvedModelProfile;
use crate::ai::traits::chat::ModelDetails;
use crate::db::config_transfer::ConfigCategory;
use crate::db::{ChatHub, Conversation, Note, NoteTag, ProxyGroup, SandboxScheme};
use crate::runtime_client::RuntimeSupervisor;
use crate::sensitive::manager::SensitiveConfig;
use chatspeed_contracts::{
    ModelDetailsDto, ModelsDevPresetProviderDto, ModelsDevProviderModelsRequest,
    ResolveModelProfileRequest,
};
use chatspeed_runtime_client::{ClientError, RuntimeClient};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

/// Canonical control-plane route for data commands.
const DATA_COMMAND_ROUTE: &str = chatspeed_runtime_client::DATA_COMMANDS_PATH;

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(|error| error.to_string())
}

/// Fresh idempotency key for one mutating command invocation.
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Maps a transport/domain client error to the Tauri string error.
fn map_client_error(error: ClientError) -> String {
    match error {
        ClientError::Server { message, .. } => message,
        other => other.to_string(),
    }
}

/// Encodes a typed request body for the wire.
fn encode<T: Serialize>(body: &T) -> Result<Value, String> {
    serde_json::to_value(body).map_err(|error| error.to_string())
}

/// Decodes a runtime response back into its original Tauri-wire type.
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Sends a read-only data command.
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
pub async fn get_available_tools(supervisor: &RuntimeSupervisor) -> Result<Value, String> {
    read_command(supervisor, "get_available_tools", json!({})).await
}

/// `update_agent_order` — persists the agent sort order.
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
pub async fn proxy_group_list(supervisor: &RuntimeSupervisor) -> Result<Vec<ProxyGroup>, String> {
    decode(read_command(supervisor, "proxy_group_list", json!({})).await?)
}

/// `proxy_group_add` — inserts one proxy group and returns its id.
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
#[allow(clippy::too_many_arguments)]
pub async fn proxy_group_batch_update(
    supervisor: &RuntimeSupervisor,
    body: ProxyGroupBatchUpdateBody,
) -> Result<(), String> {
    mutate_command(supervisor, "proxy_group_batch_update", encode(&body)?).await?;
    Ok(())
}

/// `proxy_group_delete` — removes one proxy group.
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
pub async fn get_active_proxy_group(supervisor: &RuntimeSupervisor) -> Result<String, String> {
    decode(read_command(supervisor, "get_active_proxy_group", json!({})).await?)
}

/// `set_active_proxy_group` — sets the active proxy group name.
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
pub async fn add_note(supervisor: &RuntimeSupervisor, body: AddNoteBody) -> Result<(), String> {
    mutate_command(supervisor, "add_note", encode(&body)?).await?;
    Ok(())
}

/// `get_tags` — all note tags.
pub async fn get_tags(supervisor: &RuntimeSupervisor) -> Result<Vec<NoteTag>, String> {
    decode(read_command(supervisor, "get_tags", json!({})).await?)
}

/// `get_notes` — notes, optionally filtered by tag.
pub async fn get_notes(
    supervisor: &RuntimeSupervisor,
    tag_id: Option<i64>,
) -> Result<Vec<Note>, String> {
    decode(read_command(supervisor, "get_notes", encode(&GetNotesBody { tag_id })?).await?)
}

/// `get_note` — one note by id.
pub async fn get_note(supervisor: &RuntimeSupervisor, id: i64) -> Result<Note, String> {
    decode(read_command(supervisor, "get_note", encode(&IdBody { id })?).await?)
}

/// `search_notes` — notes matching a keyword.
pub async fn search_notes(supervisor: &RuntimeSupervisor, kw: String) -> Result<Vec<Note>, String> {
    decode(read_command(supervisor, "search_notes", encode(&SearchNotesBody { kw })?).await?)
}

/// `delete_note` — removes a note.
pub async fn delete_note(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_note", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `get_all_conversations` — all conversations.
pub async fn get_all_conversations(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<Conversation>, String> {
    decode(read_command(supervisor, "get_all_conversations", json!({})).await?)
}

/// `get_conversation_by_id` — one conversation.
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
pub async fn update_conversation(
    supervisor: &RuntimeSupervisor,
    body: UpdateConversationBody,
) -> Result<(), String> {
    mutate_command(supervisor, "update_conversation", encode(&body)?).await?;
    Ok(())
}

/// `delete_conversation` — removes a conversation.
pub async fn delete_conversation(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_conversation", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `add_message` — stores a message after the runtime sensitive filter and
/// returns `(id, final_content)`.
pub async fn add_message(
    supervisor: &RuntimeSupervisor,
    body: AddMessageBody,
) -> Result<(i64, String), String> {
    decode(mutate_command(supervisor, "add_message", encode(&body)?).await?)
}

/// `delete_message` — removes one or more messages.
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
pub async fn update_message_metadata(
    supervisor: &RuntimeSupervisor,
    body: UpdateMessageMetadataBody,
) -> Result<(), String> {
    mutate_command(supervisor, "update_message_metadata", encode(&body)?).await?;
    Ok(())
}

/// `get_sandbox_schemes` — all sandbox schemes.
pub async fn get_sandbox_schemes(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<SandboxScheme>, String> {
    decode(read_command(supervisor, "get_sandbox_schemes", json!({})).await?)
}

/// `add_sandbox_scheme` — creates a sandbox scheme and returns its id.
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
pub async fn get_sensitive_config(
    supervisor: &RuntimeSupervisor,
) -> Result<SensitiveConfig, String> {
    decode(read_command(supervisor, "get_sensitive_config", json!({})).await?)
}

/// `update_sensitive_config` — replaces the sensitive-filter configuration.
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
pub async fn get_all_chat_hubs(supervisor: &RuntimeSupervisor) -> Result<Vec<ChatHub>, String> {
    decode(read_command(supervisor, "get_all_chat_hubs", json!({})).await?)
}

/// `add_chat_hub` — creates a ChatHub entry.
pub async fn add_chat_hub(
    supervisor: &RuntimeSupervisor,
    body: ChatHubAddBody,
) -> Result<ChatHub, String> {
    decode(mutate_command(supervisor, "add_chat_hub", encode(&body)?).await?)
}

/// `update_chat_hub` — updates a ChatHub entry.
pub async fn update_chat_hub(
    supervisor: &RuntimeSupervisor,
    body: ChatHubUpdateBody,
) -> Result<ChatHub, String> {
    decode(mutate_command(supervisor, "update_chat_hub", encode(&body)?).await?)
}

/// `delete_chat_hub` — removes a ChatHub entry.
pub async fn delete_chat_hub(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_chat_hub", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `update_chat_hub_order` — persists the ChatHub order.
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
pub async fn get_ccproxy_today_cost_stats(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<Value>, String> {
    decode(read_command(supervisor, "get_ccproxy_today_cost_stats", json!({})).await?)
}

/// `get_ccproxy_provider_stats_by_date` — provider aggregates for one date.
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
pub async fn get_all_config(supervisor: &RuntimeSupervisor) -> Result<Value, String> {
    read_command(supervisor, "get_all_config", json!({})).await
}

/// `set_config` — sets or deletes one configuration key.
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
pub async fn reload_config(supervisor: &RuntimeSupervisor) -> Result<(), String> {
    mutate_command(supervisor, "reload_config", json!({})).await?;
    Ok(())
}

/// `get_api_key_encryption_status` — the API-key encryption status.
pub async fn get_api_key_encryption_status(
    supervisor: &RuntimeSupervisor,
) -> Result<Value, String> {
    read_command(supervisor, "get_api_key_encryption_status", json!({})).await
}

/// `activate_api_key_file` — activates a chosen key file and returns the status.
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
pub async fn get_ai_model_by_id(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> Result<crate::db::AiModel, String> {
    decode(read_command(supervisor, "get_ai_model_by_id", encode(&IdBody { id })?).await?)
}

/// `get_all_ai_models` — all AI models.
pub async fn get_all_ai_models(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<crate::db::AiModel>, String> {
    decode(read_command(supervisor, "get_all_ai_models", json!({})).await?)
}

/// `add_ai_model` — creates an AI model and returns the stored record.
pub async fn add_ai_model(
    supervisor: &RuntimeSupervisor,
    body: AddAiModelBody,
) -> Result<crate::db::AiModel, String> {
    decode(mutate_command(supervisor, "add_ai_model", encode(&body)?).await?)
}

/// `update_ai_model` — updates an AI model and returns the stored record.
pub async fn update_ai_model(
    supervisor: &RuntimeSupervisor,
    body: UpdateAiModelBody,
) -> Result<crate::db::AiModel, String> {
    decode(mutate_command(supervisor, "update_ai_model", encode(&body)?).await?)
}

/// `update_ai_model_order` — persists the AI model order.
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
pub async fn delete_ai_model(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_ai_model", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `get_ai_skill_by_id` — one AI skill.
pub async fn get_ai_skill_by_id(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> Result<crate::db::AiSkill, String> {
    decode(read_command(supervisor, "get_ai_skill_by_id", encode(&IdBody { id })?).await?)
}

/// `get_all_ai_skills` — all AI skills.
pub async fn get_all_ai_skills(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<crate::db::AiSkill>, String> {
    decode(read_command(supervisor, "get_all_ai_skills", json!({})).await?)
}

/// `add_ai_skill` — creates an AI skill (logo already uploaded) and returns it.
pub async fn add_ai_skill(
    supervisor: &RuntimeSupervisor,
    body: AddAiSkillBody,
) -> Result<crate::db::AiSkill, String> {
    decode(mutate_command(supervisor, "add_ai_skill", encode(&body)?).await?)
}

/// `update_ai_skill` — updates an AI skill and returns the stored record.
pub async fn update_ai_skill(
    supervisor: &RuntimeSupervisor,
    body: UpdateAiSkillBody,
) -> Result<crate::db::AiSkill, String> {
    decode(mutate_command(supervisor, "update_ai_skill", encode(&body)?).await?)
}

/// `update_ai_skill_order` — persists the AI skill order.
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
pub async fn delete_ai_skill(supervisor: &RuntimeSupervisor, id: i64) -> Result<(), String> {
    mutate_command(supervisor, "delete_ai_skill", encode(&IdBody { id })?).await?;
    Ok(())
}

/// `backup_setting` — flushes and writes a full backup.
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
fn model_details_from_wire(model: ModelDetailsDto) -> Result<ModelDetails, String> {
    let value = serde_json::to_value(&model).map_err(|error| error.to_string())?;
    decode(value)
}
