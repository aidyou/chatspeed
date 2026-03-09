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
            scope: Some(self.scope()),
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

/// List available reference files for a skill
pub struct SkillListReferences {
    pub available_skills: HashMap<String, SkillManifest>,
}

impl SkillListReferences {
    pub fn new(skills: HashMap<String, SkillManifest>) -> Self {
        Self {
            available_skills: skills,
        }
    }
}

#[async_trait]
impl ToolDefinition for SkillListReferences {
    fn name(&self) -> &str {
        crate::tools::TOOL_SKILL_LIST_REFERENCES
    }

    fn description(&self) -> &str {
        r#"List available reference files for a skill.

Returns a list of reference files that can be loaded on-demand.
Each reference contains specialized patterns, examples, or detailed guidance.

Use this after activating a skill to discover available references."#
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
                    "skill": {
                        "type": "string",
                        "description": "The name of the skill"
                    }
                },
                "required": ["skill"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let skill_name = params["skill"]
            .as_str()
            .ok_or(ToolError::InvalidParams("skill is required".to_string()))?;

        let skill = self.available_skills.get(skill_name)
            .ok_or(ToolError::ExecutionFailed(format!(
                "Skill '{}' not found. Available skills: {:?}",
                skill_name,
                self.available_skills.keys().collect::<Vec<_>>()
            )))?;

        if skill.references.is_empty() {
            return Ok(ToolCallResult::success(
                Some(format!(
                    "Skill '{}' has no reference files available.",
                    skill_name
                )),
                None
            ));
        }

        let refs_json = serde_json::to_string_pretty(&skill.references)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to serialize references: {}", e)))?;

        Ok(ToolCallResult::success(
            Some(format!(
                "<skill_references skill=\"{}\">\n{}\n</skill_references>",
                skill_name, refs_json
            )),
            None
        ))
    }
}

/// Load a specific reference file from a skill
pub struct SkillLoadReference {
    pub available_skills: HashMap<String, SkillManifest>,
}

impl SkillLoadReference {
    pub fn new(skills: HashMap<String, SkillManifest>) -> Self {
        Self {
            available_skills: skills,
        }
    }
}

#[async_trait]
impl ToolDefinition for SkillLoadReference {
    fn name(&self) -> &str {
        crate::tools::TOOL_SKILL_LOAD_REFERENCE
    }

    fn description(&self) -> &str {
        r#"Load a specific reference file from a skill.

Loads the full content of a reference file on-demand.
Use this after listing available references with skill_list_references.

Reference files contain detailed patterns, examples, and guidance
that supplement the skill's main instructions."#
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
                    "skill": {
                        "type": "string",
                        "description": "The name of the skill"
                    },
                    "reference": {
                        "type": "string",
                        "description": "The filename of the reference to load"
                    }
                },
                "required": ["skill", "reference"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let skill_name = params["skill"]
            .as_str()
            .ok_or(ToolError::InvalidParams("skill is required".to_string()))?;
        let reference_name = params["reference"]
            .as_str()
            .ok_or(ToolError::InvalidParams("reference is required".to_string()))?;

        let skill = self.available_skills.get(skill_name)
            .ok_or(ToolError::ExecutionFailed(format!(
                "Skill '{}' not found. Available skills: {:?}",
                skill_name,
                self.available_skills.keys().collect::<Vec<_>>()
            )))?;

        let skill_dir = skill.skill_dir.as_ref()
            .ok_or(ToolError::ExecutionFailed(
                "Skill directory information not available".to_string()
            ))?;

        // Security check: prevent path traversal attacks
        if reference_name.contains('/') || reference_name.contains('\\') || reference_name.contains("..") {
            return Err(ToolError::ExecutionFailed(
                "Invalid reference filename: path separators not allowed".to_string()
            ));
        }

        let reference_path = skill_dir.join("references").join(reference_name);

        // Verify file exists and is within skill directory
        if !reference_path.exists() {
            let available: Vec<String> = skill.references.iter()
                .map(|r| r.filename.clone())
                .collect();
            return Err(ToolError::ExecutionFailed(format!(
                "Reference '{}' not found. Available references: {:?}",
                reference_name, available
            )));
        }

        // Ensure file is within references directory (security check)
        let canonical_reference = reference_path.canonicalize()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to resolve reference path: {}", e)))?;
        let canonical_references_dir = skill_dir.join("references").canonicalize()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to resolve references directory: {}", e)))?;

        if !canonical_reference.starts_with(&canonical_references_dir) {
            return Err(ToolError::ExecutionFailed(
                "Security violation: reference file must be within skill's references directory".to_string()
            ));
        }

        // Read file content
        let content = std::fs::read_to_string(&reference_path)
            .map_err(|e| ToolError::ExecutionFailed(format!(
                "Failed to read reference file '{}': {}",
                reference_name, e
            )))?;

        log::info!("Loaded reference '{}' from skill '{}'", reference_name, skill_name);

        Ok(ToolCallResult::success(
            Some(format!(
                "<skill_reference skill=\"{}\" file=\"{}\">\n{}\n</skill_reference>",
                skill_name, reference_name, content
            )),
            None
        ))
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
                skill_dir: None,
                references: vec![],
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
                skill_dir: None,
                references: vec![],
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
                skill_dir: None,
                references: vec![],
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
                skill_dir: None,
                references: vec![],
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

    // Tests for SkillListReferences
    #[tokio::test]
    async fn test_skill_list_references_with_files() {
        let mut skills = HashMap::new();
        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test skill".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
                skill_dir: None,
                references: vec![
                    crate::workflow::react::skills::ReferenceInfo {
                        filename: "guide.md".to_string(),
                        description: "Guide".to_string(),
                        size: Some(100),
                    },
                    crate::workflow::react::skills::ReferenceInfo {
                        filename: "examples.md".to_string(),
                        description: "Examples".to_string(),
                        size: Some(200),
                    },
                ],
            },
        );

        let tool = SkillListReferences::new(skills);
        let params = json!({
            "skill": "test"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("<skill_references"));
        assert!(output.contains("guide.md"));
        assert!(output.contains("examples.md"));
    }

    #[tokio::test]
    async fn test_skill_list_references_empty() {
        let mut skills = HashMap::new();
        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test skill".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
                skill_dir: None,
                references: vec![],
            },
        );

        let tool = SkillListReferences::new(skills);
        let params = json!({
            "skill": "test"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("no reference files available"));
    }

    #[tokio::test]
    async fn test_skill_list_references_not_found() {
        let skills = HashMap::new();
        let tool = SkillListReferences::new(skills);
        let params = json!({
            "skill": "nonexistent"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
    }

    // Tests for SkillLoadReference
    #[tokio::test]
    async fn test_skill_load_reference_path_traversal() {
        let mut skills = HashMap::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let skill_dir = temp_dir.path().to_path_buf();

        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test skill".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
                skill_dir: Some(skill_dir),
                references: vec![],
            },
        );

        let tool = SkillLoadReference::new(skills);

        // Test path traversal with ..
        let result = tool.call(json!({
            "skill": "test",
            "reference": "../../../etc/passwd"
        })).await;
        assert!(result.is_err());

        // Test path traversal with /
        let result = tool.call(json!({
            "skill": "test",
            "reference": "subdir/file.md"
        })).await;
        assert!(result.is_err());

        // Test path traversal with \
        let result = tool.call(json!({
            "skill": "test",
            "reference": "..\\..\\etc\\passwd"
        })).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_skill_load_reference_success() {
        let mut skills = HashMap::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        // Create references directory and file
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir(&refs_dir).unwrap();
        std::fs::write(
            refs_dir.join("guide.md"),
            "# Guide\n\nThis is a detailed guide."
        ).unwrap();

        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test skill".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
                skill_dir: Some(skill_dir.to_path_buf()),
                references: vec![
                    crate::workflow::react::skills::ReferenceInfo {
                        filename: "guide.md".to_string(),
                        description: "Guide".to_string(),
                        size: Some(36),
                    },
                ],
            },
        );

        let tool = SkillLoadReference::new(skills);
        let params = json!({
            "skill": "test",
            "reference": "guide.md"
        });

        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("<skill_reference"));
        assert!(output.contains("guide.md"));
        assert!(output.contains("This is a detailed guide"));
    }

    #[tokio::test]
    async fn test_skill_load_reference_not_found() {
        let mut skills = HashMap::new();
        let temp_dir = tempfile::TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        // Create references directory but no file
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        skills.insert(
            "test".to_string(),
            SkillManifest {
                name: "test".to_string(),
                instructions: "Test skill".to_string(),
                version: "1.0".to_string(),
                description: "Test skill".to_string(),
                tools: vec![],
                skill_dir: Some(skill_dir.to_path_buf()),
                references: vec![],
            },
        );

        let tool = SkillLoadReference::new(skills);
        let params = json!({
            "skill": "test",
            "reference": "nonexistent.md"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Expected ExecutionFailed error"),
        }
    }
}
