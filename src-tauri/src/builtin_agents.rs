//! Built-in agent synchronization from bundled assets.

use crate::constants::CFG_BUILTIN_AGENTS_LAST_SYNCED_APP_VERSION;
use crate::db::agent::{AgentModels, ShellPolicyRule};
use crate::db::{Agent, MainStore};
use crate::tools::MCP_TOOL_NAME_SPLIT;
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

fn merge_builtin_tools_with_existing_mcp(
    builtin_tools: Option<String>,
    existing_tools: Option<&str>,
) -> Option<String> {
    let existing_mcp_tools = existing_tools
        .and_then(|tools| serde_json::from_str::<Vec<String>>(tools).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| tool.contains(MCP_TOOL_NAME_SPLIT))
        .collect::<Vec<_>>();

    if existing_mcp_tools.is_empty() {
        return builtin_tools;
    }

    match builtin_tools {
        Some(tools) => {
            let Ok(mut merged_tools) = serde_json::from_str::<Vec<String>>(&tools) else {
                return Some(tools);
            };
            for mcp_tool in existing_mcp_tools {
                if !merged_tools.contains(&mcp_tool) {
                    merged_tools.push(mcp_tool);
                }
            }
            serde_json::to_string(&merged_tools).ok()
        }
        None => None,
    }
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

fn resolve_builtin_agents_root() -> Option<PathBuf> {
    let candidates = crate::constants::resolve_resource_subdirs(BUILTIN_AGENTS_DIR);
    let existing = candidates.iter().find(|path| path.exists()).cloned();
    let selected = existing.or_else(|| candidates.first().cloned());

    if let Some(path) = &selected {
        log::debug!("Builtin agents candidate selected: {:?}", path);
    }

    selected
}

pub fn load_default_shell_policy_from_resources() -> Result<Vec<ShellPolicyRule>, String> {
    let builtin_agents_root = match resolve_builtin_agents_root() {
        Some(path) => path,
        None => return Ok(Vec::new()),
    };
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
        mcp_tool_exposure: None,
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

            let available_tools = merge_builtin_tools_with_existing_mcp(
                desired.available_tools,
                current.available_tools.as_deref(),
            );
            let mut updated = current;
            updated.role = desired.role;
            updated.parent_agent_id = desired.parent_agent_id;
            updated.system_prompt = desired.system_prompt;
            updated.planning_prompt = desired.planning_prompt;
            updated.image_recognition_prompt = desired.image_recognition_prompt;
            updated.available_tools = available_tools;
            updated.auto_approve = desired.auto_approve;
            updated.is_system = Some(true);
            updated.version = Some(definition.manifest.builtin_version);
            store.update_agent(&updated).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        builtin_agent_db_id, sync_single_builtin_agent, BuiltinAgentConfig, BuiltinAgentDefinition,
        BuiltinAgentManifest, BuiltinAgentPrompts, BuiltinAgentRole,
    };
    use crate::db::agent::{AgentModels, ModelConfig};
    use crate::db::{Agent, MainStore};

    #[test]
    fn builtin_upgrade_preserves_user_configuration_and_updates_core_definition() {
        let store = MainStore::new(":memory:").expect("in-memory store");
        let configured_models = AgentModels {
            plan: Some(ModelConfig {
                id: 42,
                model: "user-configured-model".to_string(),
                temperature: Some(0.7),
                thinking: None,
                function_call: None,
                context_size: None,
                max_tokens: None,
            }),
            act: None,
            vision: None,
            utility: None,
        };
        let mut existing = Agent::new(
            builtin_agent_db_id("test-child"),
            "User Child Name".to_string(),
            Some("user description".to_string()),
            Some("child".to_string()),
            None,
            "old prompt".to_string(),
            None,
            None,
            Some(format!(
                "[\"user-tool\",\"example{}search\"]",
                crate::tools::MCP_TOOL_NAME_SPLIT
            )),
            Some("[\"user-auto-approve\"]".to_string()),
            Some(configured_models),
            Some("[\"user-shell-policy\"]".to_string()),
            Some("[\"/user/path\"]".to_string()),
            Some(true),
            Some("full".to_string()),
            Some(true),
            Some("[\"user-skill\"]".to_string()),
            Some("planning".to_string()),
            Some(true),
            Some(false),
            Some(64000),
        );
        existing.version = Some(2);
        let mut parent = existing.clone();
        parent.id = builtin_agent_db_id("test-parent");
        parent.name = "Test Parent".to_string();
        parent.role = Some("primary".to_string());
        parent.version = Some(3);
        store.add_agent(&parent).expect("seed parent builtin agent");
        store.add_agent(&existing).expect("seed builtin agent");

        let definition = BuiltinAgentDefinition {
            manifest: BuiltinAgentManifest {
                schema_version: 1,
                builtin_id: "test-child".to_string(),
                builtin_version: 3,
                name: "Updated Child".to_string(),
                description: "updated description".to_string(),
                role: BuiltinAgentRole::Child,
                parent_builtin_id: Some("test-parent".to_string()),
                prompts: BuiltinAgentPrompts {
                    system: "system.md".to_string(),
                    planning: Some("planning.md".to_string()),
                    image_recognition: Some("image.md".to_string()),
                },
                config: BuiltinAgentConfig {
                    allowed_paths: Some(vec!["/manifest/path".to_string()]),
                    shell_policy: Some(super::BuiltinShellPolicyConfig::Mode("none".to_string())),
                    approval_level: Some("default".to_string()),
                    auto_approve: Some(vec![crate::tools::TOOL_GIT_DIFF.to_string()]),
                    available_tools: Some(vec![crate::tools::TOOL_GIT_INSPECT.to_string()]),
                    final_audit: Some(false),
                    skill_enabled: Some(false),
                    selected_skills: Some(vec!["manifest-skill".to_string()]),
                    phase: Some("standard".to_string()),
                    max_contexts: Some(128000),
                    ..Default::default()
                },
                disabled: true,
            },
            system_prompt: "new prompt".to_string(),
            planning_prompt: Some("new planning prompt".to_string()),
            image_recognition_prompt: Some("new image prompt".to_string()),
        };

        sync_single_builtin_agent(&store, &definition, None).expect("sync builtin agent");
        let updated = store
            .get_agent(&builtin_agent_db_id("test-child"))
            .expect("load agent")
            .expect("agent exists");
        assert_eq!(updated.version, Some(3));
        assert_eq!(updated.name, "User Child Name");
        assert_eq!(updated.description.as_deref(), Some("user description"));
        assert_eq!(updated.role.as_deref(), Some("child"));
        assert_eq!(
            updated.parent_agent_id.as_deref(),
            Some("builtin:test-parent")
        );
        assert_eq!(updated.system_prompt, "new prompt");
        assert_eq!(
            updated.planning_prompt.as_deref(),
            Some("new planning prompt")
        );
        assert_eq!(
            updated.image_recognition_prompt.as_deref(),
            Some("new image prompt")
        );
        assert_eq!(updated.disabled, Some(false));
        assert_eq!(updated.final_audit, Some(true));
        assert_eq!(updated.approval_level.as_deref(), Some("full"));
        assert_eq!(updated.skill_enabled, Some(true));
        assert_eq!(updated.selected_skills.as_deref(), Some("[\"user-skill\"]"));
        assert_eq!(updated.phase.as_deref(), Some("planning"));
        assert_eq!(updated.max_contexts, Some(64000));
        assert_eq!(
            updated.shell_policy.as_deref(),
            Some("[\"user-shell-policy\"]")
        );
        assert_eq!(updated.allowed_paths.as_deref(), Some("[\"/user/path\"]"));
        assert_eq!(
            updated
                .models
                .and_then(|models| models.plan)
                .map(|model| model.model),
            Some("user-configured-model".to_string())
        );
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&updated.available_tools.expect("tools"))
                .expect("tools json"),
            vec![
                crate::tools::TOOL_GIT_INSPECT.to_string(),
                format!("example{}search", crate::tools::MCP_TOOL_NAME_SPLIT),
            ]
        );
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&updated.auto_approve.expect("auto approve"))
                .expect("auto approve json"),
            vec![crate::tools::TOOL_GIT_DIFF.to_string()]
        );
    }

    #[test]
    fn builtin_creation_uses_manifest_disabled_and_models_defaults() {
        let store = MainStore::new(":memory:").expect("in-memory store");
        let definition = BuiltinAgentDefinition {
            manifest: BuiltinAgentManifest {
                schema_version: 1,
                builtin_id: "new-child".to_string(),
                builtin_version: 1,
                name: "New Child".to_string(),
                description: String::new(),
                role: BuiltinAgentRole::Child,
                parent_builtin_id: None,
                prompts: BuiltinAgentPrompts {
                    system: "system.md".to_string(),
                    planning: None,
                    image_recognition: None,
                },
                config: BuiltinAgentConfig {
                    models: Some(AgentModels {
                        plan: Some(ModelConfig {
                            id: 7,
                            model: "manifest-default-model".to_string(),
                            temperature: None,
                            thinking: None,
                            function_call: None,
                            context_size: None,
                            max_tokens: None,
                        }),
                        act: None,
                        vision: None,
                        utility: None,
                    }),
                    ..Default::default()
                },
                disabled: true,
            },
            system_prompt: "new prompt".to_string(),
            planning_prompt: None,
            image_recognition_prompt: None,
        };

        sync_single_builtin_agent(&store, &definition, None).expect("create builtin agent");
        let created = store
            .get_agent(&builtin_agent_db_id("new-child"))
            .expect("load agent")
            .expect("agent exists");
        assert_eq!(created.disabled, Some(true));
        assert_eq!(
            created
                .models
                .and_then(|models| models.plan)
                .map(|model| model.model),
            Some("manifest-default-model".to_string())
        );
    }
}

pub fn sync_builtin_agents_if_needed(
    app: &AppHandle,
    main_store: Arc<RwLock<MainStore>>,
) -> Result<(), String> {
    let current_app_version = app.package_info().version.to_string();

    let builtin_agents_root = match resolve_builtin_agents_root() {
        Some(path) => path,
        None => {
            log::info!("Builtin agents sync skipped: resource directory is not available");
            return Ok(());
        }
    };
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
