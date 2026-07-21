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
    pub final_review_mode: Option<String>,
    pub skill_enabled: Option<bool>,
    pub selected_skills: Option<Vec<String>>,
    pub mcp_tool_exposure: Option<Vec<String>>,
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

    pub fn normalized_final_review_mode(&self) -> &'static str {
        match self.final_review_mode.as_deref() {
            Some("sub_agent_review") => "sub_agent_review",
            Some("off") => "off",
            Some(_) => "off",
            None => {
                if self.final_audit.unwrap_or(false) {
                    "sub_agent_review"
                } else {
                    "off"
                }
            }
        }
    }

    pub fn sync_legacy_final_audit_flag(&mut self) {
        let mode = self.normalized_final_review_mode().to_string();
        self.final_audit = Some(mode == "sub_agent_review");
        self.final_review_mode = Some(mode);
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
        if other.final_review_mode.is_some() {
            self.final_review_mode = other.final_review_mode.clone();
        }
        if other.skill_enabled.is_some() {
            self.skill_enabled = other.skill_enabled;
        }
        if other.selected_skills.is_some() {
            self.selected_skills = other.selected_skills.clone();
        }
        if other.mcp_tool_exposure.is_some() {
            self.mcp_tool_exposure = other.mcp_tool_exposure.clone();
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
        self.sync_legacy_final_audit_flag();
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
    pub function_call: Option<bool>,
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
    pub vision: Option<ModelConfig>,
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
    /// Prompt for image recognition preprocessing
    pub image_recognition_prompt: Option<String>,
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
    /// JSON array of MCP tool names exposed with full parameter schemas in workflows
    pub mcp_tool_exposure: Option<String>,
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
        image_recognition_prompt: Option<String>,
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
            image_recognition_prompt,
            available_tools,
            auto_approve,
            models,
            shell_policy,
            allowed_paths,
            final_audit,
            approval_level,
            skill_enabled,
            selected_skills,
            mcp_tool_exposure: None,
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
            let final_review_enabled = config.normalized_final_review_mode() == "sub_agent_review";
            // Merge models
            if let Some(config_models) = config.models.clone() {
                let mut merged_models = self.models.clone().unwrap_or_default();
                if config_models.plan.is_some() {
                    merged_models.plan = config_models.plan;
                }
                if config_models.act.is_some() {
                    merged_models.act = config_models.act;
                }
                if config_models.vision.is_some() {
                    merged_models.vision = config_models.vision;
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
            if config.final_review_mode.is_some() {
                self.final_audit = Some(final_review_enabled);
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

            // Merge mcp_tool_exposure (Vec<String> -> JSON string)
            if let Some(tools) = config.mcp_tool_exposure {
                self.mcp_tool_exposure = serde_json::to_string(&tools).ok();
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
            image_recognition_prompt: row.get("image_recognition_prompt").ok(),
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
            mcp_tool_exposure: row.get("mcp_tool_exposure").ok(),
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
        let mcp_tool_exposure_json = agent.mcp_tool_exposure.as_ref().cloned();

        // Insert the agent
        tx.execute(
            "INSERT INTO agents (id, name, description, role, parent_agent_id, system_prompt, planning_prompt, image_recognition_prompt, available_tools, auto_approve, models, shell_policy, allowed_paths, final_audit, approval_level, skill_enabled, selected_skills, mcp_tool_exposure, phase, is_system, disabled, version, sort_index, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                agent.id,
                agent.name,
                agent.description,
                agent.role.clone().unwrap_or_else(|| "primary".to_string()),
                agent.parent_agent_id,
                agent.system_prompt,
                agent.planning_prompt,
                agent.image_recognition_prompt,
                available_tools_json,
                auto_approve_json,
                models_json,
                shell_policy_json,
                allowed_paths_json,
                agent.final_audit,
                agent.approval_level,
                agent.skill_enabled,
                selected_skills_json,
                mcp_tool_exposure_json,
                agent.phase,
                agent.is_system,
                agent.disabled,
                agent.version.unwrap_or(0),
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
        let mcp_tool_exposure_json = agent.mcp_tool_exposure.as_ref().cloned();

        // Update the agent
        tx.execute(
            "UPDATE agents SET
                name = ?1,
                description = ?2,
                role = ?3,
                parent_agent_id = ?4,
                system_prompt = ?5,
                planning_prompt = ?6,
                image_recognition_prompt = ?7,
                available_tools = ?8,
                auto_approve = ?9,
                models = ?10,
                shell_policy = ?11,
                allowed_paths = ?12,
                final_audit = ?13,
                approval_level = ?14,
                skill_enabled = ?15,
                selected_skills = ?16,
                mcp_tool_exposure = ?17,
                phase = ?18,
                is_system = ?19,
                disabled = ?20,
                version = ?21,
                sort_index = ?22,
                max_contexts = ?23,
                updated_at = CURRENT_TIMESTAMP
             WHERE id = ?24",
            params![
                effective_name,
                agent.description,
                agent.role.clone().unwrap_or_else(|| "primary".to_string()),
                agent.parent_agent_id,
                agent.system_prompt,
                agent.planning_prompt,
                agent.image_recognition_prompt,
                available_tools_json,
                auto_approve_json,
                models_json,
                shell_policy_json,
                allowed_paths_json,
                agent.final_audit,
                agent.approval_level,
                agent.skill_enabled,
                selected_skills_json,
                mcp_tool_exposure_json,
                agent.phase,
                agent.is_system,
                agent.disabled,
                agent.version.unwrap_or(0),
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

        let agent_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE agent_tree(id, depth) AS (
                    SELECT id, 0 FROM agents WHERE id = ?1
                    UNION ALL
                    SELECT agents.id, agent_tree.depth + 1
                    FROM agents
                    JOIN agent_tree ON agents.parent_agent_id = agent_tree.id
                )
                SELECT id FROM agent_tree ORDER BY depth DESC",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        if agent_ids.is_empty() {
            tx.commit()?;
            return Ok(());
        }

        let workflow_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE agent_tree(id) AS (
                    SELECT id FROM agents WHERE id = ?1
                    UNION ALL
                    SELECT agents.id
                    FROM agents
                    JOIN agent_tree ON agents.parent_agent_id = agent_tree.id
                ),
                workflow_tree(id, depth) AS (
                    SELECT id, 0
                    FROM workflows
                    WHERE agent_id IN (SELECT id FROM agent_tree)
                    UNION ALL
                    SELECT workflows.id, workflow_tree.depth + 1
                    FROM workflows
                    JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
                )
                SELECT id
                FROM workflow_tree
                GROUP BY id
                ORDER BY MAX(depth) DESC",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        for workflow_id in &workflow_ids {
            tx.execute(
                "DELETE FROM workflow_context_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;
            tx.execute(
                "DELETE FROM workflow_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;
            tx.execute(
                "DELETE FROM workflow_snapshots WHERE session_id = ?1",
                params![workflow_id],
            )?;
            if let Err(e) = tx.execute(
                "DELETE FROM workflow_events WHERE session_id = ?1",
                params![workflow_id],
            ) {
                log::error!(
                    "[Agent][id={}] Failed to delete workflow events for session {} (non-fatal, continuing): {}",
                    id,
                    workflow_id,
                    e
                );
            }
        }

        for workflow_id in &workflow_ids {
            tx.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;
        }

        for agent_id in &agent_ids {
            tx.execute("DELETE FROM agents WHERE id = ?1", params![agent_id])?;
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_store() -> MainStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("agent_test.db");
        MainStore::new(db_path).expect("failed to create MainStore")
    }

    fn make_agent(id: &str, name: &str, parent_agent_id: Option<&str>) -> Agent {
        Agent {
            id: id.to_string(),
            name: name.to_string(),
            description: Some(format!("Description for {}", name)),
            role: Some(if parent_agent_id.is_some() {
                "child".to_string()
            } else {
                "primary".to_string()
            }),
            parent_agent_id: parent_agent_id.map(ToString::to_string),
            system_prompt: format!("System prompt for {}", name),
            planning_prompt: None,
            image_recognition_prompt: None,
            available_tools: Some("[]".to_string()),
            auto_approve: Some("[]".to_string()),
            models: None,
            shell_policy: Some("[]".to_string()),
            allowed_paths: Some("[]".to_string()),
            final_audit: Some(false),
            approval_level: Some("default".to_string()),
            skill_enabled: Some(true),
            selected_skills: None,
            mcp_tool_exposure: None,
            phase: Some("standard".to_string()),
            is_system: Some(false),
            disabled: Some(false),
            version: None,
            sort_index: None,
            max_contexts: Some(128000),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_add_agent_defaults_version_to_zero() {
        let store = create_test_store();
        let agent = make_agent("agent-primary", "Primary Agent", None);

        store
            .add_agent(&agent)
            .expect("failed to add custom agent without explicit version");

        let stored = store
            .get_agent("agent-primary")
            .expect("failed to load saved agent")
            .expect("saved agent should exist");
        assert_eq!(
            stored.version,
            Some(0),
            "custom agents should persist with version 0 when UI omits it"
        );
    }

    #[test]
    fn test_mcp_tool_exposure_persists_with_agent() {
        let store = create_test_store();
        let mut agent = make_agent("agent-mcp", "MCP Agent", None);
        agent.mcp_tool_exposure =
            Some(serde_json::json!(["server__MCP__important_tool"]).to_string());

        store.add_agent(&agent).expect("failed to add agent");
        let stored = store
            .get_agent("agent-mcp")
            .expect("failed to load agent")
            .expect("agent should exist");

        assert_eq!(
            stored.mcp_tool_exposure,
            Some(serde_json::json!(["server__MCP__important_tool"]).to_string())
        );
    }

    #[test]
    fn test_delete_agent_removes_descendant_workflows_before_agents() {
        let store = create_test_store();

        store
            .add_agent(&make_agent("agent-primary", "Primary Agent", None))
            .expect("failed to add primary agent");
        store
            .add_agent(&make_agent(
                "agent-child",
                "Child Agent",
                Some("agent-primary"),
            ))
            .expect("failed to add child agent");

        store
            .create_workflow(
                "workflow-parent",
                "Parent query",
                "agent-primary",
                None,
                None,
            )
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "workflow-child",
                "Child query",
                "agent-child",
                None,
                Some("workflow-parent"),
            )
            .expect("failed to create child workflow");

        store
            .delete_agent("agent-primary")
            .expect("failed to delete agent tree");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for delete assertion");
        let remaining_agents: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM agents WHERE id IN ('agent-primary', 'agent-child')",
                [],
                |row| row.get(0),
            )
            .expect("failed to count remaining agents");
        let remaining_workflows: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflows WHERE id IN ('workflow-parent', 'workflow-child')",
                [],
                |row| row.get(0),
            )
            .expect("failed to count remaining workflows");

        assert_eq!(remaining_agents, 0, "agent tree should be deleted");
        assert_eq!(
            remaining_workflows, 0,
            "workflows bound to the deleted agents should be removed first"
        );
    }
}
