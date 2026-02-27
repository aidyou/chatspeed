use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory, ToolError};
use crate::ai::traits::chat::MCPToolDeclaration;

pub struct ReadFile;

#[async_trait]
impl ToolDefinition for ReadFile {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { 
        "Reads a file from the local filesystem. You can access any file directly by using this tool.\n\
        Assume this tool is able to read all files on the machine. If the user provides a path to a file assume that path is valid. \
        It is okay to read a file that does not exist; an error will be returned.\n\n\
        Usage:\n\
        - The file_path parameter must be an absolute path, not a relative path\n\
        - By default, it reads up to 2000 lines starting from the beginning of the file\n\
        - You can optionally specify a line offset and limit (especially handy for long files), but it's recommended to read the whole file by not providing these parameters\n\
        - Any lines longer than 2000 characters will be truncated\n\
        - Results are returned using cat -n format, with line numbers starting at 1\n\
        - This tool can only read text files, not directories. To read a directory, use an ls command via the bash tool.\n\
        - You can call multiple tools in a single response. It is always better to speculatively read multiple potentially useful files in parallel.\n\
        - If you read a file that exists but has empty contents you will receive a system reminder warning in place of file contents."
    }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The absolute path to the file to read" },
                    "offset": { "type": "integer", "description": "The line number to start reading from. Only provide if the file is too large to read at once", "default": 0 },
                    "limit": { "type": "integer", "description": "The number of lines to read. Only provide if the file is too large to read at once.", "default": 2000 }
                },
                "required": ["file_path"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"].as_str().ok_or(ToolError::InvalidParams("file_path is required".to_string()))?;
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"].as_u64().unwrap_or(2000) as usize;

        const MAX_LINE_LENGTH: usize = 10_000;
        const MAX_TOTAL_SIZE: usize = 1_024_000;

        let file = fs::File::open(path_str).map_err(|e| ToolError::IoError(format!("Failed to open file: {}", e)))?;
        let reader = BufReader::new(file);
        
        let mut lines = Vec::new();
        let mut total_size = 0;

        for (i, line) in reader.lines().enumerate() {
            if i < offset { continue; }
            if i >= offset + limit { break; }
            let content = line.map_err(|e| ToolError::IoError(format!("Error reading line {}: {}", i, e)))?;
            if content.len() > MAX_LINE_LENGTH {
                return Err(ToolError::ExecutionFailed(format!("Line {} is too long. Use 'grep' or 'edit_file'.", i + 1)));
            }
            total_size += content.len();
            if total_size > MAX_TOTAL_SIZE {
                return Err(ToolError::ExecutionFailed("Total content size exceeds 1MB limit.".to_string()));
            }
            lines.push(format!("{:>6}\t{}", i + 1, content));
        }
        Ok(ToolCallResult::success(Some(lines.join("\n")), None))
    }
}

pub struct WriteFile;

#[async_trait]
impl ToolDefinition for WriteFile {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { 
        "Writes a file to the local filesystem.\n\n\
        Usage:\n\
        - This tool will overwrite the existing file if there is one at the provided path.\n\
        - If this is an existing file, you MUST use the read_file tool first to read the file's contents. This tool will fail if you did not read the file first.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the user.\n\
        - Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked."
    }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The absolute path to the file to write (must be absolute, not relative)" },
                    "content": { "type": "string", "description": "The content to write to the file" }
                },
                "required": ["file_path", "content"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"].as_str().ok_or(ToolError::InvalidParams("file_path is required".to_string()))?;
        let content = params["content"].as_str().ok_or(ToolError::InvalidParams("content is required".to_string()))?;
        let path = Path::new(path_str);
        if path.exists() {
            let bak = path.with_extension("bak");
            fs::copy(path, bak).ok();
        }
        fs::write(path, content).map_err(|e| ToolError::IoError(format!("Write failed: {}", e)))?;
        Ok(ToolCallResult::success(Some("File written successfully.".into()), None))
    }
}

pub struct EditFile;

#[async_trait]
impl ToolDefinition for EditFile {
    fn name(&self) -> &str { "edit_file" }
    fn description(&self) -> &str { 
        "Performs exact string replacements in files.\n\n\
        Usage:\n\
        - You must use your `read_file` tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file. \n\
        - When editing text from read_file tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: spaces + line number + tab. Everything after that tab is the actual file content to match. Never include any part of the line number prefix in the old_string or new_string.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\
        - The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.\n\
        - Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."
    }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The absolute path to the file to modify" },
                    "old_string": { "type": "string", "description": "The text to replace" },
                    "new_string": { "type": "string", "description": "The text to replace it with (must be different from old_string)" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences of old_string (default false)", "default": false }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"].as_str().ok_or(ToolError::InvalidParams("file_path is required".to_string()))?;
        let old_str = params["old_string"].as_str().ok_or(ToolError::InvalidParams("old_string is required".to_string()))?;
        let new_str = params["new_string"].as_str().ok_or(ToolError::InvalidParams("new_string is required".to_string()))?;
        let replace_all = params["replace_all"].as_bool().unwrap_or(false);
        let content = fs::read_to_string(path_str).map_err(|e| ToolError::IoError(format!("Read failed: {}", e)))?;
        let matches: Vec<_> = content.match_indices(old_str).collect();
        if matches.is_empty() {
            return Err(ToolError::ExecutionFailed(format!("String '{}' not found.", old_str)));
        }
        if !replace_all && matches.len() > 1 {
            return Err(ToolError::ExecutionFailed(format!("String '{}' is not unique (found {} matches).", old_str, matches.len())));
        }
        let new_content = if replace_all { content.replace(old_str, new_str) } else { content.replacen(old_str, new_str, 1) };
        fs::write(path_str, new_content).map_err(|e| ToolError::IoError(format!("Edit write failed: {}", e)))?;
        Ok(ToolCallResult::success(Some("File edited successfully.".into()), None))
    }
}

pub struct ListDir;

#[async_trait]
impl ToolDefinition for ListDir {
    fn name(&self) -> &str { "list_dir" }
    fn description(&self) -> &str { "Lists files and directories in a given path. To read a file, use the read_file tool instead." }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to list." },
                    "recursive": { "type": "boolean", "default": false }
                },
                "required": ["path"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["path"].as_str().ok_or(ToolError::InvalidParams("path is required".to_string()))?;
        let recursive = params["recursive"].as_bool().unwrap_or(false);
        let mut entries = vec![];
        if recursive {
            for entry in walkdir::WalkDir::new(path_str).max_depth(3).into_iter().flatten() {
                entries.push(entry.path().to_string_lossy().to_string());
            }
        } else if let Ok(read_dir) = fs::read_dir(path_str) {
            for entry in read_dir.flatten() {
                entries.push(entry.path().to_string_lossy().to_string());
            }
        }
        Ok(ToolCallResult::success(None, Some(json!(entries))))
    }
}
