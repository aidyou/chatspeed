use crate::db::{WorkflowAutomation, WorkflowAutomationRun};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationShellConfig {
    pub command: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub args: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRequest {
    pub id: Option<String>,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<Value>,
    pub allowed_paths: Vec<String>,
    pub shell_config: Option<Value>,
    pub schedule_kind: String,
    pub schedule_config: Value,
    pub self_review: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRunNowResult {
    pub automation: WorkflowAutomation,
    pub run: WorkflowAutomationRun,
    pub workflow_session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DailyScheduleConfig {
    pub time: String,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntervalScheduleConfig {
    #[serde(alias = "interval_hours")]
    pub interval_minutes: u32,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub anchor_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OnceScheduleConfig {
    pub run_at: String,
}
