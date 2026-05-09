//! Agent database operations
//!
//! This module provides database operations for managing ReAct agents.

use crate::db::ThinkingConfig;
use crate::db::{MainStore, StoreError};
use rusqlite::{params, OptionalExtension, Row};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

// Re-export ShellPolicyRule for backward compatibility
pub use crate::tools::ShellPolicyRule;

/// Agent runtime configuration stored in workflow sessions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub allowed_paths: Option<Vec<String>>,
    pub shell_policy: Option<Vec<ShellPolicyRule>>,
    pub approval_level: Option<String>,
    pub auto_approve: Option<Vec<String>>,
    pub auto_compress: Option<bool>,
    pub available_tools: Option<Vec<String>>,
    pub final_audit: Option<bool>,
    pub skill_enabled: Option<bool>,
    pub selected_skills: Option<Vec<String>>,
    pub phase: Option<String>,
    pub models: Option<AgentModels>,
    pub max_contexts: Option<i32>,
}

impl AgentConfig {
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn apply_overrides(&mut self, other: &AgentConfig) {
        if other.allowed_paths.is_some() {
            self.allowed_paths = other.allowed_paths.clone();
        }
        if other.shell_policy.is_some() {
            self.shell_policy = other.shell_policy.clone();
        }
        if other.approval_level.is_some() {
            self.approval_level = other.approval_level.clone();
        }
        if other.auto_approve.is_some() {
            self.auto_approve = other.auto_approve.clone();
        }
        if other.auto_compress.is_some() {
            self.auto_compress = other.auto_compress;
        }
        if other.available_tools.is_some() {
            self.available_tools = other.available_tools.clone();
        }
        if other.final_audit.is_some() {
            self.final_audit = other.final_audit;
        }
        if other.skill_enabled.is_some() {
            self.skill_enabled = other.skill_enabled;
        }
        if other.selected_skills.is_some() {
            self.selected_skills = other.selected_skills.clone();
        }
        if other.phase.is_some() {
            self.phase = other.phase.clone();
        }
        if other.models.is_some() {
            self.models = other.models.clone();
        }
        if other.max_contexts.is_some() {
            self.max_contexts = other.max_contexts;
        }
    }
}

/// Model configuration within an Agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: i64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentModels {
    pub plan: Option<ModelConfig>,
    pub act: Option<ModelConfig>,
    pub utility: Option<ModelConfig>,
}

/// Represents an AI agent for ReAct workflows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier for the agent
    pub id: String,
    /// Name of the agent
    pub name: String,
    /// Description of the agent
    pub description: Option<String>,
    /// Agent role in the hierarchy (`primary` or `child`)
    pub role: Option<String>,
    /// Parent agent ID when this agent is a child agent
    pub parent_agent_id: Option<String>,
    /// System prompt for the agent
    pub system_prompt: String,
    /// Prompt for the planning phase
    pub planning_prompt: Option<String>,
    /// JSON array of available tool IDs
    pub available_tools: Option<String>,
    /// JSON array of tools that can be executed without user confirmation
    pub auto_approve: Option<String>,
    /// Unified models configuration (JSON string)
    pub models: Option<AgentModels>,
    /// Shell command whitelist/blacklist (JSON array of {pattern, decision})
    pub shell_policy: Option<String>,
    /// JSON array of authorized directory paths
    pub allowed_paths: Option<String>,
    /// Whether the agent's tasks require final audit
    pub final_audit: Option<bool>,
    /// Approval level for tool calls (default, smart, full)
    pub approval_level: Option<String>,
    /// Whether skills are enabled for this agent
    pub skill_enabled: Option<bool>,
    /// JSON array of enabled skill names for this agent
    pub selected_skills: Option<String>,
    /// Default workflow phase for this agent
    pub phase: Option<String>,
    /// Whether this agent is a built-in system agent
    pub is_system: Option<bool>,
    /// Whether this agent is disabled from workflow selection/delegation
    pub disabled: Option<bool>,
    /// Built-in definition version used for system agent upgrades
    pub version: Option<i32>,
    /// UI ordering index for agent lists
    pub sort_index: Option<i32>,
    /// Maximum context length (in tokens)
    pub max_contexts: Option<i32>,
    /// Creation timestamp
    pub created_at: Option<String>,
    /// Last update timestamp
    pub updated_at: Option<String>,
}

impl Agent {
    /// Creates a new agent
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        name: String,
        description: Option<String>,
        role: Option<String>,
        parent_agent_id: Option<String>,
        system_prompt: String,
        planning_prompt: Option<String>,
        available_tools: Option<String>,
        auto_approve: Option<String>,
        models: Option<AgentModels>,
        shell_policy: Option<String>,
        allowed_paths: Option<String>,
        final_audit: Option<bool>,
        approval_level: Option<String>,
        skill_enabled: Option<bool>,
        selected_skills: Option<String>,
        phase: Option<String>,
        is_system: Option<bool>,
        disabled: Option<bool>,
        max_contexts: Option<i32>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            role,
            parent_agent_id,
            system_prompt,
            planning_prompt,
            available_tools,
            auto_approve,
            models,
            shell_policy,
            allowed_paths,
            final_audit,
            approval_level,
            skill_enabled,
            selected_skills,
            phase,
            is_system,
            disabled,
            version: None,
            sort_index: None,
            max_contexts,
            created_at: None,
            updated_at: None,
        }
    }

    /// Merges values from a JSON config string into this Agent instance
    /// The config JSON uses camelCase field names (from AgentConfig serialization)
    pub fn merge_config(&mut self, config_json: &str) {
        if let Some(config) = AgentConfig::from_json(config_json) {
            // Merge models
            if let Some(config_models) = config.models {
                let mut merged_models = self.models.clone().unwrap_or_default();
                if config_models.plan.is_some() {
                    merged_models.plan = config_models.plan;
                }
                if config_models.act.is_some() {
                    merged_models.act = config_models.act;
                }
                if config_models.utility.is_some() {
                    merged_models.utility = config_models.utility;
                }
                self.models = Some(merged_models);
            }

            // Merge shell_policy (Vec<ShellPolicyRule> -> JSON string)
            if let Some(policy) = config.shell_policy {
                self.shell_policy = serde_json::to_string(&policy).ok();
            }

            // Merge allowed_paths (Vec<String> -> JSON string)
            if let Some(paths) = config.allowed_paths {
                self.allowed_paths = serde_json::to_string(&paths).ok();
            }

            // Merge final_audit
            if config.final_audit.is_some() {
                self.final_audit = config.final_audit;
            }

            // Merge auto_approve (Vec<String> -> JSON string)
            if let Some(tools) = config.auto_approve {
                self.auto_approve = serde_json::to_string(&tools).ok();
            }

            // Merge approval_level
            if config.approval_level.is_some() {
                self.approval_level = config.approval_level;
            }

            // Merge skill_enabled
            if config.skill_enabled.is_some() {
                self.skill_enabled = config.skill_enabled;
            }

            // Merge selected_skills (Vec<String> -> JSON string)
            if let Some(skills) = config.selected_skills {
                self.selected_skills = serde_json::to_string(&skills).ok();
            }

            // Merge phase
            if config.phase.is_some() {
                self.phase = config.phase;
            }

            // Merge available_tools (Vec<String> -> JSON string)
            if let Some(tools) = config.available_tools {
                self.available_tools = serde_json::to_string(&tools).ok();
            }

            // Merge max_contexts
            if config.max_contexts.is_some() {
                self.max_contexts = config.max_contexts;
            }
        }
    }
}

impl From<&Row<'_>> for Agent {
    /// Converts a database row to an Agent instance
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").unwrap_or_default(),
            name: row.get("name").unwrap_or_default(),
            description: row.get("description").ok(),
            role: row
                .get::<_, String>("role")
                .ok()
                .or_else(|| row.get::<_, String>("agent_type").ok())
                .or_else(|| Some("primary".to_string())),
            parent_agent_id: row.get("parent_agent_id").ok(),
            system_prompt: row.get("system_prompt").unwrap_or_default(),
            planning_prompt: row.get("planning_prompt").ok(),
            available_tools: row.get("available_tools").ok(),
            auto_approve: row.get("auto_approve").ok(),
            models: row
                .get::<_, String>("models")
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok()),
            shell_policy: row.get("shell_policy").ok(),
            allowed_paths: row.get("allowed_paths").ok(),
            final_audit: row.get("final_audit").ok(),
            approval_level: row.get("approval_level").ok(),
            skill_enabled: row.get("skill_enabled").ok(),
            selected_skills: row.get("selected_skills").ok(),
            phase: row.get("phase").ok(),
            is_system: row.get("is_system").ok(),
            disabled: row.get("disabled").ok(),
            version: row.get("version").ok(),
            sort_index: row.get("sort_index").ok(),
            max_contexts: row.get("max_contexts").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
        }
    }
}

impl MainStore {
    /// Adds a new agent to the database
    pub fn add_agent(&self, agent: &Agent) -> Result<String, StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let tx = conn.transaction()?;

        // Check for name uniqueness
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agents WHERE name = ?1",
            params![&agent.name],
            |row| row.get(0),
        )?;

        if count > 0 {
            return Err(StoreError::Query(
                t!("db.agent_name_exists", name = &agent.name).to_string(),
            ));
        }

        let new_sort_index: i32 = tx.query_row(
            "SELECT COALESCE(MAX(sort_index), -1) FROM agents",
            [],
            |row| row.get(0),
        )?;

        // Serialize JSON fields
        let models_json = agent
            .models
            .as_ref()
            .map(|m| serde_json::to_string(m).ok())
            .flatten();
        let available_tools_json = agent.available_tools.as_ref().cloned();
        let auto_approve_json = agent.auto_approve.as_ref().cloned();
        let shell_policy_json = agent.shell_policy.as_ref().cloned();
        let allowed_paths_json = agent.allowed_paths.as_ref().cloned();
        let selected_skills_json = agent.selected_skills.as_ref().cloned();

        // Insert the agent
        tx.execute(
            "INSERT INTO agents (id, name, description, role, parent_agent_id, system_prompt, planning_prompt, available_tools, auto_approve, models, shell_policy, allowed_paths, final_audit, approval_level, skill_enabled, selected_skills, phase, is_system, disabled, version, sort_index, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                agent.id,
                agent.name,
                agent.description,
                agent.role.clone().unwrap_or_else(|| "primary".to_string()),
                agent.parent_agent_id,
                agent.system_prompt,
                agent.planning_prompt,
                available_tools_json,
                auto_approve_json,
                models_json,
                shell_policy_json,
                allowed_paths_json,
                agent.final_audit,
                agent.approval_level,
                agent.skill_enabled,
                selected_skills_json,
                agent.phase,
                agent.is_system,
                agent.disabled,
                agent.version,
                agent.sort_index.unwrap_or(new_sort_index + 1),
                agent.max_contexts,
            ],
        )?;
        tx.commit()?;

        Ok(agent.id.clone())
    }

    /// Updates an existing agent in the database
    pub fn update_agent(&self, agent: &Agent) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let tx = conn.transaction()?;

        let existing_agent: Option<Agent> = {
            let mut stmt = tx.prepare("SELECT * FROM agents WHERE id = ?1")?;
            stmt.query_row(params![&agent.id], |row| Ok(Agent::from(row)))
                .optional()?
        };
        let persisted_name = existing_agent.as_ref().and_then(|current| {
            current
                .is_system
                .filter(|v| *v)
                .map(|_| current.name.clone())
        });
        let persisted_sort_index = existing_agent
            .as_ref()
            .and_then(|current| current.sort_index);
        let effective_name = persisted_name.unwrap_or_else(|| agent.name.clone());

        // Check for name uniqueness on other agents
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agents WHERE name = ?1 AND id != ?2",
            params![&effective_name, &agent.id],
            |row| row.get(0),
        )?;

        if count > 0 {
            return Err(StoreError::Query(
                t!("db.agent_name_exists", name = &agent.name).to_string(),
            ));
        }

        // Serialize JSON fields
        let models_json = agent
            .models
            .as_ref()
            .map(|m| serde_json::to_string(m).ok())
            .flatten();
        let available_tools_json = agent.available_tools.as_ref().cloned();
        let auto_approve_json = agent.auto_approve.as_ref().cloned();
        let shell_policy_json = agent.shell_policy.as_ref().cloned();
        let allowed_paths_json = agent.allowed_paths.as_ref().cloned();
        let selected_skills_json = agent.selected_skills.as_ref().cloned();

        // Update the agent
        tx.execute(
            "UPDATE agents SET
                name = ?1,
                description = ?2,
                role = ?3,
                parent_agent_id = ?4,
                system_prompt = ?5,
                planning_prompt = ?6,
                available_tools = ?7,
                auto_approve = ?8,
                models = ?9,
                shell_policy = ?10,
                allowed_paths = ?11,
                final_audit = ?12,
                approval_level = ?13,
                skill_enabled = ?14,
                selected_skills = ?15,
                phase = ?16,
                is_system = ?17,
                disabled = ?18,
                version = ?19,
                sort_index = ?20,
                max_contexts = ?21,
                updated_at = CURRENT_TIMESTAMP
             WHERE id = ?22",
            params![
                effective_name,
                agent.description,
                agent.role.clone().unwrap_or_else(|| "primary".to_string()),
                agent.parent_agent_id,
                agent.system_prompt,
                agent.planning_prompt,
                available_tools_json,
                auto_approve_json,
                models_json,
                shell_policy_json,
                allowed_paths_json,
                agent.final_audit,
                agent.approval_level,
                agent.skill_enabled,
                selected_skills_json,
                agent.phase,
                agent.is_system,
                agent.disabled,
                agent.version,
                agent.sort_index.or(persisted_sort_index),
                agent.max_contexts,
                agent.id,
            ],
        )?;

        tx.commit()?;

        Ok(())
    }

    /// Deletes an agent from the database
    pub fn delete_agent(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let tx = conn.transaction()?;

        // Delete the agent
        tx.execute(
            "DELETE FROM agents WHERE id = ?1 OR parent_agent_id = ?1",
            params![id],
        )?;

        tx.commit()?;

        Ok(())
    }

    /// Gets an agent by ID
    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare("SELECT * FROM agents WHERE id = ?1")?;

        let agent = stmt
            .query_row(params![id], |row| Ok(Agent::from(row)))
            .optional()?;

        Ok(agent)
    }

    /// Gets all agents
    pub fn get_all_agents(&self) -> Result<Vec<Agent>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT * FROM agents
             ORDER BY sort_index ASC, name",
        )?;

        let agents = stmt
            .query_map(params![], |row| Ok(Agent::from(row)))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(agents)
    }

    /// Gets child agents owned by the specified primary agent.
    pub fn get_child_agents(&self, parent_agent_id: &str) -> Result<Vec<Agent>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT * FROM agents
             WHERE COALESCE(role, agent_type, 'primary') = 'child'
               AND parent_agent_id = ?1
               AND COALESCE(disabled, 0) = 0
             ORDER BY sort_index ASC, name",
        )?;

        let agents = stmt
            .query_map(params![parent_agent_id], |row| Ok(Agent::from(row)))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(agents)
    }

    /// Updates the persisted ordering for agents.
    pub fn update_agent_order(&self, agent_ids: Vec<String>) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        for (index, id) in agent_ids.iter().enumerate() {
            conn.execute(
                "UPDATE agents SET sort_index = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![index as i64, id],
            )?;
        }
        Ok(())
    }
}
