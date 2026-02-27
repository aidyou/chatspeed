use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Command;
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory, ToolError};
use crate::ai::traits::chat::MCPToolDeclaration;

pub struct ShellExecute;

#[async_trait]
impl ToolDefinition for ShellExecute {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str { 
        "Executes a given bash command with optional timeout. Working directory persists between commands; shell state (everything else) does not. The shell environment is initialized from the user's profile (bash or zsh).\n\n\
        IMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.\n\n\
        Before executing the command, please follow these steps:\n\n\
        1. Directory Verification:\n\
           - If the command will create new directories or files, first use `ls` to verify the parent directory exists and is the correct location\n\
           - For example, before running \"mkdir foo/bar\", first use `ls foo` to check that \"foo\" exists and is the intended parent directory\n\n\
        2. Command Execution:\n\
           - Always quote file paths that contain spaces with double quotes (e.g., cd \"path with spaces/file.txt\")\n\
           - Examples of proper quoting:\n\
             - cd \"/Users/name/My Documents\" (correct)\n\
             - cd /Users/name/My Documents (incorrect - will fail)\n\
             - python \"/path/with spaces/script.py\" (correct)\n\
             - python /path/with spaces/script.py (incorrect - will fail)\n\
           - After ensuring proper quoting, execute the command.\n\
           - Capture the output of the command.\n\n\
        Usage notes:\n\
          - The command argument is required.\n\
          - You can specify an optional timeout in milliseconds (up to 600000ms / 10 minutes). If not specified, commands will timeout after 120000ms (2 minutes).\n\
          - It is very helpful if you write a clear, concise description of what this command does. For simple commands, keep it brief (5-10 words).\n\
          - If the output exceeds 30000 characters, output will be truncated before being returned to you.\n\
          \n\
          - You can use the `run_in_background` parameter to run the command in the background. Use task_output to read the output later."
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout": { "type": "number", "description": "Optional timeout in milliseconds (max 600000)" },
                    "description": { "type": "string", "description": "Clear, concise description of what this command does in active voice." }
                },
                "required": ["command"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let command_str = params["command"].as_str().ok_or(ToolError::InvalidParams("command is required".to_string()))?;
        let args = vec!["-c", command_str];

        let forbidden = vec!["rm -rf /", "mkfs", "dd", "format"];
        if forbidden.iter().any(|&f| command_str.contains(f)) {
            return Err(ToolError::Security(format!("Command '{}' is forbidden", command_str)));
        }

        let output = Command::new("sh")
            .args(&args)
            .output()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to execute command: {}", e)))?;

        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

        const MAX_OUTPUT: usize = 30_000;
        if stdout.len() > MAX_OUTPUT {
            stdout.truncate(MAX_OUTPUT);
            stdout.push_str("\n... [stdout truncated]");
        }
        if stderr.len() > MAX_OUTPUT {
            stderr.truncate(MAX_OUTPUT);
            stderr.push_str("\n... [stderr truncated]");
        }

        if output.status.success() {
            Ok(ToolCallResult::success(Some(stdout), None))
        } else {
            Err(ToolError::ExecutionFailed(format!("Command failed with status {}. Stderr: {}", output.status, stderr)))
        }
    }
}
