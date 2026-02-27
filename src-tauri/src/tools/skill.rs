use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory, ToolError};
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::workflow::react::skills::SkillManifest;

/// The Skill tool allows the agent to invoke specialized capabilities (skills).
/// It retrieves the instructions and context for a specific skill.
pub struct SkillExecute {
    pub available_skills: HashMap<String, SkillManifest>,
}

impl SkillExecute {
    pub fn new(skills: HashMap<String, SkillManifest>) -> Self {
        Self { available_skills: skills }
    }
}

#[async_trait]
impl ToolDefinition for SkillExecute {
    fn name(&self) -> &str { "Skill" }

    fn description(&self) -> &str { 
        r#"Execute a skill within the main conversation

When users ask you to perform tasks, check if any of the available skills match. Skills provide specialized capabilities and domain knowledge.

When users reference a "slash command" or "/<something>" (e.g., "/commit", "/review-pr"), they are referring to a skill. Use this tool to invoke it.

How to invoke:
- Use this tool with the skill name and optional arguments
- Examples:
  - `skill: "pdf"` - invoke the pdf skill
  - `skill: "commit", args: "-m 'Fix bug'"` - invoke with arguments
  - `skill: "review-pr", args: "123"` - invoke with arguments

Important:
- Available skills are listed in system-reminder messages in the conversation
- When a skill matches the user's request, this is a BLOCKING REQUIREMENT: invoke the relevant Skill tool BEFORE generating any other response about the task
- NEVER mention a skill without actually calling this tool
- Do not invoke a skill that is already running
- Do not use this tool for built-in CLI commands (like /help, /clear, etc.)
- If you see a <command-name> tag in the current conversation turn, the skill has ALREADY been loaded - follow the instructions directly instead of calling this tool again"#
    }

    fn category(&self) -> ToolCategory { ToolCategory::System }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "The skill name. E.g., \"commit\", \"review-pr\", or \"pdf\"" },
                    "args": { "type": "string", "description": "Optional arguments for the skill" }
                },
                "required": ["skill"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let skill_name = params["skill"].as_str().ok_or(ToolError::InvalidParams("skill is required".to_string()))?;
        let _args = params["args"].as_str().unwrap_or("");

        if let Some(skill) = self.available_skills.get(skill_name) {
            log::info!("Skill tool: Loading instructions for '{}'", skill_name);
            
            let result_content = format!(
                "<activated_skill name=\"{}\">\n<instructions>\n{}\n</instructions>\n</activated_skill>",
                skill.name,
                skill.instructions
            );

            Ok(ToolCallResult::success(Some(result_content), None))
        } else {
            Err(ToolError::ExecutionFailed(format!("Skill '{}' not found. Available: {:?}", skill_name, self.available_skills.keys().collect::<Vec<_>>())))
        }
    }
}
