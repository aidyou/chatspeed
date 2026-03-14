use serde::{Deserialize, Serialize};

use crate::db::Agent;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentPayload {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub planning_prompt: Option<String>,
    pub available_tools: Option<String>,
    pub auto_approve: Option<String>,
    pub models: Option<crate::db::agent::AgentModels>,
    pub shell_policy: Option<String>,
    pub allowed_paths: Option<String>,
    pub final_audit: Option<bool>,
    pub approval_level: Option<String>,
    pub max_contexts: Option<i32>,
}

impl From<AgentPayload> for Agent {
    fn from(payload: AgentPayload) -> Self {
        Agent::new(
            payload.id.unwrap_or_default(),
            payload.name,
            payload.description,
            payload.system_prompt,
            payload.planning_prompt,
            payload.available_tools,
            payload.auto_approve,
            payload.models,
            payload.shell_policy,
            payload.allowed_paths,
            payload.final_audit,
            payload.approval_level,
            payload.max_contexts,
        )
    }
}
