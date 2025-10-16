//! Agent database operations
//!
//! This module provides database operations for managing ReAct agents.

use crate::db::{MainStore, StoreError};
use rusqlite::{params, OptionalExtension, Row};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

/// Represents an AI agent for ReAct workflows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier for the agent
    pub id: String,
    /// Name of the agent
    pub name: String,
    /// Description of the agent
    pub description: Option<String>,
    /// System prompt for the agent
    pub system_prompt: String,
    /// Type of the agent, e.g., 'autonomous' or 'planning'
    pub agent_type: String,
    /// Prompt for the planning phase
    pub planning_prompt: Option<String>,
    /// JSON array of available tool IDs
    pub available_tools: Option<String>,
    /// JSON array of tools that can be executed without user confirmation
    pub auto_approve: Option<String>,
    /// Model for planning phase
    pub plan_model: Option<String>,
    /// Model for action phase
    pub act_model: Option<String>,
    /// Vision model (reserved)
    pub vision_model: Option<String>,
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
        system_prompt: String,
        agent_type: String,
        planning_prompt: Option<String>,
        available_tools: Option<String>,
        auto_approve: Option<String>,
        plan_model: Option<String>,
        act_model: Option<String>,
        vision_model: Option<String>,
        max_contexts: Option<i32>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            system_prompt,
            agent_type,
            planning_prompt,
            available_tools,
            auto_approve,
            plan_model,
            act_model,
            vision_model,
            max_contexts,
            created_at: None,
            updated_at: None,
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
            system_prompt: row.get("system_prompt").unwrap_or_default(),
            agent_type: row
                .get("agent_type")
                .unwrap_or_else(|_| "autonomous".to_string()),
            planning_prompt: row.get("planning_prompt").ok(),
            available_tools: row.get("available_tools").ok(),
            auto_approve: row.get("auto_approve").ok(),
            plan_model: row.get("plan_model").ok(),
            act_model: row.get("act_model").ok(),
            vision_model: row.get("vision_model").ok(),
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

        // Insert the agent
        tx.execute(
            "INSERT INTO agents (id, name, description, system_prompt, agent_type, planning_prompt, available_tools, auto_approve, plan_model, act_model, vision_model, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                agent.id,
                agent.name,
                agent.description,
                agent.system_prompt,
                agent.agent_type,
                agent.planning_prompt,
                agent.available_tools,
                agent.auto_approve,
                agent.plan_model,
                agent.act_model,
                agent.vision_model,
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

        // Check for name uniqueness on other agents
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agents WHERE name = ?1 AND id != ?2",
            params![&agent.name, &agent.id],
            |row| row.get(0),
        )?;

        if count > 0 {
            return Err(StoreError::Query(
                t!("db.agent_name_exists", name = &agent.name).to_string(),
            ));
        }

        // Update the agent
        tx.execute(
            "UPDATE agents SET name = ?1, description = ?2, system_prompt = ?3, agent_type = ?4, planning_prompt = ?5, available_tools = ?6, auto_approve = ?7, plan_model = ?8, act_model = ?9, vision_model = ?10, max_contexts = ?11, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?12",
            params![
                agent.name,
                agent.description,
                agent.system_prompt,
                agent.agent_type,
                agent.planning_prompt,
                agent.available_tools,
                agent.auto_approve,
                agent.plan_model,
                agent.act_model,
                agent.vision_model,
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
        tx.execute("DELETE FROM agents WHERE id = ?1", params![id])?;

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

        let mut stmt = conn.prepare("SELECT * FROM agents ORDER BY name")?;

        let agents = stmt
            .query_map(params![], |row| Ok(Agent::from(row)))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(agents)
    }
}
