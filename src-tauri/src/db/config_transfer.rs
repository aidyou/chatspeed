//! Versioned, portable configuration package support.
//!
//! This module deliberately keeps configuration transfer separate from full database backups.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

use chrono::Utc;
use rusqlite::{
    params,
    types::{Value as SqlValue, ValueRef},
    Connection, Transaction,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::{db::agent::AgentModels, tools::MCP_TOOL_NAME_SPLIT};

use super::StoreError;

const FORMAT_VERSION: u32 = 1;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub const AI_CONFIG_KEYS: &[&str] = &[
    "conversation_title_gen_model",
    "vision_model",
    "websearch_model",
];

pub const PROXY_CONFIG_KEYS: &[&str] = &[
    "chat_completion_proxy",
    "chat_completion_proxy_keys",
    "active_proxy_group",
    "chat_completion_proxy_port",
    "chat_completion_proxy_listen",
    "chat_completion_proxy_log_to_file",
    "chat_completion_proxy_log_proxy_to_file",
    "chat_completion_proxy_retry_on_429",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigCategory {
    AiModels,
    Skills,
    Mcp,
    Proxy,
    Agents,
    Sandbox,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigTransferPackage {
    pub format_version: u32,
    pub exported_at: String,
    pub categories: Vec<ConfigCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_models: Option<TablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<TablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<TablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<TablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<TablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<TablePayload>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TablePayload {
    pub rows: Vec<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigTransferPreview {
    pub format_version: u32,
    pub categories: Vec<ConfigCategory>,
    pub counts: BTreeMap<ConfigCategory, usize>,
    pub contains_encrypted_api_keys: bool,
    pub contains_proxy_tokens: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigImportResult {
    pub imported_counts: BTreeMap<ConfigCategory, usize>,
    pub preserved_agents: usize,
    pub preserved_sandbox_schemes: usize,
    pub api_keys_locked: bool,
    pub restart_recommended: bool,
}

pub fn category_closure(
    categories: impl IntoIterator<Item = ConfigCategory>,
) -> BTreeSet<ConfigCategory> {
    let mut categories: BTreeSet<_> = categories.into_iter().collect();
    if categories.contains(&ConfigCategory::Proxy) {
        categories.insert(ConfigCategory::AiModels);
    }
    if categories.contains(&ConfigCategory::Agents) {
        categories.extend([
            ConfigCategory::AiModels,
            ConfigCategory::Skills,
            ConfigCategory::Mcp,
            ConfigCategory::Sandbox,
        ]);
    }
    categories
}

pub fn export_config_package(
    conn: &Connection,
    path: impl AsRef<Path>,
    categories: impl IntoIterator<Item = ConfigCategory>,
) -> Result<ConfigTransferPreview, StoreError> {
    let categories = category_closure(categories);
    if categories.is_empty() {
        return Err(StoreError::InvalidData(
            "at least one configuration category is required".into(),
        ));
    }

    let mut package = ConfigTransferPackage {
        format_version: FORMAT_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        categories: categories.iter().copied().collect(),
        ai_models: None,
        skills: None,
        mcp: None,
        proxy: None,
        agents: None,
        sandbox: None,
    };

    if categories.contains(&ConfigCategory::AiModels) {
        package.ai_models = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM ai_model ORDER BY id")?,
            config: read_config(conn, AI_CONFIG_KEYS)?,
        });
    }
    if categories.contains(&ConfigCategory::Skills) {
        package.skills = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM ai_skill ORDER BY id")?,
            config: BTreeMap::new(),
        });
    }
    if categories.contains(&ConfigCategory::Mcp) {
        package.mcp = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM mcp ORDER BY id")?,
            config: BTreeMap::new(),
        });
    }
    if categories.contains(&ConfigCategory::Proxy) {
        package.proxy = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM proxy_group ORDER BY id")?,
            config: read_config(conn, PROXY_CONFIG_KEYS)?,
        });
    }
    if categories.contains(&ConfigCategory::Agents) {
        package.agents = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM agents ORDER BY id")?,
            config: BTreeMap::new(),
        });
    }
    if categories.contains(&ConfigCategory::Sandbox) {
        package.sandbox = Some(TablePayload {
            rows: read_rows(conn, "SELECT * FROM sandbox_schemes ORDER BY id")?,
            config: BTreeMap::new(),
        });
    }

    let preview = validate_package(&package)?;
    write_atomic(path.as_ref(), &serde_json::to_vec_pretty(&package)?)?;
    Ok(preview)
}

pub fn inspect_config_package(path: impl AsRef<Path>) -> Result<ConfigTransferPreview, StoreError> {
    let bytes = fs::read(path)?;
    let package: ConfigTransferPackage = serde_json::from_slice(&bytes)?;
    validate_package(&package)
}

pub fn read_config_package(path: impl AsRef<Path>) -> Result<ConfigTransferPackage, StoreError> {
    let package: ConfigTransferPackage = serde_json::from_slice(&fs::read(path)?)?;
    validate_package(&package)?;
    Ok(package)
}

pub fn validate_package(
    package: &ConfigTransferPackage,
) -> Result<ConfigTransferPreview, StoreError> {
    if package.format_version != FORMAT_VERSION {
        return Err(StoreError::InvalidData(format!(
            "unsupported configuration package format version: {}",
            package.format_version
        )));
    }
    let categories = category_closure(package.categories.iter().copied());
    if categories.is_empty() || categories.len() != package.categories.len() {
        return Err(StoreError::InvalidData(
            "configuration categories must be non-empty and include required dependencies".into(),
        ));
    }

    let payloads = [
        (ConfigCategory::AiModels, &package.ai_models),
        (ConfigCategory::Skills, &package.skills),
        (ConfigCategory::Mcp, &package.mcp),
        (ConfigCategory::Proxy, &package.proxy),
        (ConfigCategory::Agents, &package.agents),
        (ConfigCategory::Sandbox, &package.sandbox),
    ];
    let mut counts = BTreeMap::new();
    for (category, payload) in payloads {
        match (categories.contains(&category), payload) {
            (true, Some(payload)) => {
                validate_rows(category, &payload.rows)?;
                counts.insert(category, payload.rows.len());
            }
            (true, None) => {
                return Err(StoreError::InvalidData(format!(
                    "package is missing required {category:?} payload"
                )))
            }
            (false, Some(_)) => {
                return Err(StoreError::InvalidData(format!(
                    "package contains unselected {category:?} payload"
                )))
            }
            (false, None) => {}
        }
    }
    validate_config_keys(package.ai_models.as_ref(), AI_CONFIG_KEYS)?;
    validate_config_keys(package.proxy.as_ref(), PROXY_CONFIG_KEYS)?;
    validate_package_references(package, &categories)?;

    Ok(ConfigTransferPreview {
        format_version: package.format_version,
        categories: package.categories.clone(),
        counts,
        contains_encrypted_api_keys: package
            .ai_models
            .as_ref()
            .is_some_and(|payload| !payload.rows.is_empty()),
        contains_proxy_tokens: package
            .proxy
            .as_ref()
            .is_some_and(|payload| payload.config.contains_key("chat_completion_proxy_keys")),
    })
}

fn validate_config_keys(payload: Option<&TablePayload>, keys: &[&str]) -> Result<(), StoreError> {
    let Some(payload) = payload else {
        return Ok(());
    };
    if payload
        .config
        .keys()
        .any(|key| !keys.contains(&key.as_str()))
    {
        return Err(StoreError::InvalidData(
            "package has an unsupported config key".into(),
        ));
    }
    for (key, value) in &payload.config {
        match key.as_str() {
            "conversation_title_gen_model" | "vision_model" | "websearch_model" => {
                validate_model_selection(value, key)?;
            }
            "chat_completion_proxy" => validate_proxy_targets(value)?,
            "chat_completion_proxy_keys" => validate_proxy_keys(value)?,
            "active_proxy_group" => {
                if !value.is_null() && !value.is_string() {
                    return Err(StoreError::InvalidData(
                        "active proxy group must be a string".into(),
                    ));
                }
            }
            "chat_completion_proxy_port" => {
                if value.as_i64().is_none() {
                    return Err(StoreError::InvalidData(
                        "proxy port must be an integer".into(),
                    ));
                }
            }
            "chat_completion_proxy_listen" => {
                if !value.is_string() {
                    return Err(StoreError::InvalidData(
                        "proxy listen address must be a string".into(),
                    ));
                }
            }
            "chat_completion_proxy_log_to_file" | "chat_completion_proxy_log_proxy_to_file" => {
                if !value.is_boolean() {
                    return Err(StoreError::InvalidData(
                        "proxy logging setting must be a boolean".into(),
                    ));
                }
            }
            "chat_completion_proxy_retry_on_429" => {
                if value.as_i64().is_none() {
                    return Err(StoreError::InvalidData(
                        "proxy retry setting must be an integer".into(),
                    ));
                }
            }
            _ => unreachable!("config keys are whitelisted above"),
        }
    }
    Ok(())
}

fn validate_model_selection(value: &Value, key: &str) -> Result<(), StoreError> {
    if value.is_null() {
        return Ok(());
    }
    let selection = value.as_object().ok_or_else(|| {
        StoreError::InvalidData(format!("{key} must be a model selection object or null"))
    })?;
    if selection
        .keys()
        .any(|field| field != "id" && field != "model")
        || !selection.get("model").is_some_and(Value::is_string)
    {
        return Err(StoreError::InvalidData(format!(
            "{key} has an invalid model selection shape"
        )));
    }
    let id = selection.get("id").ok_or_else(|| {
        StoreError::InvalidData(format!("{key} model selection is missing an ID"))
    })?;
    let valid_id = id
        .as_i64()
        .is_some_and(|id| (0..=MAX_SAFE_INTEGER).contains(&id))
        || id.as_str().is_some_and(str::is_empty);
    if !valid_id {
        return Err(StoreError::InvalidData(format!(
            "{key} model ID must be a safe integer or an empty string"
        )));
    }
    Ok(())
}

fn validate_proxy_targets(value: &Value) -> Result<(), StoreError> {
    let groups = value
        .as_object()
        .ok_or_else(|| StoreError::InvalidData("proxy targets must be an object".into()))?;
    for aliases in groups.values() {
        let aliases = aliases.as_object().ok_or_else(|| {
            StoreError::InvalidData("proxy target group must be an object".into())
        })?;
        for targets in aliases.values() {
            let targets = targets
                .as_array()
                .ok_or_else(|| StoreError::InvalidData("proxy targets must be an array".into()))?;
            for target in targets {
                let target = target.as_object().ok_or_else(|| {
                    StoreError::InvalidData("proxy target must be an object".into())
                })?;
                let valid_id = target
                    .get("id")
                    .and_then(Value::as_i64)
                    .is_some_and(|id| (0..=MAX_SAFE_INTEGER).contains(&id));
                if target.keys().any(|field| field != "id" && field != "model")
                    || !valid_id
                    || !target.get("model").is_some_and(Value::is_string)
                {
                    return Err(StoreError::InvalidData(
                        "proxy target has an invalid shape".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_proxy_keys(value: &Value) -> Result<(), StoreError> {
    let keys = value
        .as_array()
        .ok_or_else(|| StoreError::InvalidData("proxy keys must be an array".into()))?;
    for key in keys {
        let key = key
            .as_object()
            .ok_or_else(|| StoreError::InvalidData("proxy key must be an object".into()))?;
        if key.keys().any(|field| field != "name" && field != "token")
            || !key.get("name").is_some_and(Value::is_string)
            || !key.get("token").is_some_and(Value::is_string)
        {
            return Err(StoreError::InvalidData(
                "proxy key has an invalid shape".into(),
            ));
        }
    }
    Ok(())
}

fn validate_rows(
    category: ConfigCategory,
    rows: &[BTreeMap<String, Value>],
) -> Result<(), StoreError> {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let allowed_columns = package_columns(category);
    for row in rows {
        if row
            .keys()
            .any(|key| !allowed_columns.contains(&key.as_str()))
        {
            return Err(StoreError::InvalidData(format!(
                "{category:?} row contains an unsupported column"
            )));
        }
        for required in required_columns(category) {
            if !row.contains_key(*required) || row[*required].is_null() {
                return Err(StoreError::InvalidData(format!(
                    "{category:?} row is missing {required}"
                )));
            }
        }
        let id = row
            .get("id")
            .ok_or_else(|| StoreError::InvalidData(format!("{category:?} row is missing id")))?;
        if matches!(
            category,
            ConfigCategory::AiModels
                | ConfigCategory::Skills
                | ConfigCategory::Mcp
                | ConfigCategory::Proxy
        ) {
            let id = id.as_i64().ok_or_else(|| {
                StoreError::InvalidData("auto-increment id must be an integer".into())
            })?;
            if !(0..=MAX_SAFE_INTEGER).contains(&id) || !ids.insert(id.to_string()) {
                return Err(StoreError::InvalidData(
                    "invalid or duplicate auto-increment id".into(),
                ));
            }
        } else {
            let id = id
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| StoreError::InvalidData("text id must be non-empty".into()))?;
            if !ids.insert(id.to_string()) {
                return Err(StoreError::InvalidData("duplicate text id".into()));
            }
        }
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| StoreError::InvalidData(format!("{category:?} row is missing name")))?;
        if !names.insert(name.to_string()) {
            return Err(StoreError::InvalidData(format!(
                "duplicate {category:?} name"
            )));
        }
        validate_row_contract(category, row)?;
    }
    Ok(())
}

fn validate_row_contract(
    category: ConfigCategory,
    row: &BTreeMap<String, Value>,
) -> Result<(), StoreError> {
    match category {
        ConfigCategory::AiModels => {
            require_string(
                row,
                &[
                    "models",
                    "default_model",
                    "api_protocol",
                    "base_url",
                    "api_key",
                ],
            )?;
            let _: Vec<super::ModelConfig> = parse_json_field(row, "models")?;
            require_integer_fields(row, &["max_tokens", "top_k", "sort_index"])?;
            require_number_fields(row, &["temperature", "top_p"])?;
            require_boolean_fields(row, &["is_default", "disabled", "is_official"])?;
            validate_optional_json(row, "metadata")?;
        }
        ConfigCategory::Skills => {
            require_string(row, &["prompt"])?;
            require_integer_fields(row, &["sort_index"])?;
            require_boolean_fields(row, &["disabled"])?;
            validate_optional_json(row, "metadata")?;
        }
        ConfigCategory::Mcp => {
            require_string(row, &["description", "config"])?;
            let config: Value = parse_json_field(row, "config")?;
            validate_mcp_config(&config)?;
            require_boolean_fields(row, &["disabled"])?;
        }
        ConfigCategory::Proxy => {
            require_string(
                row,
                &[
                    "description",
                    "prompt_injection",
                    "prompt_text",
                    "tool_filter",
                ],
            )?;
            require_number_fields(row, &["temperature"])?;
            require_boolean_fields(row, &["disabled"])?;
            validate_optional_json(row, "metadata")?;
        }
        ConfigCategory::Agents => {
            require_string(row, &["system_prompt"])?;
            require_string_fields(
                row,
                &[
                    "description",
                    "agent_type",
                    "planning_prompt",
                    "available_tools",
                    "auto_approve",
                    "plan_model",
                    "act_model",
                    "vision_model",
                    "created_at",
                    "updated_at",
                    "shell_policy",
                    "allowed_paths",
                    "approval_level",
                    "role",
                    "parent_agent_id",
                    "selected_skills",
                    "image_recognition_prompt",
                    "phase",
                    "mcp_tool_exposure",
                    "sub_agent_role",
                    "sandbox_execution_mode",
                    "sandbox_scheme_id",
                ],
            )?;
            require_integer_fields(row, &["max_contexts", "sort_index", "version"])?;
            require_boolean_fields(
                row,
                &["final_audit", "skill_enabled", "is_system", "disabled"],
            )?;
            for field in [
                "available_tools",
                "auto_approve",
                "selected_skills",
                "mcp_tool_exposure",
            ] {
                if row.get(field).is_some_and(|value| !value.is_null()) {
                    let _: Vec<String> = parse_json_field(row, field)?;
                }
            }
            for field in ["shell_policy", "allowed_paths"] {
                if row.get(field).is_some_and(|value| !value.is_null()) {
                    validate_json_field(row, field)?;
                }
            }
            if row.get("models").is_some_and(|value| !value.is_null()) {
                validate_agent_models(row)?;
            }
        }
        ConfigCategory::Sandbox => {
            let mut scheme_value = serde_json::to_value(row)?;
            if let Some(config) = scheme_value.get_mut("config") {
                if let Some(config_json) = config.as_str() {
                    *config = serde_json::from_str(config_json)?;
                }
            }
            if let Some(disabled) = scheme_value.get_mut("disabled") {
                if let Some(value) = disabled.as_i64() {
                    *disabled = Value::Bool(value != 0);
                }
            }
            let scheme: super::SandboxScheme = serde_json::from_value(scheme_value)?;
            scheme.validate()?;
            require_boolean_fields(row, &["disabled"])?;
        }
    }
    Ok(())
}

fn require_string(row: &BTreeMap<String, Value>, fields: &[&str]) -> Result<(), StoreError> {
    for field in fields {
        if !row.get(*field).is_some_and(Value::is_string) {
            return Err(StoreError::InvalidData(format!(
                "row field {field} must be a string"
            )));
        }
    }
    Ok(())
}

fn require_string_fields(row: &BTreeMap<String, Value>, fields: &[&str]) -> Result<(), StoreError> {
    for field in fields {
        if let Some(value) = row.get(*field) {
            if !value.is_null() && !value.is_string() {
                return Err(StoreError::InvalidData(format!(
                    "row field {field} must be a string or null"
                )));
            }
        }
    }
    Ok(())
}

fn require_integer_fields(
    row: &BTreeMap<String, Value>,
    fields: &[&str],
) -> Result<(), StoreError> {
    for field in fields {
        if let Some(value) = row.get(*field) {
            if !value.is_null() && value.as_i64().is_none() {
                return Err(StoreError::InvalidData(format!(
                    "row field {field} must be an integer or null"
                )));
            }
        }
    }
    Ok(())
}

fn require_number_fields(row: &BTreeMap<String, Value>, fields: &[&str]) -> Result<(), StoreError> {
    for field in fields {
        if let Some(value) = row.get(*field) {
            if !value.is_null() && !value.is_number() {
                return Err(StoreError::InvalidData(format!(
                    "row field {field} must be a number or null"
                )));
            }
        }
    }
    Ok(())
}

fn require_boolean_fields(
    row: &BTreeMap<String, Value>,
    fields: &[&str],
) -> Result<(), StoreError> {
    for field in fields {
        if let Some(value) = row.get(*field) {
            let sqlite_boolean = matches!(value.as_i64(), Some(0 | 1));
            if !value.is_null() && !value.is_boolean() && !sqlite_boolean {
                return Err(StoreError::InvalidData(format!(
                    "row field {field} must be a boolean or null"
                )));
            }
        }
    }
    Ok(())
}

fn validate_optional_json(row: &BTreeMap<String, Value>, field: &str) -> Result<(), StoreError> {
    if row.contains_key(field) {
        validate_json_field(row, field)?;
    }
    Ok(())
}

fn validate_json_field(row: &BTreeMap<String, Value>, field: &str) -> Result<(), StoreError> {
    if let Some(value) = row.get(field).filter(|value| !value.is_null()) {
        let value = value.as_str().ok_or_else(|| {
            StoreError::InvalidData(format!("row field {field} must be a JSON string or null"))
        })?;
        let _: Value = serde_json::from_str(value)?;
    }
    Ok(())
}

fn validate_agent_models(row: &BTreeMap<String, Value>) -> Result<(), StoreError> {
    let value: Value = parse_json_field(row, "models")?;
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::InvalidData("agent models must be an object".into()))?;
    const ALLOWED: &[&str] = &["plan", "act", "vision", "utility"];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(StoreError::InvalidData(
            "agent models contains an unsupported field".into(),
        ));
    }
    for value in object.values().filter(|value| !value.is_null()) {
        let model = value
            .as_object()
            .ok_or_else(|| StoreError::InvalidData("agent model entry must be an object".into()))?;
        const MODEL_FIELDS: &[&str] = &[
            "id",
            "model",
            "temperature",
            "thinking",
            "functionCall",
            "contextSize",
            "maxTokens",
        ];
        if model
            .keys()
            .any(|key| !MODEL_FIELDS.contains(&key.as_str()))
            || model.get("id").and_then(Value::as_i64).is_none()
            || !model.get("model").is_some_and(Value::is_string)
        {
            return Err(StoreError::InvalidData(
                "agent model entry has an invalid shape".into(),
            ));
        }
    }
    let _: AgentModels = serde_json::from_value(value)?;
    Ok(())
}

fn validate_mcp_config(value: &Value) -> Result<(), StoreError> {
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::InvalidData("MCP config must be an object".into()))?;
    const ALLOWED: &[&str] = &[
        "name",
        "type",
        "url",
        "bearer_token",
        "proxy",
        "command",
        "args",
        "env",
        "disabled_tools",
        "timeout",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(StoreError::InvalidData(
            "MCP config contains an unsupported field".into(),
        ));
    }
    if !object.get("name").is_some_and(Value::is_string)
        || !object.get("type").is_some_and(Value::is_string)
    {
        return Err(StoreError::InvalidData(
            "MCP config requires string name and type".into(),
        ));
    }
    if !matches!(
        object.get("type").and_then(Value::as_str),
        Some("stdio" | "sse" | "streamable_http")
    ) {
        return Err(StoreError::InvalidData(
            "MCP config has an unsupported type".into(),
        ));
    }
    for field in ["url", "bearer_token", "proxy", "command"] {
        if let Some(value) = object.get(field) {
            if !value.is_null() && !value.is_string() {
                return Err(StoreError::InvalidData(format!(
                    "MCP config field {field} must be a string"
                )));
            }
        }
    }
    for field in ["args", "disabled_tools"] {
        if let Some(value) = object.get(field) {
            if !value.is_null()
                && !value
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
            {
                return Err(StoreError::InvalidData(format!(
                    "MCP config field {field} must be a string array"
                )));
            }
        }
    }
    if let Some(value) = object.get("env") {
        if !value.is_null()
            && !value.as_array().is_some_and(|pairs| {
                pairs.iter().all(|pair| {
                    pair.as_array()
                        .is_some_and(|pair| pair.len() == 2 && pair.iter().all(Value::is_string))
                })
            })
        {
            return Err(StoreError::InvalidData(
                "MCP config env must be string pairs".into(),
            ));
        }
    }
    if let Some(value) = object.get("timeout") {
        if !value.is_null() && value.as_u64().is_none() {
            return Err(StoreError::InvalidData(
                "MCP config timeout must be an unsigned integer".into(),
            ));
        }
    }
    Ok(())
}

fn parse_json_field<T: DeserializeOwned>(
    row: &BTreeMap<String, Value>,
    field: &str,
) -> Result<T, StoreError> {
    let value = row.get(field).and_then(Value::as_str).ok_or_else(|| {
        StoreError::InvalidData(format!("row field {field} must be a JSON string"))
    })?;
    serde_json::from_str(value).map_err(StoreError::from)
}

fn package_columns(category: ConfigCategory) -> BTreeSet<&'static str> {
    let columns: &[&str] = match category {
        ConfigCategory::AiModels => &[
            "id",
            "name",
            "models",
            "default_model",
            "api_protocol",
            "base_url",
            "api_key",
            "max_tokens",
            "temperature",
            "top_p",
            "top_k",
            "sort_index",
            "is_default",
            "disabled",
            "is_official",
            "official_id",
            "metadata",
        ],
        ConfigCategory::Skills => &[
            "id",
            "name",
            "icon",
            "logo",
            "prompt",
            "share_id",
            "sort_index",
            "disabled",
            "metadata",
        ],
        ConfigCategory::Mcp => &["id", "name", "description", "config", "disabled"],
        ConfigCategory::Proxy => &[
            "id",
            "name",
            "description",
            "prompt_injection",
            "prompt_text",
            "tool_filter",
            "temperature",
            "metadata",
            "disabled",
        ],
        ConfigCategory::Agents => &[
            "id",
            "name",
            "description",
            "system_prompt",
            "agent_type",
            "planning_prompt",
            "available_tools",
            "auto_approve",
            "plan_model",
            "act_model",
            "vision_model",
            "max_contexts",
            "created_at",
            "updated_at",
            "models",
            "shell_policy",
            "allowed_paths",
            "final_audit",
            "approval_level",
            "role",
            "parent_agent_id",
            "skill_enabled",
            "selected_skills",
            "image_recognition_prompt",
            "is_system",
            "disabled",
            "phase",
            "sort_index",
            "version",
            "mcp_tool_exposure",
            "sub_agent_role",
            "sandbox_execution_mode",
            "sandbox_scheme_id",
        ],
        ConfigCategory::Sandbox => &[
            "id",
            "name",
            "description",
            "config",
            "disabled",
            "created_at",
            "updated_at",
        ],
    };
    columns.iter().copied().collect()
}

fn required_columns(category: ConfigCategory) -> &'static [&'static str] {
    match category {
        ConfigCategory::AiModels => &[
            "id",
            "name",
            "models",
            "default_model",
            "api_protocol",
            "base_url",
            "api_key",
        ],
        ConfigCategory::Skills => &["id", "name", "prompt"],
        ConfigCategory::Mcp => &["id", "name", "description", "config"],
        ConfigCategory::Proxy => &[
            "id",
            "name",
            "description",
            "prompt_injection",
            "prompt_text",
            "tool_filter",
            "temperature",
        ],
        ConfigCategory::Agents => &["id", "name", "system_prompt"],
        ConfigCategory::Sandbox => &["id", "name", "description", "config", "disabled"],
    }
}

fn validate_package_references(
    package: &ConfigTransferPackage,
    categories: &BTreeSet<ConfigCategory>,
) -> Result<(), StoreError> {
    let model_ids = package
        .ai_models
        .as_ref()
        .map(|payload| integer_ids(&payload.rows))
        .unwrap_or_default();
    if categories.contains(&ConfigCategory::AiModels) {
        let payload = package.ai_models.as_ref().ok_or_else(|| {
            StoreError::InvalidData("package is missing AI models payload".into())
        })?;
        for key in AI_CONFIG_KEYS {
            if let Some(value) = payload.config.get(*key) {
                validate_model_selection_reference(value, &model_ids, key)?;
            }
        }
    }
    if categories.contains(&ConfigCategory::Proxy) {
        let payload = package
            .proxy
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing proxy payload".into()))?;
        if let Some(value) = payload.config.get("chat_completion_proxy") {
            validate_proxy_target_references(value, &model_ids)?;
        }
        if let Some(active_group) = payload
            .config
            .get("active_proxy_group")
            .and_then(Value::as_str)
        {
            let proxy_groups = string_field_set(&payload.rows, "name");
            if !active_group.is_empty() && !proxy_groups.contains(active_group) {
                return Err(StoreError::InvalidData(
                    "active proxy group is absent from the package".into(),
                ));
            }
        }
    }
    if categories.contains(&ConfigCategory::Agents) {
        let payload = package
            .agents
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing agents payload".into()))?;
        let skill_names = package
            .skills
            .as_ref()
            .map(|payload| string_field_set(&payload.rows, "name"))
            .unwrap_or_default();
        let mcp_server_names = package
            .mcp
            .as_ref()
            .map(|payload| string_field_set(&payload.rows, "name"))
            .unwrap_or_default();
        let sandbox_ids = package
            .sandbox
            .as_ref()
            .map(|payload| string_field_set(&payload.rows, "id"))
            .unwrap_or_default();
        for row in &payload.rows {
            validate_agent_row_references(
                row,
                &model_ids,
                &skill_names,
                &mcp_server_names,
                &sandbox_ids,
            )?;
        }
    }
    Ok(())
}

fn integer_ids(rows: &[BTreeMap<String, Value>]) -> BTreeSet<i64> {
    rows.iter()
        .filter_map(|row| row.get("id")?.as_i64())
        .collect()
}

fn string_field_set(rows: &[BTreeMap<String, Value>], field: &str) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|row| row.get(field)?.as_str().map(ToString::to_string))
        .collect()
}

fn validate_model_selection_reference(
    value: &Value,
    model_ids: &BTreeSet<i64>,
    key: &str,
) -> Result<(), StoreError> {
    let Some(id) = value.as_object().and_then(|selection| selection.get("id")) else {
        return Ok(());
    };
    let Some(id) = id.as_i64() else {
        return Ok(());
    };
    if !model_ids.contains(&id) {
        return Err(StoreError::InvalidData(format!(
            "{key} references an unknown AI model"
        )));
    }
    Ok(())
}

fn validate_proxy_target_references(
    value: &Value,
    model_ids: &BTreeSet<i64>,
) -> Result<(), StoreError> {
    let groups = value
        .as_object()
        .ok_or_else(|| StoreError::InvalidData("proxy targets must be an object".into()))?;
    for aliases in groups.values() {
        let aliases = aliases.as_object().ok_or_else(|| {
            StoreError::InvalidData("proxy target group must be an object".into())
        })?;
        for targets in aliases.values() {
            let targets = targets
                .as_array()
                .ok_or_else(|| StoreError::InvalidData("proxy targets must be an array".into()))?;
            for target in targets {
                let id = target.get("id").and_then(Value::as_i64).ok_or_else(|| {
                    StoreError::InvalidData("proxy target ID must be an integer".into())
                })?;
                if !model_ids.contains(&id) {
                    return Err(StoreError::InvalidData(
                        "proxy configuration references an unknown AI model".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_agent_row_references(
    row: &BTreeMap<String, Value>,
    model_ids: &BTreeSet<i64>,
    skill_names: &BTreeSet<String>,
    mcp_server_names: &BTreeSet<String>,
    sandbox_ids: &BTreeSet<String>,
) -> Result<(), StoreError> {
    if let Some(models) = row.get("models").and_then(Value::as_str) {
        let models: AgentModels = serde_json::from_str(models)?;
        for model in [models.plan, models.act, models.vision, models.utility]
            .into_iter()
            .flatten()
        {
            if !model_ids.contains(&model.id) {
                return Err(StoreError::InvalidData(
                    "agent models references an unknown AI model".into(),
                ));
            }
        }
    }
    if let Some(values) = row.get("selected_skills").and_then(Value::as_str) {
        for value in serde_json::from_str::<Vec<String>>(values)? {
            if !skill_names.contains(&value) {
                return Err(StoreError::InvalidData(
                    "agent references an unknown skill".into(),
                ));
            }
        }
    }
    if let Some(values) = row.get("mcp_tool_exposure").and_then(Value::as_str) {
        for tool_id in serde_json::from_str::<Vec<String>>(values)? {
            let Some((server_name, tool_name)) = tool_id.split_once(MCP_TOOL_NAME_SPLIT) else {
                return Err(StoreError::InvalidData(
                    "agent MCP tool exposure has an invalid tool ID".into(),
                ));
            };
            if server_name.is_empty()
                || tool_name.is_empty()
                || tool_name.contains(MCP_TOOL_NAME_SPLIT)
                || !mcp_server_names.contains(server_name)
            {
                return Err(StoreError::InvalidData(
                    "agent references an unknown MCP server".into(),
                ));
            }
        }
    }
    if let Some(scheme_id) = row.get("sandbox_scheme_id").and_then(Value::as_str) {
        if !sandbox_ids.contains(scheme_id) {
            return Err(StoreError::InvalidData(
                "agent references an unknown sandbox scheme".into(),
            ));
        }
    }
    Ok(())
}

fn read_rows(conn: &Connection, sql: &str) -> Result<Vec<BTreeMap<String, Value>>, StoreError> {
    let mut statement = conn.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(ToString::to_string)
        .collect();
    let rows = statement.query_map([], |row| {
        let mut item = BTreeMap::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => Value::from(value),
                ValueRef::Real(value) => Value::from(value),
                ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
                ValueRef::Blob(_) => {
                    return Err(rusqlite::Error::InvalidColumnType(
                        index,
                        name.clone(),
                        rusqlite::types::Type::Blob,
                    ))
                }
            };
            item.insert(name.clone(), value);
        }
        Ok(item)
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

fn read_config(conn: &Connection, keys: &[&str]) -> Result<BTreeMap<String, Value>, StoreError> {
    let mut output = BTreeMap::new();
    let mut statement = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
    for key in keys {
        if let Ok(value) = statement.query_row([key], |row| row.get::<_, String>(0)) {
            output.insert(
                (*key).to_string(),
                serde_json::from_str(&value).unwrap_or(Value::String(value)),
            );
        }
    }
    Ok(output)
}

pub fn import_config_package(
    store: &super::MainStore,
    path: impl AsRef<Path>,
    selected_categories: impl IntoIterator<Item = ConfigCategory>,
) -> Result<ConfigImportResult, StoreError> {
    let package = read_config_package(path)?;
    let selected = category_closure(selected_categories);
    if selected.is_empty()
        || !selected
            .iter()
            .all(|category| package.categories.contains(category))
    {
        return Err(StoreError::InvalidData(
            "selected configuration categories are unavailable in the package".into(),
        ));
    }
    let _guard = store.config_update_lock.lock();
    let runtime = store.db_runtime()?;
    let (result, config) = runtime.write_blocking(move |conn| {
        let transaction = conn.transaction()?;
        transaction.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
        let result = import_in_transaction(&transaction, &package, &selected)?;
        let config = super::MainStore::load_config(&transaction)?;
        transaction.commit()?;
        Ok((result, config))
    })?;
    store.config.replace(config);
    Ok(result)
}

fn import_in_transaction(
    transaction: &Transaction<'_>,
    package: &ConfigTransferPackage,
    selected: &BTreeSet<ConfigCategory>,
) -> Result<ConfigImportResult, StoreError> {
    let mut result = ConfigImportResult {
        restart_recommended: selected.contains(&ConfigCategory::Mcp)
            || selected.contains(&ConfigCategory::Proxy),
        ..Default::default()
    };
    let protected_agents = if selected.contains(&ConfigCategory::Agents) {
        protected_agent_ids(transaction)?
    } else {
        BTreeSet::new()
    };
    let incoming_agent_ids = package
        .agents
        .as_ref()
        .map(|payload| string_field_set(&payload.rows, "id"))
        .unwrap_or_default();
    let retained_agents = if selected.contains(&ConfigCategory::Agents) {
        retained_agent_ids(transaction, &protected_agents, &incoming_agent_ids)?
    } else {
        BTreeSet::new()
    };
    validate_import_preflight(transaction, package, selected, &protected_agents)?;

    if selected.contains(&ConfigCategory::Sandbox) {
        let incoming = package
            .sandbox
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing sandbox payload".into()))?;
        let protected_sandbox = protected_sandbox_ids(
            transaction,
            &retained_agents,
            selected.contains(&ConfigCategory::Agents),
        )?;
        result.preserved_sandbox_schemes =
            replace_sandbox_schemes(transaction, incoming, &protected_sandbox)?;
        result
            .imported_counts
            .insert(ConfigCategory::Sandbox, incoming.rows.len());
    }
    if selected.contains(&ConfigCategory::AiModels) {
        let payload = package.ai_models.as_ref().ok_or_else(|| {
            StoreError::InvalidData("package is missing AI models payload".into())
        })?;
        replace_auto_table(transaction, "ai_model", payload)?;
        replace_config(transaction, AI_CONFIG_KEYS, &payload.config)?;
        result
            .imported_counts
            .insert(ConfigCategory::AiModels, payload.rows.len());
    }
    if selected.contains(&ConfigCategory::Skills) {
        let payload = package
            .skills
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing skills payload".into()))?;
        replace_auto_table(transaction, "ai_skill", payload)?;
        result
            .imported_counts
            .insert(ConfigCategory::Skills, payload.rows.len());
    }
    if selected.contains(&ConfigCategory::Mcp) {
        let payload = package
            .mcp
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing MCP payload".into()))?;
        replace_auto_table(transaction, "mcp", payload)?;
        result
            .imported_counts
            .insert(ConfigCategory::Mcp, payload.rows.len());
    }
    if selected.contains(&ConfigCategory::Proxy) {
        let payload = package
            .proxy
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing proxy payload".into()))?;
        replace_auto_table(transaction, "proxy_group", payload)?;
        replace_config(transaction, PROXY_CONFIG_KEYS, &payload.config)?;
        result
            .imported_counts
            .insert(ConfigCategory::Proxy, payload.rows.len());
    }
    if selected.contains(&ConfigCategory::Agents) {
        let payload = package
            .agents
            .as_ref()
            .ok_or_else(|| StoreError::InvalidData("package is missing agents payload".into()))?;
        result.preserved_agents = retained_agents.len();
        replace_agents(transaction, payload, &protected_agents)?;
        result
            .imported_counts
            .insert(ConfigCategory::Agents, payload.rows.len());
    }

    validate_references(transaction, selected)?;
    let foreign_key_errors: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_errors != 0 {
        return Err(StoreError::InvalidData(
            "configuration import would violate foreign key constraints".into(),
        ));
    }
    result.api_keys_locked =
        super::api_key_crypto::inspect_encryption_status(transaction)?.is_locked();
    Ok(result)
}

fn validate_import_preflight(
    transaction: &Transaction<'_>,
    package: &ConfigTransferPackage,
    selected: &BTreeSet<ConfigCategory>,
    protected_agents: &BTreeSet<String>,
) -> Result<(), StoreError> {
    if !selected.contains(&ConfigCategory::Agents) {
        return Ok(());
    }
    let payload = package
        .agents
        .as_ref()
        .ok_or_else(|| StoreError::InvalidData("package is missing agents payload".into()))?;
    let incoming_ids = string_field_set(&payload.rows, "id");
    for row in &payload.rows {
        let Some(parent) = row.get("parent_agent_id").and_then(Value::as_str) else {
            continue;
        };
        if incoming_ids.contains(parent) || protected_agents.contains(parent) {
            continue;
        }
        let builtin_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1 AND id LIKE 'builtin:%')",
            [parent],
            |row| row.get(0),
        )?;
        if !builtin_exists {
            return Err(StoreError::InvalidData(
                "agent parent is absent from the package and target database".into(),
            ));
        }
    }
    Ok(())
}

fn replace_auto_table(
    transaction: &Transaction<'_>,
    table: &str,
    payload: &TablePayload,
) -> Result<(), StoreError> {
    transaction.execute(&format!("DELETE FROM {table}"), [])?;
    transaction.execute("DELETE FROM sqlite_sequence WHERE name = ?1", [table])?;
    for row in &payload.rows {
        insert_row(transaction, table, row)?;
    }
    Ok(())
}

fn replace_config(
    transaction: &Transaction<'_>,
    keys: &[&str],
    values: &BTreeMap<String, Value>,
) -> Result<(), StoreError> {
    for key in keys {
        transaction.execute("DELETE FROM config WHERE key = ?1", [key])?;
    }
    for (key, value) in values {
        transaction.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)",
            params![key, serde_json::to_string(value)?],
        )?;
    }
    Ok(())
}

fn protected_agent_ids(transaction: &Transaction<'_>) -> Result<BTreeSet<String>, StoreError> {
    let mut ids = BTreeSet::new();
    for sql in [
        "SELECT agent_id FROM workflows WHERE agent_id IS NOT NULL",
        "SELECT agent_id FROM workflow_automations",
    ] {
        let mut statement = transaction.prepare(sql)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        ids.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }
    loop {
        let mut added = false;
        let snapshot: Vec<_> = ids.iter().cloned().collect();
        for id in snapshot {
            let parent: Option<String> = transaction
                .query_row(
                    "SELECT parent_agent_id FROM agents WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap_or(None);
            if let Some(parent) = parent {
                added |= ids.insert(parent);
            }
        }
        if !added {
            return Ok(ids);
        }
    }
}

fn retained_agent_ids(
    transaction: &Transaction<'_>,
    protected_agents: &BTreeSet<String>,
    incoming_agent_ids: &BTreeSet<String>,
) -> Result<BTreeSet<String>, StoreError> {
    let mut ids = protected_agents.clone();
    let mut statement = transaction.prepare("SELECT id FROM agents WHERE id LIKE 'builtin:%'")?;
    let builtin_ids = statement.query_map([], |row| row.get::<_, String>(0))?;
    for builtin_id in builtin_ids.collect::<Result<Vec<_>, _>>()? {
        if !incoming_agent_ids.contains(&builtin_id) {
            ids.insert(builtin_id);
        }
    }
    Ok(ids)
}

fn protected_sandbox_ids(
    transaction: &Transaction<'_>,
    retained_agents: &BTreeSet<String>,
    agents_are_replaced: bool,
) -> Result<BTreeSet<String>, StoreError> {
    let mut output = BTreeSet::new();
    if agents_are_replaced {
        let mut statement = transaction.prepare(
            "SELECT sandbox_scheme_id FROM agents WHERE id = ?1 AND sandbox_scheme_id IS NOT NULL",
        )?;
        for agent_id in retained_agents {
            if let Ok(scheme_id) = statement.query_row([agent_id], |row| row.get::<_, String>(0)) {
                output.insert(scheme_id);
            }
        }
    } else {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT sandbox_scheme_id FROM agents WHERE sandbox_scheme_id IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        output.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }
    Ok(output)
}

fn replace_sandbox_schemes(
    transaction: &Transaction<'_>,
    payload: &TablePayload,
    protected: &BTreeSet<String>,
) -> Result<usize, StoreError> {
    let incoming_names: BTreeMap<_, _> = payload
        .rows
        .iter()
        .filter_map(|row| {
            Some((
                row.get("name")?.as_str()?.to_string(),
                row.get("id")?.as_str()?.to_string(),
            ))
        })
        .collect();
    for (name, incoming_id) in &incoming_names {
        transaction.execute(
            "UPDATE agents SET sandbox_scheme_id = ?1 WHERE sandbox_scheme_id IN (SELECT id FROM sandbox_schemes WHERE name = ?2)",
            params![incoming_id, name],
        )?;
    }
    let incoming_ids: BTreeSet<_> = incoming_names.values().cloned().collect();
    let existing = read_rows(transaction, "SELECT * FROM sandbox_schemes")?;
    let mut preserved_count = 0;
    for row in existing {
        let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = row.get("name").and_then(Value::as_str).unwrap_or_default();
        let remapped_to_incoming = incoming_names.contains_key(name);
        if protected.contains(id) && !remapped_to_incoming && !incoming_ids.contains(id) {
            preserved_count += 1;
        }
        if (!protected.contains(id) || remapped_to_incoming) && !incoming_ids.contains(id) {
            transaction.execute("DELETE FROM sandbox_schemes WHERE id = ?1", [id])?;
        }
    }
    for row in &payload.rows {
        upsert_row(transaction, "sandbox_schemes", row)?;
    }
    Ok(preserved_count)
}

fn replace_agents(
    transaction: &Transaction<'_>,
    payload: &TablePayload,
    protected: &BTreeSet<String>,
) -> Result<(), StoreError> {
    let incoming_ids: BTreeSet<_> = payload
        .rows
        .iter()
        .filter_map(|row| row.get("id")?.as_str().map(ToString::to_string))
        .collect();
    let existing = read_rows(transaction, "SELECT * FROM agents")?;
    for row in &existing {
        let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
        if id.starts_with("builtin:") || protected.contains(id) || incoming_ids.contains(id) {
            continue;
        }
        transaction.execute("DELETE FROM agents WHERE id = ?1", [id])?;
    }
    for row in &payload.rows {
        let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = row.get("name").and_then(Value::as_str).unwrap_or_default();
        let conflicting_id: Option<String> = transaction
            .query_row("SELECT id FROM agents WHERE name = ?1", [name], |row| {
                row.get(0)
            })
            .ok();
        if let Some(conflicting_id) = conflicting_id.filter(|existing_id| existing_id != id) {
            if protected.contains(&conflicting_id) || conflicting_id.starts_with("builtin:") {
                let suffix = conflicting_id.chars().take(8).collect::<String>();
                transaction.execute(
                    "UPDATE agents SET name = ?1 WHERE id = ?2",
                    params![format!("{name} ({suffix})"), conflicting_id],
                )?;
            } else {
                transaction.execute("DELETE FROM agents WHERE id = ?1", [conflicting_id])?;
            }
        }
        upsert_row(transaction, "agents", row)?;
    }
    Ok(())
}

fn validate_references(
    transaction: &Transaction<'_>,
    selected: &BTreeSet<ConfigCategory>,
) -> Result<(), StoreError> {
    let ai_models_replaced = selected.contains(&ConfigCategory::AiModels);
    if ai_models_replaced {
        for key in AI_CONFIG_KEYS {
            let value: Option<String> = transaction
                .query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .ok();
            if let Some(value) = value {
                validate_json_model_ids(transaction, &value)?;
            }
        }
    }
    if ai_models_replaced || selected.contains(&ConfigCategory::Proxy) {
        let value: Option<String> = transaction
            .query_row(
                "SELECT value FROM config WHERE key = 'chat_completion_proxy'",
                [],
                |row| row.get(0),
            )
            .ok();
        if let Some(value) = value {
            let value: Value = serde_json::from_str(&value)?;
            let model_ids = integer_ids(&read_rows(transaction, "SELECT id FROM ai_model")?);
            validate_proxy_target_references(&value, &model_ids)?;
        }
    }
    validate_retained_agent_references(transaction, selected)?;
    let dangling_sandbox: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agents a LEFT JOIN sandbox_schemes s ON s.id = a.sandbox_scheme_id WHERE a.sandbox_scheme_id IS NOT NULL AND s.id IS NULL",
        [], |row| row.get(0),
    )?;
    if dangling_sandbox != 0 {
        return Err(StoreError::InvalidData(
            "agent has an unknown sandbox scheme".into(),
        ));
    }
    let dangling_parent: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agents a LEFT JOIN agents parent ON parent.id = a.parent_agent_id WHERE a.parent_agent_id IS NOT NULL AND parent.id IS NULL",
        [], |row| row.get(0),
    )?;
    if dangling_parent != 0 {
        return Err(StoreError::InvalidData(
            "agent has an unknown parent agent".into(),
        ));
    }
    Ok(())
}

fn validate_retained_agent_references(
    transaction: &Transaction<'_>,
    selected: &BTreeSet<ConfigCategory>,
) -> Result<(), StoreError> {
    let validates_agent_dependencies = selected.contains(&ConfigCategory::AiModels)
        || selected.contains(&ConfigCategory::Skills)
        || selected.contains(&ConfigCategory::Mcp)
        || selected.contains(&ConfigCategory::Sandbox);
    if !validates_agent_dependencies {
        return Ok(());
    }

    let model_ids = if selected.contains(&ConfigCategory::AiModels) {
        integer_ids(&read_rows(transaction, "SELECT id FROM ai_model")?)
    } else {
        BTreeSet::new()
    };
    let skill_names = if selected.contains(&ConfigCategory::Skills) {
        string_field_set(
            &read_rows(transaction, "SELECT name FROM ai_skill")?,
            "name",
        )
    } else {
        BTreeSet::new()
    };
    let mcp_server_names = if selected.contains(&ConfigCategory::Mcp) {
        string_field_set(&read_rows(transaction, "SELECT name FROM mcp")?, "name")
    } else {
        BTreeSet::new()
    };
    let sandbox_ids = if selected.contains(&ConfigCategory::Sandbox) {
        string_field_set(
            &read_rows(transaction, "SELECT id FROM sandbox_schemes")?,
            "id",
        )
    } else {
        BTreeSet::new()
    };

    for row in read_rows(transaction, "SELECT * FROM agents")? {
        if selected.contains(&ConfigCategory::AiModels) {
            if let Some(models) = row.get("models").and_then(Value::as_str) {
                let models: AgentModels = serde_json::from_str(models)?;
                for model in [models.plan, models.act, models.vision, models.utility]
                    .into_iter()
                    .flatten()
                {
                    if !model_ids.contains(&model.id) {
                        return Err(StoreError::InvalidData(
                            "retained agent references an unknown AI model".into(),
                        ));
                    }
                }
            }
        }
        if selected.contains(&ConfigCategory::Skills) {
            if let Some(values) = row.get("selected_skills").and_then(Value::as_str) {
                for value in serde_json::from_str::<Vec<String>>(values)? {
                    if !skill_names.contains(&value) {
                        return Err(StoreError::InvalidData(
                            "retained agent references an unknown skill".into(),
                        ));
                    }
                }
            }
        }
        if selected.contains(&ConfigCategory::Mcp) {
            if let Some(values) = row.get("mcp_tool_exposure").and_then(Value::as_str) {
                for tool_id in serde_json::from_str::<Vec<String>>(values)? {
                    let Some((server_name, tool_name)) = tool_id.split_once(MCP_TOOL_NAME_SPLIT)
                    else {
                        return Err(StoreError::InvalidData(
                            "retained agent has an invalid MCP tool ID".into(),
                        ));
                    };
                    if server_name.is_empty()
                        || tool_name.is_empty()
                        || tool_name.contains(MCP_TOOL_NAME_SPLIT)
                        || !mcp_server_names.contains(server_name)
                    {
                        return Err(StoreError::InvalidData(
                            "retained agent references an unknown MCP server".into(),
                        ));
                    }
                }
            }
        }
        if selected.contains(&ConfigCategory::Sandbox) {
            if let Some(scheme_id) = row.get("sandbox_scheme_id").and_then(Value::as_str) {
                if !sandbox_ids.contains(scheme_id) {
                    return Err(StoreError::InvalidData(
                        "retained agent references an unknown sandbox scheme".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_json_model_ids(transaction: &Transaction<'_>, json: &str) -> Result<(), StoreError> {
    let value: Value = serde_json::from_str(json)?;
    let mut ids = Vec::new();
    collect_model_ids(&value, &mut ids);
    for id in ids {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_model WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::InvalidData(
                "proxy configuration references an unknown AI model".into(),
            ));
        }
    }
    Ok(())
}

fn collect_model_ids(value: &Value, ids: &mut Vec<i64>) {
    match value {
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(Value::as_i64) {
                ids.push(id);
            }
            for value in object.values() {
                collect_model_ids(value, ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_model_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn insert_row(
    transaction: &Transaction<'_>,
    table: &str,
    row: &BTreeMap<String, Value>,
) -> Result<(), StoreError> {
    let columns = table_columns(transaction, table)?;
    if row.keys().any(|key| !columns.contains(key)) {
        return Err(StoreError::InvalidData(format!(
            "{table} row contains an unsupported column"
        )));
    }
    let entries: Vec<_> = row.iter().collect();
    if entries.is_empty() {
        return Err(StoreError::InvalidData(format!(
            "{table} row has no known columns"
        )));
    }
    let names = entries
        .iter()
        .map(|(key, _)| format!("\"{key}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=entries.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let values = entries
        .iter()
        .map(|(_, value)| json_to_sql(value))
        .collect::<Result<Vec<_>, _>>()?;
    transaction.execute(
        &format!("INSERT INTO {table} ({names}) VALUES ({placeholders})"),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
}

fn upsert_row(
    transaction: &Transaction<'_>,
    table: &str,
    row: &BTreeMap<String, Value>,
) -> Result<(), StoreError> {
    let id = row
        .get("id")
        .ok_or_else(|| StoreError::InvalidData("row is missing id".into()))?;
    let columns = table_columns(transaction, table)?;
    if row.keys().any(|key| !columns.contains(key)) {
        return Err(StoreError::InvalidData(format!(
            "{table} row contains an unsupported column"
        )));
    }
    let entries: Vec<_> = row.iter().filter(|(key, _)| key.as_str() != "id").collect();
    let values = entries
        .iter()
        .map(|(_, value)| json_to_sql(value))
        .collect::<Result<Vec<_>, _>>()?;
    if !entries.is_empty() {
        let assignments = entries
            .iter()
            .enumerate()
            .map(|(index, (key, _))| format!("\"{key}\" = ?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut update_values = values.clone();
        update_values.push(json_to_sql(id)?);
        if transaction.execute(
            &format!(
                "UPDATE {table} SET {assignments} WHERE id = ?{}",
                entries.len() + 1
            ),
            rusqlite::params_from_iter(update_values),
        )? != 0
        {
            return Ok(());
        }
    }
    insert_row(transaction, table, row)
}

fn table_columns(connection: &Connection, table: &str) -> Result<BTreeSet<String>, StoreError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<Result<BTreeSet<_>, _>>()?)
}

fn json_to_sql(value: &Value) -> Result<SqlValue, StoreError> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                SqlValue::Integer(value)
            } else {
                SqlValue::Real(
                    value
                        .as_f64()
                        .ok_or_else(|| StoreError::InvalidData("invalid JSON number".into()))?,
                )
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(serde_json::to_string(value)?),
    })
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), StoreError> {
    let parent = path.parent().ok_or_else(|| {
        StoreError::InvalidData("configuration package path has no parent directory".into())
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| StoreError::IoError(error.error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_closure_enforces_dependencies() {
        assert_eq!(
            category_closure([ConfigCategory::Proxy]),
            BTreeSet::from([ConfigCategory::AiModels, ConfigCategory::Proxy])
        );
        assert_eq!(
            category_closure([ConfigCategory::Agents]),
            BTreeSet::from([
                ConfigCategory::AiModels,
                ConfigCategory::Skills,
                ConfigCategory::Mcp,
                ConfigCategory::Agents,
                ConfigCategory::Sandbox
            ])
        );
    }

    #[test]
    fn export_inspect_round_trip_preserves_raw_sensitive_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("configuration.json");
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE ai_model (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, models TEXT, default_model TEXT, api_protocol TEXT, base_url TEXT, api_key TEXT);
             CREATE TABLE ai_skill (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, prompt TEXT);
             CREATE TABLE mcp (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, description TEXT, config TEXT);
             CREATE TABLE proxy_group (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, description TEXT, prompt_injection TEXT, prompt_text TEXT, tool_filter TEXT, temperature REAL);
             CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT, system_prompt TEXT);
             CREATE TABLE sandbox_schemes (id TEXT PRIMARY KEY, name TEXT, description TEXT, config TEXT, disabled INTEGER);
             CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (7, 'Provider', '[]', '', 'openai', 'https://example.test', 'encrypted-api-key');
             INSERT INTO config (key, value) VALUES ('chat_completion_proxy_keys', '[{\"name\":\"Test\",\"token\":\"plain-proxy-token\"}]');
             INSERT INTO config (key, value) VALUES ('proxy_type', '\"excluded-system-proxy\"');",
        ).unwrap();

        let preview = export_config_package(&connection, &path, [ConfigCategory::Proxy]).unwrap();
        assert_eq!(
            preview.categories,
            vec![ConfigCategory::AiModels, ConfigCategory::Proxy]
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("encrypted-api-key"));
        assert!(contents.contains("plain-proxy-token"));
        assert!(!contents.contains("excluded-system-proxy"));
        assert!(!contents.contains("conversations"));
        assert_eq!(
            inspect_config_package(&path).unwrap().counts,
            preview.counts
        );
    }

    #[test]
    fn import_replaces_ai_models_and_resets_sequence() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("config-transfer.sqlite");
        let package_path = directory.path().join("configuration.json");
        let store = super::super::MainStore::new(&database_path).unwrap();
        let runtime = store.db_runtime().unwrap();
        runtime.write_blocking(|connection| {
            connection.execute(
                "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key)
                 VALUES (12, 'Exported', '[]', '', 'openai', 'https://example.test', '')",
                [],
            )?;
            Ok(())
        }).unwrap();
        let export_path = package_path.clone();
        runtime
            .read_blocking(move |connection| {
                export_config_package(connection, &export_path, [ConfigCategory::AiModels])
            })
            .unwrap();
        runtime.write_blocking(|connection| {
            connection.execute("DELETE FROM ai_model", [])?;
            connection.execute(
                "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key)
                 VALUES (99, 'Local', '[]', '', 'openai', 'https://local.test', '')",
                [],
            )?;
            Ok(())
        }).unwrap();

        let result =
            import_config_package(&store, &package_path, [ConfigCategory::AiModels]).unwrap();
        assert_eq!(
            result.imported_counts.get(&ConfigCategory::AiModels),
            Some(&1)
        );
        let (id, name, sequence): (i64, String, i64) = runtime
            .read_blocking(|connection| {
                Ok((
                    connection.query_row("SELECT id FROM ai_model", [], |row| row.get(0))?,
                    connection.query_row("SELECT name FROM ai_model", [], |row| row.get(0))?,
                    connection.query_row(
                        "SELECT seq FROM sqlite_sequence WHERE name = 'ai_model'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!((id, name, sequence), (12, "Exported".to_string(), 12));
        assert_eq!(store.config.get_ai_models().unwrap()[0].id, Some(12));
    }

    #[test]
    fn unknown_model_references_are_rejected_during_inspection() {
        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: vec![ConfigCategory::AiModels],
            ai_models: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from(1)),
                    ("name".into(), Value::from("Provider")),
                    ("models".into(), Value::from("[]")),
                    ("default_model".into(), Value::from("")),
                    ("api_protocol".into(), Value::from("openai")),
                    ("base_url".into(), Value::from("https://example.test")),
                    ("api_key".into(), Value::from("key")),
                ])],
                config: BTreeMap::from([(
                    "vision_model".into(),
                    serde_json::json!({ "id": 99, "model": "missing" }),
                )]),
            }),
            skills: None,
            mcp: None,
            proxy: None,
            agents: None,
            sandbox: None,
        };
        assert!(validate_package(&package).is_err());
    }

    #[test]
    fn invalid_rows_are_rejected_before_import() {
        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: vec![ConfigCategory::AiModels],
            ai_models: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from(1)),
                    ("name".into(), Value::from("Provider")),
                    ("models".into(), Value::from("[]")),
                    ("default_model".into(), Value::from("")),
                    ("api_protocol".into(), Value::from("openai")),
                    ("base_url".into(), Value::from("https://example.test")),
                    ("api_key".into(), Value::from("key")),
                    ("unexpected".into(), Value::from("must fail")),
                ])],
                config: BTreeMap::new(),
            }),
            skills: None,
            mcp: None,
            proxy: None,
            agents: None,
            sandbox: None,
        };
        assert!(validate_package(&package).is_err());
    }

    #[test]
    fn strict_config_contract_rejects_string_ids_malformed_targets_and_unknown_groups() {
        assert!(validate_model_selection(
            &serde_json::json!({ "id": "7", "model": "gpt" }),
            "vision_model"
        )
        .is_err());
        assert!(
            validate_proxy_targets(&serde_json::json!({ "default": { "alias": [{}] } })).is_err()
        );

        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: vec![ConfigCategory::AiModels, ConfigCategory::Proxy],
            ai_models: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from(7)),
                    ("name".into(), Value::from("Provider")),
                    ("models".into(), Value::from("[]")),
                    ("default_model".into(), Value::from("")),
                    ("api_protocol".into(), Value::from("openai")),
                    ("base_url".into(), Value::from("https://example.test")),
                    ("api_key".into(), Value::from("")),
                ])],
                config: BTreeMap::new(),
            }),
            skills: None,
            mcp: None,
            proxy: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from(1)),
                    ("name".into(), Value::from("Default")),
                    ("description".into(), Value::from("")),
                    ("prompt_injection".into(), Value::from("")),
                    ("prompt_text".into(), Value::from("")),
                    ("tool_filter".into(), Value::from("")),
                    ("temperature".into(), Value::from(1.0)),
                ])],
                config: BTreeMap::from([("active_proxy_group".into(), Value::from("Missing"))]),
            }),
            agents: None,
            sandbox: None,
        };
        assert!(validate_package(&package).is_err());

        let directory = tempfile::tempdir().unwrap();
        let package_path = directory.path().join("invalid-proxy.json");
        fs::write(&package_path, serde_json::to_vec(&package).unwrap()).unwrap();
        assert!(inspect_config_package(&package_path).is_err());

        let destination =
            super::super::MainStore::new(directory.path().join("target.sqlite")).unwrap();
        let runtime = destination.db_runtime().unwrap();
        runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO config (key, value) VALUES ('sentinel', '\"unchanged\"')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(import_config_package(
            &destination,
            &package_path,
            [ConfigCategory::AiModels, ConfigCategory::Proxy],
        )
        .is_err());
        let sentinel: String = runtime
            .read_blocking(|connection| {
                connection
                    .query_row(
                        "SELECT value FROM config WHERE key = 'sentinel'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert_eq!(sentinel, r#""unchanged""#);
    }

    #[test]
    fn package_agent_does_not_delete_same_named_external_builtin() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("agents.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO agents (id, name, system_prompt) VALUES ('custom:incoming', 'Shared name', 'system')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Agents])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        destination_runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO agents (id, name, system_prompt, sandbox_scheme_id) VALUES ('builtin:external', 'Shared name', 'system', 'external-scheme')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO sandbox_schemes (id, name, description, config, disabled) VALUES ('external-scheme', 'External', '', '{}', 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::Agents]).unwrap();
        assert_eq!(result.preserved_agents, 1);
        assert_eq!(result.preserved_sandbox_schemes, 1);
        let (agents, sandbox_scheme_id, sandbox_exists): (Vec<(String, String)>, String, bool) =
            destination_runtime
                .read_blocking(|connection| {
                    let mut statement =
                        connection.prepare("SELECT id, name FROM agents ORDER BY id")?;
                    let agents = statement
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((
                    agents,
                    connection.query_row(
                        "SELECT sandbox_scheme_id FROM agents WHERE id = 'builtin:external'",
                        [],
                        |row| row.get(0),
                    )?,
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sandbox_schemes WHERE id = 'external-scheme')",
                        [],
                        |row| row.get(0),
                    )?,
                ))
                })
                .unwrap();
        assert_eq!(sandbox_scheme_id, "external-scheme");
        assert!(sandbox_exists);
        assert!(agents
            .iter()
            .any(|(id, name)| id == "builtin:external" && name != "Shared name"));
        assert!(agents
            .iter()
            .any(|(id, name)| id == "custom:incoming" && name == "Shared name"));
    }

    #[test]
    fn sandbox_import_remaps_protected_agent_to_same_named_incoming_scheme() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("sandbox-remap.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime
            .write_blocking(|connection| {
                connection.execute("INSERT INTO sandbox_schemes (id, name, description, config, disabled) VALUES ('incoming-scheme', 'Shared', '', '{}', 0)", [])?;
                connection.execute("INSERT INTO agents (id, name, system_prompt, sandbox_scheme_id) VALUES ('incoming', 'Incoming', 'system', 'incoming-scheme')", [])?;
                Ok(())
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Agents])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        destination_runtime
            .write_blocking(|connection| {
                connection.execute("INSERT INTO sandbox_schemes (id, name, description, config, disabled) VALUES ('old-scheme', 'Shared', '', '{}', 0)", [])?;
                connection.execute("INSERT INTO agents (id, name, system_prompt, sandbox_scheme_id) VALUES ('protected', 'Protected', 'system', 'old-scheme')", [])?;
                connection.execute("INSERT INTO workflows (id, user_query, agent_id) VALUES ('workflow', 'query', 'protected')", [])?;
                Ok(())
            })
            .unwrap();
        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::Agents]).unwrap();
        assert_eq!(result.preserved_sandbox_schemes, 0);
        let (scheme, old_scheme_exists): (String, bool) = destination_runtime
            .read_blocking(|connection| {
                Ok((
                    connection.query_row(
                        "SELECT sandbox_scheme_id FROM agents WHERE id = 'protected'",
                        [],
                        |row| row.get(0),
                    )?,
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sandbox_schemes WHERE id = 'old-scheme')",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(scheme, "incoming-scheme");
        assert!(!old_scheme_exists);
    }

    #[test]
    fn later_write_failure_rolls_back_earlier_category_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("rollback.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime
            .write_blocking(|connection| {
                connection.execute("INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (4, 'Incoming', '[]', '', 'openai', 'https://incoming.test', '')", [])?;
                connection.execute("INSERT INTO ai_skill (id, name, prompt) VALUES (5, 'Incoming Skill', 'prompt')", [])?;
                Ok(())
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(
                    connection,
                    export_path,
                    [ConfigCategory::AiModels, ConfigCategory::Skills],
                )
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        destination_runtime
            .write_blocking(|connection| {
                connection.execute("INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (9, 'Local', '[]', '', 'openai', 'https://local.test', '')", [])?;
                connection.execute_batch("CREATE TRIGGER reject_skill_import BEFORE INSERT ON ai_skill BEGIN SELECT RAISE(ABORT, 'test rollback'); END;")?;
                Ok(())
            })
            .unwrap();
        assert!(import_config_package(
            &destination,
            &package_path,
            [ConfigCategory::AiModels, ConfigCategory::Skills]
        )
        .is_err());
        let models: Vec<(i64, String)> = destination_runtime
            .read_blocking(|connection| {
                let mut statement =
                    connection.prepare("SELECT id, name FROM ai_model ORDER BY id")?;
                let models = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(models)
            })
            .unwrap();
        assert_eq!(models, vec![(9, "Local".to_string())]);
    }

    #[test]
    fn legacy_encrypted_api_keys_import_as_locked_across_environments() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("legacy-key.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        let encrypted_key = source_runtime
            .write_blocking(|connection| {
                let encrypted = super::super::api_key_crypto::encrypt_api_key(connection, "secret")?;
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (4, 'Imported', '[]', '', 'openai', 'https://source.test', ?1)",
                    [&encrypted],
                )?;
                Ok(encrypted)
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::AiModels])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        let missing_key_file = directory.path().join("missing-destination-key.csk");
        destination_runtime
            .write_blocking(move |connection| {
                connection.execute(
                    "INSERT OR REPLACE INTO config (key, value) VALUES ('api_key_file', ?1)",
                    [serde_json::to_string(
                        &missing_key_file.display().to_string(),
                    )?],
                )?;
                Ok(())
            })
            .unwrap();
        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::AiModels]).unwrap();
        assert!(result.api_keys_locked);
        let destination_runtime = destination.db_runtime().unwrap();
        let imported_key: String = destination_runtime
            .read_blocking(|connection| {
                connection
                    .query_row("SELECT api_key FROM ai_model WHERE id = 4", [], |row| {
                        row.get(0)
                    })
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert_eq!(imported_key, encrypted_key);
        let config = destination_runtime
            .read_blocking(|connection| super::super::MainStore::load_config(connection))
            .unwrap();
        assert!(config.api_keys_locked);
        assert!(config.ai_models.is_empty());
    }

    #[test]
    fn mixed_v2_and_cross_environment_legacy_api_keys_import_as_locked() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("mixed-keys.json");
        let key_path = directory.path().join("shared.csk");
        super::super::api_key_crypto::generate_key_file(&key_path).unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        let destination_key_path = key_path.clone();
        let v2_key = destination_runtime
            .write_blocking(move |connection| {
                super::super::api_key_crypto::activate_key_file(connection, &destination_key_path)?;
                super::super::api_key_crypto::encrypt_api_key(connection, "v2")
            })
            .unwrap();

        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        let source_v2_key = v2_key.clone();
        let legacy_key = source_runtime
            .write_blocking(move |connection| {
                let encrypted = super::super::api_key_crypto::encrypt_api_key(connection, "legacy")?;
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (4, 'Legacy', '[]', '', 'openai', 'https://legacy.test', ?1)",
                    [&encrypted],
                )?;
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (5, 'V2', '[]', '', 'openai', 'https://v2.test', ?1)",
                    [&source_v2_key],
                )?;
                Ok(encrypted)
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::AiModels])
            })
            .unwrap();

        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::AiModels]).unwrap();
        assert!(result.api_keys_locked);
        let imported_keys = destination_runtime
            .read_blocking(|connection| {
                let mut statement =
                    connection.prepare("SELECT id, api_key FROM ai_model ORDER BY id")?;
                let keys = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(keys)
            })
            .unwrap();
        assert_eq!(imported_keys, vec![(4, legacy_key), (5, v2_key)]);
        let config = destination_runtime
            .read_blocking(|connection| super::super::MainStore::load_config(connection))
            .unwrap();
        assert!(config.api_keys_locked);
        assert!(config.ai_models.is_empty());
    }

    #[test]
    fn agent_import_preserves_workflow_referenced_local_agents() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("agents.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO agents (id, name, system_prompt) VALUES ('incoming', 'Incoming', 'system')", [])?;
            Ok(())
        }).unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Agents])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        destination_runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO agents (id, name, system_prompt) VALUES ('protected', 'Protected', 'system')", [])?;
            connection.execute("INSERT INTO workflows (id, user_query, agent_id) VALUES ('workflow', 'query', 'protected')", [])?;
            Ok(())
        }).unwrap();
        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::Agents]).unwrap();
        assert_eq!(result.preserved_agents, 1);
        let (protected_exists, incoming_exists, workflow_agent): (bool, bool, String) =
            destination_runtime
                .read_blocking(|connection| {
                    Ok((
                        connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = 'protected')",
                            [],
                            |row| row.get(0),
                        )?,
                        connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = 'incoming')",
                            [],
                            |row| row.get(0),
                        )?,
                        connection.query_row(
                            "SELECT agent_id FROM workflows WHERE id = 'workflow'",
                            [],
                            |row| row.get(0),
                        )?,
                    ))
                })
                .unwrap();
        assert!(protected_exists && incoming_exists);
        assert_eq!(workflow_agent, "protected");
    }

    #[test]
    fn all_auto_increment_categories_restore_explicit_ids_and_sequences() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("all-auto.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (11, 'Model', '[]', '', 'openai', 'https://example.test', '')", [])?;
            connection.execute("INSERT INTO ai_skill (id, name, prompt) VALUES (12, 'Skill', 'prompt')", [])?;
            connection.execute("INSERT INTO mcp (id, name, description, config, disabled) VALUES (13, 'Server', 'MCP', '{\"name\":\"Server\",\"type\":\"stdio\"}', 0)", [])?;
            connection.execute("INSERT INTO proxy_group (id, name, description, prompt_injection, prompt_text, tool_filter, temperature, disabled) VALUES (14, 'Proxy', '', '', '', '', 1.0, 0)", [])?;
            Ok(())
        }).unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(
                    connection,
                    export_path,
                    [
                        ConfigCategory::AiModels,
                        ConfigCategory::Skills,
                        ConfigCategory::Mcp,
                        ConfigCategory::Proxy,
                    ],
                )
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        import_config_package(
            &destination,
            &package_path,
            [
                ConfigCategory::AiModels,
                ConfigCategory::Skills,
                ConfigCategory::Mcp,
                ConfigCategory::Proxy,
            ],
        )
        .unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        let sequences: Vec<(String, i64)> = destination_runtime.read_blocking(|connection| {
            let mut statement = connection.prepare("SELECT name, seq FROM sqlite_sequence WHERE name IN ('ai_model', 'ai_skill', 'mcp', 'proxy_group') ORDER BY name")?;
            let sequences = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(sequences)
        }).unwrap();
        assert_eq!(
            sequences,
            vec![
                ("ai_model".into(), 11),
                ("ai_skill".into(), 12),
                ("mcp".into(), 13),
                ("proxy_group".into(), 14),
            ]
        );
    }

    #[test]
    fn proxy_import_covers_whitelisted_keys_and_preserves_general_network_proxy() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("proxy.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (3, 'Provider', '[]', '', 'openai', 'https://example.test', '')", [])?;
            connection.execute("INSERT INTO proxy_group (id, name, description, prompt_injection, prompt_text, tool_filter, temperature, disabled) VALUES (8, 'Proxy', '', '', '', '', 1.0, 0)", [])?;
            for (key, value) in [
                ("chat_completion_proxy", r#"{"default":{"targets":[{"id":3,"model":"gpt"}]}}"#),
                ("chat_completion_proxy_keys", r#"[{"name":"Test","token":"token"}]"#),
                ("active_proxy_group", r#""Proxy""#),
                ("chat_completion_proxy_port", "9527"),
                ("chat_completion_proxy_listen", r#""127.0.0.1""#),
                ("chat_completion_proxy_log_to_file", "true"),
                ("chat_completion_proxy_log_proxy_to_file", "false"),
                ("chat_completion_proxy_retry_on_429", "1"),
                ("proxy_type", r#""source-system-proxy""#),
            ] { connection.execute("INSERT INTO config (key, value) VALUES (?1, ?2)", params![key, value])?; }
            Ok(())
        }).unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Proxy])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let destination_runtime = destination.db_runtime().unwrap();
        destination_runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO config (key, value) VALUES ('proxy_type', '\"local-system-proxy\"')", [])?;
            Ok(())
        }).unwrap();
        import_config_package(&destination, &package_path, [ConfigCategory::Proxy]).unwrap();
        let (proxy_id, sequence, token, network_proxy): (i64, i64, String, String) =
            destination_runtime
                .read_blocking(|connection| {
                    Ok((
                        connection.query_row("SELECT id FROM proxy_group", [], |row| row.get(0))?,
                        connection.query_row(
                            "SELECT seq FROM sqlite_sequence WHERE name = 'proxy_group'",
                            [],
                            |row| row.get(0),
                        )?,
                        connection.query_row(
                            "SELECT value FROM config WHERE key = 'chat_completion_proxy_keys'",
                            [],
                            |row| row.get(0),
                        )?,
                        connection.query_row(
                            "SELECT value FROM config WHERE key = 'proxy_type'",
                            [],
                            |row| row.get(0),
                        )?,
                    ))
                })
                .unwrap();
        assert_eq!((proxy_id, sequence), (8, 8));
        assert_eq!(token, r#"[{"name":"Test","token":"token"}]"#);
        assert_eq!(network_proxy, r#""local-system-proxy""#);
    }

    #[test]
    fn valid_mcp_tool_ids_are_accepted_and_imported_with_agents() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("agents-with-mcp.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime.write_blocking(|connection| {
            connection.execute(
                "INSERT INTO mcp (id, name, description, config, disabled) VALUES (7, 'server', 'MCP', ?1, 0)",
                [r#"{"name":"server","type":"stdio","command":"server"}"#],
            )?;
            connection.execute(
                "INSERT INTO agents (id, name, system_prompt, mcp_tool_exposure) VALUES ('agent', 'Agent', 'system', ?1)",
                [r#"["server__MCP__tool"]"#],
            )?;
            Ok(())
        }).unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Agents])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let result =
            import_config_package(&destination, &package_path, [ConfigCategory::Agents]).unwrap();
        assert_eq!(result.imported_counts.get(&ConfigCategory::Mcp), Some(&1));
        let destination_runtime = destination.db_runtime().unwrap();
        let exposure: String = destination_runtime
            .read_blocking(|connection| {
                connection
                    .query_row(
                        "SELECT mcp_tool_exposure FROM agents WHERE id = 'agent'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert_eq!(exposure, r#"["server__MCP__tool"]"#);
    }

    #[test]
    fn ai_only_import_rejects_retained_proxy_target_to_removed_model() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("ai-only.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (4, 'Imported', '[]', '', 'openai', 'https://source.test', '')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::AiModels])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let runtime = destination.db_runtime().unwrap();
        runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (9, 'Local', '[]', '', 'openai', 'https://local.test', '')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO config (key, value) VALUES ('chat_completion_proxy', ?1)",
                    [r#"{"default":{"alias":[{"id":9,"model":"local"}]}}"#],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            import_config_package(&destination, &package_path, [ConfigCategory::AiModels]).is_err()
        );
        let (model_exists, proxy): (bool, String) = runtime
            .read_blocking(|connection| {
                Ok((
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM ai_model WHERE id = 9)",
                        [],
                        |row| row.get(0),
                    )?,
                    connection.query_row(
                        "SELECT value FROM config WHERE key = 'chat_completion_proxy'",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert!(model_exists);
        assert!(proxy.contains("\"id\":9"));
    }

    #[test]
    fn agents_import_rejects_protected_agent_with_removed_model() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let package_path = directory.path().join("agents.json");
        let source = super::super::MainStore::new(&source_path).unwrap();
        let source_runtime = source.db_runtime().unwrap();
        source_runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (4, 'Imported', '[]', '', 'openai', 'https://source.test', '')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO agents (id, name, system_prompt) VALUES ('incoming', 'Incoming', 'system')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let export_path = package_path.clone();
        source_runtime
            .read_blocking(move |connection| {
                export_config_package(connection, export_path, [ConfigCategory::Agents])
            })
            .unwrap();

        let destination = super::super::MainStore::new(&destination_path).unwrap();
        let runtime = destination.db_runtime().unwrap();
        runtime
            .write_blocking(|connection| {
                connection.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_protocol, base_url, api_key) VALUES (9, 'Local', '[]', '', 'openai', 'https://local.test', '')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO agents (id, name, system_prompt, models) VALUES ('protected', 'Protected', 'system', ?1)",
                    [r#"{"plan":{"id":9,"model":"local"}}"#],
                )?;
                connection.execute(
                    "INSERT INTO workflows (id, user_query, agent_id) VALUES ('workflow', 'query', 'protected')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            import_config_package(&destination, &package_path, [ConfigCategory::Agents]).is_err()
        );
        let (model_exists, agent_exists): (bool, bool) = runtime
            .read_blocking(|connection| {
                Ok((
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM ai_model WHERE id = 9)",
                        [],
                        |row| row.get(0),
                    )?,
                    connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM agents WHERE id = 'protected')",
                        [],
                        |row| row.get(0),
                    )?,
                ))
            })
            .unwrap();
        assert!(model_exists && agent_exists);
    }

    #[test]
    fn malformed_agent_models_and_unknown_package_fields_fail_before_import() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("config-transfer.sqlite");
        let package_path = directory.path().join("invalid.json");
        let store = super::super::MainStore::new(&database_path).unwrap();
        let runtime = store.db_runtime().unwrap();
        runtime.write_blocking(|connection| {
            connection.execute("INSERT INTO ai_model (name, models, default_model, api_protocol, base_url, api_key) VALUES ('Local', '[]', '', 'openai', 'https://local.test', '')", [])?;
            Ok(())
        }).unwrap();
        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: category_closure([ConfigCategory::Agents])
                .into_iter()
                .collect(),
            ai_models: Some(TablePayload::default()),
            skills: Some(TablePayload::default()),
            mcp: Some(TablePayload::default()),
            proxy: None,
            agents: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from("agent")),
                    ("name".into(), Value::from("Agent")),
                    ("system_prompt".into(), Value::from("system")),
                    ("models".into(), Value::from(r#"{"plan":{"id":"wrong"}}"#)),
                ])],
                config: BTreeMap::new(),
            }),
            sandbox: Some(TablePayload::default()),
        };
        fs::write(&package_path, serde_json::to_vec(&package).unwrap()).unwrap();
        assert!(import_config_package(&store, &package_path, [ConfigCategory::Agents]).is_err());
        let model_count: i64 = runtime
            .read_blocking(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM ai_model WHERE name = 'Local'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert_eq!(model_count, 1);

        let mut value = serde_json::to_value(package).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), Value::Bool(true));
        assert!(serde_json::from_value::<ConfigTransferPackage>(value).is_err());
    }

    #[test]
    fn invalid_mcp_tool_id_is_rejected_during_inspection() {
        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: category_closure([ConfigCategory::Agents])
                .into_iter()
                .collect(),
            ai_models: Some(TablePayload::default()),
            skills: Some(TablePayload::default()),
            mcp: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from(1)),
                    ("name".into(), Value::from("server")),
                    ("description".into(), Value::from("MCP")),
                    (
                        "config".into(),
                        Value::from(r#"{"name":"server","type":"stdio"}"#),
                    ),
                ])],
                config: BTreeMap::new(),
            }),
            proxy: None,
            agents: Some(TablePayload {
                rows: vec![BTreeMap::from([
                    ("id".into(), Value::from("agent")),
                    ("name".into(), Value::from("Agent")),
                    ("system_prompt".into(), Value::from("system")),
                    ("mcp_tool_exposure".into(), Value::from(r#"["server"]"#)),
                ])],
                config: BTreeMap::new(),
            }),
            sandbox: Some(TablePayload::default()),
        };
        assert!(validate_package(&package).is_err());
    }

    #[test]
    fn sandbox_protection_distinguishes_agent_replacement() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents (id TEXT PRIMARY KEY, sandbox_scheme_id TEXT);
             INSERT INTO agents VALUES ('retained', 'retained-scheme');
             INSERT INTO agents VALUES ('removed', 'removed-scheme');",
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        let retained_agents = BTreeSet::from(["retained".to_string()]);
        assert_eq!(
            protected_sandbox_ids(&transaction, &retained_agents, true).unwrap(),
            BTreeSet::from(["retained-scheme".to_string()]),
        );
        assert_eq!(
            protected_sandbox_ids(&transaction, &BTreeSet::new(), false).unwrap(),
            BTreeSet::from(["removed-scheme".to_string(), "retained-scheme".to_string()]),
        );
    }

    #[test]
    fn invalid_safe_integer_is_rejected() {
        let package = ConfigTransferPackage {
            format_version: FORMAT_VERSION,
            exported_at: "now".into(),
            categories: vec![ConfigCategory::AiModels],
            ai_models: Some(TablePayload {
                rows: vec![BTreeMap::from([(
                    "id".into(),
                    Value::from(MAX_SAFE_INTEGER + 1),
                )])],
                config: BTreeMap::new(),
            }),
            skills: None,
            mcp: None,
            proxy: None,
            agents: None,
            sandbox: None,
        };
        assert!(validate_package(&package).is_err());
    }
}
