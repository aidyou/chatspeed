//! Built-in agent synchronization from bundled assets.

use crate::constants::CFG_BUILTIN_AGENTS_LAST_SYNCED_APP_VERSION;
use crate::db::agent::{AgentModels, ShellPolicyRule};
use crate::db::{Agent, MainStore};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tauri::AppHandle;

const BUILTIN_AGENT_ID_PREFIX: &str = "builtin:";
const BUILTIN_AGENTS_DIR: &str = "agents";
const BUILTIN_AGENT_SCHEMA_VERSION: i32 = 1;
const DEFAULT_SHELL_POLICY_FILE: &str = "default-shell-policy.json";

#[derive(Debug, Deserialize)]
struct BuiltinAgentManifest {
    schema_version: i32,
    builtin_id: String,
    builtin_version: i32,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    role: BuiltinAgentRole,
    #[serde(default)]
    parent_builtin_id: Option<String>,
    prompts: BuiltinAgentPrompts,
    #[serde(default)]
    config: BuiltinAgentConfig,
    #[serde(default)]
    disabled: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BuiltinAgentRole {
    #[default]
    Primary,
    Child,
}

impl BuiltinAgentRole {
    fn as_db_role(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Child => "child",
        }
    }
}

#[derive(Debug, Deserialize)]
struct BuiltinAgentPrompts {
    system: String,
    #[serde(default)]
    planning: Option<String>,
    #[serde(default)]
    image_recognition: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BuiltinAgentConfig {
    #[serde(default)]
    allowed_paths: Option<Vec<String>>,
    #[serde(default)]
    shell_policy: Option<BuiltinShellPolicyConfig>,
    #[serde(default)]
    approval_level: Option<String>,
    #[serde(default)]
    auto_approve: Option<Vec<String>>,
    #[serde(default)]
    available_tools: Option<Vec<String>>,
    #[serde(default)]
    final_audit: Option<bool>,
    #[serde(default)]
    skill_enabled: Option<bool>,
    #[serde(default)]
    selected_skills: Option<Vec<String>>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    models: Option<AgentModels>,
    #[serde(default)]
    max_contexts: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum BuiltinShellPolicyConfig {
    Rules(Vec<ShellPolicyRule>),
    Mode(String),
}

#[derive(Debug)]
struct BuiltinAgentDefinition {
    manifest: BuiltinAgentManifest,
    system_prompt: String,
    planning_prompt: Option<String>,
    image_recognition_prompt: Option<String>,
}

fn builtin_agent_db_id(builtin_id: &str) -> String {
    format!("{}{}", BUILTIN_AGENT_ID_PREFIX, builtin_id)
}

fn serialize_json<T: serde::Serialize>(value: &Option<T>) -> Option<String> {
    value.as_ref().and_then(|v| serde_json::to_string(v).ok())
}

fn read_prompt_file(base_dir: &Path, relative_path: &str) -> Result<String, String> {
    let path = base_dir.join(relative_path);
    fs::read_to_string(&path)
        .map(|content| {
            content
                .strip_prefix('\u{feff}')
                .unwrap_or(&content)
                .to_string()
        })
        .map_err(|e| format!("failed to read prompt file {:?}: {}", path, e))
}

fn read_default_shell_policy(
    builtin_agents_root: &Path,
) -> Result<Option<Vec<ShellPolicyRule>>, String> {
    let path = builtin_agents_root.join(DEFAULT_SHELL_POLICY_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read default shell policy {:?}: {}", path, e))?;
    let policy = serde_json::from_str::<Vec<ShellPolicyRule>>(&content)
        .map_err(|e| format!("failed to parse default shell policy {:?}: {}", path, e))?;
    Ok(Some(policy))
}

pub fn load_default_shell_policy_from_resources() -> Result<Vec<ShellPolicyRule>, String> {
    let builtin_agents_root = crate::RESOURCE_DIR.read().clone().join(BUILTIN_AGENTS_DIR);
    Ok(read_default_shell_policy(&builtin_agents_root)?.unwrap_or_default())
}

fn resolve_shell_policy_config(
    config: Option<&BuiltinShellPolicyConfig>,
    default_shell_policy: Option<&Vec<ShellPolicyRule>>,
) -> Result<Option<Vec<ShellPolicyRule>>, String> {
    match config {
        Some(BuiltinShellPolicyConfig::Rules(rules)) => Ok(Some(rules.clone())),
        Some(BuiltinShellPolicyConfig::Mode(mode)) => match mode.as_str() {
            "default" => Ok(default_shell_policy.cloned()),
            "none" => Ok(Some(Vec::new())),
            other => Err(format!("unsupported builtin shell_policy mode '{}'", other)),
        },
        None => Ok(None),
    }
}

fn load_builtin_agent_definition(agent_dir: &Path) -> Result<BuiltinAgentDefinition, String> {
    let manifest_path = agent_dir.join("agent.yaml");
    let manifest_str = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read manifest {:?}: {}", manifest_path, e))?;
    let manifest: BuiltinAgentManifest = serde_yaml::from_str(&manifest_str)
        .map_err(|e| format!("failed to parse manifest {:?}: {}", manifest_path, e))?;

    if manifest.schema_version != BUILTIN_AGENT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} in {:?}",
            manifest.schema_version, manifest_path
        ));
    }

    let system_prompt = read_prompt_file(agent_dir, &manifest.prompts.system)?;
    let planning_prompt = manifest
        .prompts
        .planning
        .as_deref()
        .map(|path| read_prompt_file(agent_dir, path))
        .transpose()?;
    let image_recognition_prompt = manifest
        .prompts
        .image_recognition
        .as_deref()
        .map(|path| read_prompt_file(agent_dir, path))
        .transpose()?;

    Ok(BuiltinAgentDefinition {
        manifest,
        system_prompt,
        planning_prompt,
        image_recognition_prompt,
    })
}

fn scan_builtin_agents(root: &Path) -> Result<Vec<BuiltinAgentDefinition>, String> {
    if !root.exists() {
        return Ok(vec![]);
    }

    let mut definitions = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|e| format!("failed to read builtin agents dir {:?}: {}", root, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read builtin agent entry: {}", e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        definitions.push(load_builtin_agent_definition(&path)?);
    }

    definitions.sort_by(|a, b| {
        let a_child = a.manifest.parent_builtin_id.is_some();
        let b_child = b.manifest.parent_builtin_id.is_some();
        a_child
            .cmp(&b_child)
            .then_with(|| a.manifest.builtin_id.cmp(&b.manifest.builtin_id))
    });
    Ok(definitions)
}

fn builtin_name_conflicts(store: &MainStore, agent_id: &str, name: &str) -> Result<bool, String> {
    let agents = store.get_all_agents().map_err(|e| e.to_string())?;
    Ok(agents
        .iter()
        .any(|agent| agent.id != agent_id && agent.name.eq_ignore_ascii_case(name)))
}

fn resolve_builtin_name(
    store: &MainStore,
    agent_id: &str,
    preferred_name: &str,
) -> Result<String, String> {
    if !builtin_name_conflicts(store, agent_id, preferred_name)? {
        return Ok(preferred_name.to_string());
    }

    let mut candidate = format!("{} Built-in", preferred_name);
    if !builtin_name_conflicts(store, agent_id, &candidate)? {
        return Ok(candidate);
    }

    for index in 2..100 {
        candidate = format!("{} Built-in {}", preferred_name, index);
        if !builtin_name_conflicts(store, agent_id, &candidate)? {
            return Ok(candidate);
        }
    }

    Err(format!(
        "failed to resolve a unique name for builtin agent '{}'",
        preferred_name
    ))
}

fn definition_to_agent(
    definition: &BuiltinAgentDefinition,
    default_shell_policy: Option<&Vec<ShellPolicyRule>>,
) -> Result<Agent, String> {
    let manifest = &definition.manifest;
    Ok(Agent {
        id: builtin_agent_db_id(&manifest.builtin_id),
        name: manifest.name.clone(),
        description: if manifest.description.trim().is_empty() {
            None
        } else {
            Some(manifest.description.clone())
        },
        role: Some(manifest.role.as_db_role().to_string()),
        parent_agent_id: manifest
            .parent_builtin_id
            .as_deref()
            .map(builtin_agent_db_id),
        system_prompt: definition.system_prompt.clone(),
        planning_prompt: definition.planning_prompt.clone(),
        image_recognition_prompt: definition.image_recognition_prompt.clone(),
        available_tools: serialize_json(&manifest.config.available_tools),
        auto_approve: serialize_json(&manifest.config.auto_approve),
        models: manifest.config.models.clone(),
        shell_policy: serialize_json(&resolve_shell_policy_config(
            manifest.config.shell_policy.as_ref(),
            default_shell_policy,
        )?),
        allowed_paths: serialize_json(&manifest.config.allowed_paths),
        final_audit: manifest.config.final_audit,
        approval_level: manifest.config.approval_level.clone(),
        skill_enabled: manifest.config.skill_enabled,
        selected_skills: serialize_json(&manifest.config.selected_skills),
        phase: manifest.config.phase.clone(),
        is_system: Some(true),
        disabled: Some(manifest.disabled),
        version: Some(manifest.builtin_version),
        sort_index: None,
        max_contexts: manifest.config.max_contexts,
        created_at: None,
        updated_at: None,
    })
}

fn sync_single_builtin_agent(
    store: &MainStore,
    definition: &BuiltinAgentDefinition,
    default_shell_policy: Option<&Vec<ShellPolicyRule>>,
) -> Result<(), String> {
    let desired = definition_to_agent(definition, default_shell_policy)?;
    let existing = store.get_agent(&desired.id).map_err(|e| e.to_string())?;

    match existing {
        None => {
            let mut created = desired;
            created.name = resolve_builtin_name(store, &created.id, &created.name)?;
            store.add_agent(&created).map_err(|e| e.to_string())?;
        }
        Some(current) => {
            if !current.is_system.unwrap_or(false) {
                return Err(format!(
                    "builtin agent id '{}' already exists as a non-system agent",
                    desired.id
                ));
            }

            let current_version = current.version.unwrap_or(0);
            if current_version >= definition.manifest.builtin_version {
                return Ok(());
            }

            let mut updated = current;
            updated.name = resolve_builtin_name(store, &updated.id, &desired.name)?;
            updated.description = desired.description;
            updated.role = desired.role;
            updated.parent_agent_id = desired.parent_agent_id;
            updated.system_prompt = desired.system_prompt;
            updated.planning_prompt = desired.planning_prompt;
            updated.image_recognition_prompt = desired.image_recognition_prompt;
            updated.is_system = Some(true);
            updated.version = Some(definition.manifest.builtin_version);
            store.update_agent(&updated).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

pub fn sync_builtin_agents_if_needed(
    app: &AppHandle,
    main_store: Arc<RwLock<MainStore>>,
) -> Result<(), String> {
    let current_app_version = app.package_info().version.to_string();

    #[cfg(not(debug_assertions))]
    {
        let last_synced_version = main_store
            .read()
            .map_err(|e| e.to_string())?
            .get_config(CFG_BUILTIN_AGENTS_LAST_SYNCED_APP_VERSION, String::new());

        if last_synced_version == current_app_version {
            return Ok(());
        }
    }

    let resource_dir = crate::RESOURCE_DIR.read().clone();
    let builtin_agents_root: PathBuf = resource_dir.join(BUILTIN_AGENTS_DIR);
    let definitions = scan_builtin_agents(&builtin_agents_root)?;
    let default_shell_policy = read_default_shell_policy(&builtin_agents_root)?;

    if definitions.is_empty() {
        log::info!(
            "No builtin agents found under {:?}; skipping sync",
            builtin_agents_root
        );
        return Ok(());
    }

    let mut store = main_store.write().map_err(|e| e.to_string())?;
    for definition in &definitions {
        sync_single_builtin_agent(&store, definition, default_shell_policy.as_ref())?;
    }
    store
        .set_config(
            CFG_BUILTIN_AGENTS_LAST_SYNCED_APP_VERSION,
            &json!(current_app_version),
        )
        .map_err(|e| e.to_string())?;

    log::info!(
        "Builtin agents synchronized from {:?} for app version {}",
        builtin_agents_root,
        current_app_version
    );
    Ok(())
}
