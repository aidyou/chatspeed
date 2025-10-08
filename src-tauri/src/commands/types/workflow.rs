use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ai::traits::chat::MCPToolDeclaration, db::Agent};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentPayload {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub agent_type: String,
    pub planning_prompt: Option<String>,
    pub available_tools: Option<String>,
    pub auto_approve: Option<String>,
    pub plan_model: Option<String>,
    pub act_model: Option<String>,
    pub vision_model: Option<String>,
    pub max_contexts: Option<i32>,
}

impl From<AgentPayload> for Agent {
    fn from(payload: AgentPayload) -> Self {
        Agent::new(
            payload.id.unwrap_or_default(),
            payload.name,
            payload.description,
            payload.system_prompt,
            payload.agent_type,
            payload.planning_prompt,
            payload.available_tools,
            payload.auto_approve,
            payload.plan_model,
            payload.act_model,
            payload.vision_model,
            payload.max_contexts,
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowChatPayload {
    pub model_id: String,
    pub provider_id: i64,
    pub messages: Vec<Value>,
    pub temperature: f32,
    pub available_tools: Vec<String>,
    pub ts_tools: Vec<MCPToolDeclaration>,
}
