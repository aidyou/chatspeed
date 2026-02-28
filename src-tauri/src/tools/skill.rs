use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::skills::SkillManifest;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

/// The Skill tool allows the agent to invoke specialized capabilities (skills).
/// It retrieves the instructions and context for a specific skill.
pub struct SkillExecute {
    pub available_skills: HashMap<String, SkillManifest>,
}

impl SkillExecute {
    pub fn new(skills: HashMap<String, SkillManifest>) -> Self {
        Self {
            available_skills: skills,
        }
    }
}

#[async_trait]
impl ToolDefinition for SkillExecute {
    fn name(&self) -> &str {
        crate::tools::TOOL_SKILL
    }

    fn description(&self) -> &str {
        r#"Activate a specialized skill within the current conversation.

Skills provide domain-specific knowledge and detailed operational guidelines.
When a user request matches an available skill (e.g. via a slash command like /commit), you MUST activate the relevant skill BEFORE proceeding.

Activation will inject the skill's specific instructions into your context."#
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "The name of the skill to activate (e.g., 'pdf', 'commit')." }
                },
                "required": ["skill"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let skill_name = params["skill"]
            .as_str()
            .ok_or(ToolError::InvalidParams("skill is required".to_string()))?;

        if let Some(skill) = self.available_skills.get(skill_name) {
            log::info!("Skill tool: Activating skill '{}'", skill_name);

            let result_content = format!(
                "<activated_skill name=\"{}\">\n<instructions>\n{}\n</instructions>\n</activated_skill>",
                skill.name,
                skill.instructions
            );

            Ok(ToolCallResult::success(Some(result_content), None))
        } else {
            Err(ToolError::ExecutionFailed(format!(
                "Skill '{}' not found. Available skills: {:?}",
                skill_name,
                self.available_skills.keys().collect::<Vec<_>>()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::skills::SkillManifest;
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_skills() -> HashMap<String, SkillManifest> {
        let mut skills = HashMap::new();

        skills.insert(
            "pdf".to_string(),
            SkillManifest {
                name: "pdf".to_string(),
                instructions: "Process PDF files".to_string(),
                version: "1.0".to_string(),
                description: "PDF processing skill".to_string(),
                tools: vec![],
            },
        );

        skills.insert(
            "commit".to_string(),
            SkillManifest {
                name: "commit".to_string(),
                instructions: "Create git commits".to_string(),
                version: "1.0".to_string(),
                description: "Git commit skill".to_string(),
                tools: vec![],
            },
        );

        skills
    }

    #[tokio::test]
    async fn test_skill_execute_found() {
        let skills = create_test_skills();
        let tool = SkillExecute::new(skills);

        let params = json!({
            "skill": "pdf"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        dbg!(&output);
        assert!(output.contains("<activated_skill"));
        assert!(output.contains("name=\"pdf\""));
        assert!(output.contains("Process PDF files"));
    }

    #[tokio::test]
    async fn test_skill_execute_not_found() {
        let skills = create_test_skills();
        let tool = SkillExecute::new(skills);

        let params = json!({
            "skill": "nonexistent"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => {
                assert!(msg.contains("Skill 'nonexistent' not found"));
                assert!(msg.contains("pdf"));
                assert!(msg.contains("commit"));
            }
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_skill_execute_missing_skill_param() {
        let skills = create_test_skills();
        let tool = SkillExecute::new(skills);

        let params = json!({});

        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn test_skill_execute_empty_skill_name() {
        let skills = create_test_skills();
        let tool = SkillExecute::new(skills);

        let params = json!({
            "skill": ""
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => {
                assert!(msg.contains("Skill '' not found"));
            }
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_skill_execute_special_characters() {
        let skills = create_test_skills();
        let tool = SkillExecute::new(skills);

        let params = json!({
            "skill": "pdf-2.0"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_skill_execute_case_sensitive() {
        let mut skills = HashMap::new();
        skills.insert(
            "PDF".to_string(),
            SkillManifest {
                name: "PDF".to_string(),
                instructions: "Uppercase PDF skill".to_string(),
                version: "1.0".to_string(),
                description: "Uppercase PDF skill".to_string(),
                tools: vec![],
            },
        );

        let tool = SkillExecute::new(skills);

        // Lowercase should not match uppercase
        let params = json!({
            "skill": "pdf"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());

        // Uppercase should match
        let params = json!({
            "skill": "PDF"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("Uppercase PDF skill"));
    }

    #[tokio::test]
    async fn test_skill_execute_output_format() {
        let mut skills = HashMap::new();
        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test\nMulti\nLine\nInstructions".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
            },
        );

        let tool = SkillExecute::new(skills);

        let params = json!({
            "skill": "test"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();

        // Check XML-like format
        assert!(output.starts_with("<activated_skill"));
        assert!(output.contains("</activated_skill>"));
        assert!(output.contains("<instructions>"));
        assert!(output.contains("</instructions>"));
        assert!(output.contains("Test\nMulti\nLine\nInstructions"));
    }
}
