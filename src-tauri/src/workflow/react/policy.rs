use crate::tools::ToolCategory;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PathRestriction {
    /// Restricted to temporary and draft directories (skills, tmp, planning)
    SandboxOnly,
    /// Full access to all user-authorized directories
    FullAuthorized,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionPhase {
    /// Research and design phase
    Planning,
    /// Performing actions after a plan is approved
    Implementation,
    /// Direct ReAct without formal planning phase
    Standard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApprovalLevel {
    /// Follow agent config's auto_approve list
    Default,
    /// Only allow read-only operations, block all mutations unless approved
    Smart,
    /// Allow all tool calls without confirmation (High Risk)
    Full,
}

/// ExecutionPolicy defines the rules and permissions for a ReAct session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub name: String,
    /// Categories of tools permitted in this mode
    pub allowed_categories: Vec<ToolCategory>,
    /// Level of file system access
    pub path_restriction: PathRestriction,
    /// The current operational phase
    pub phase: ExecutionPhase,
    /// Level of automatic approval for tool calls
    pub approval_level: ApprovalLevel,
}

impl ExecutionPolicy {
    /// Standard policy for the Planning phase
    pub fn planning() -> Self {
        Self {
            name: "Planning".to_string(),
            allowed_categories: vec![
                ToolCategory::Web,
                ToolCategory::Interaction,
                ToolCategory::Skill,
                ToolCategory::FileSystem,
            ],
            path_restriction: PathRestriction::SandboxOnly,
            phase: ExecutionPhase::Planning,
            approval_level: ApprovalLevel::Default,
        }
    }

    /// Policy for the Execution phase (following a plan)
    pub fn implementation() -> Self {
        Self {
            name: "Implementation".to_string(),
            allowed_categories: vec![
                ToolCategory::FileSystem,
                ToolCategory::System,
                ToolCategory::Web,
                ToolCategory::Interaction,
                ToolCategory::Skill,
                ToolCategory::Mcp,
            ],
            path_restriction: PathRestriction::FullAuthorized,
            phase: ExecutionPhase::Implementation,
            approval_level: ApprovalLevel::Default,
        }
    }

    /// Standard policy for direct execution
    pub fn standard() -> Self {
        Self {
            name: "Standard".to_string(),
            allowed_categories: vec![
                ToolCategory::FileSystem,
                ToolCategory::System,
                ToolCategory::Web,
                ToolCategory::Interaction,
                ToolCategory::Skill,
                ToolCategory::Mcp,
            ],
            path_restriction: PathRestriction::FullAuthorized,
            phase: ExecutionPhase::Standard,
            approval_level: ApprovalLevel::Default,
        }
    }
}
