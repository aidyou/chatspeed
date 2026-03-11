use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use async_trait::async_trait;
use chrono;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct ReadFile;

#[async_trait]
impl ToolDefinition for ReadFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_READ_FILE
    }
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
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
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
                    "file_path": { "type": "string", "description": "The absolute path to the file to read" },
                    "offset": { "type": "integer", "description": "The line number to start reading from. Only provide if the file is too large to read at once", "default": 0 },
                    "limit": { "type": "integer", "description": "The number of lines to read. Only provide if the file is too large to read at once.", "default": 2000 }
                },
                "required": ["file_path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"].as_u64().unwrap_or(2000) as usize;

        const MAX_LINE_LENGTH: usize = 10_000;
        const MAX_TOTAL_SIZE: usize = 1_024_000;

        let file = fs::File::open(path_str)
            .map_err(|e| ToolError::IoError(format!("Failed to open file: {}", e)))?;
        let reader = BufReader::new(file);

        let mut lines = Vec::new();
        let mut total_size = 0;

        for (i, line) in reader.lines().enumerate() {
            if i < offset {
                continue;
            }
            if i >= offset + limit {
                break;
            }
            let content =
                line.map_err(|e| ToolError::IoError(format!("Error reading line {}: {}", i, e)))?;
            if content.len() > MAX_LINE_LENGTH {
                return Err(ToolError::ExecutionFailed(format!(
                    "Line {} is too long. Use 'grep' or 'edit_file'.",
                    i + 1
                )));
            }
            total_size += content.len();
            if total_size > MAX_TOTAL_SIZE {
                return Err(ToolError::ExecutionFailed(
                    "Total content size exceeds 1MB limit.".to_string(),
                ));
            }
            lines.push(format!("{:>6}\t{}", i + 1, content));
        }
        Ok(ToolCallResult::success(Some(lines.join("\n")), None))
    }
}

pub struct WriteFile;

#[async_trait]
impl ToolDefinition for WriteFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_WRITE_FILE
    }
    fn description(&self) -> &str {
        "Writes a file to the local filesystem.\n\n\
        Usage:\n\
        - This tool will overwrite the existing file if there is one at the provided path.\n\
        - If this is an existing file, ensure you have viewed its full content (e.g., via `read_file` or user-provided context) before overwriting to avoid unintended data loss.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the user.\n\
        - Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
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
                    "file_path": { "type": "string", "description": "The absolute path to the file to write (must be absolute, not relative)" },
                    "content": { "type": "string", "description": "The content to write to the file" }
                },
                "required": ["file_path", "content"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let content = params["content"]
            .as_str()
            .ok_or(ToolError::InvalidParams("content is required".to_string()))?;
        let path = Path::new(path_str);
        
        let mut message = "File written successfully.".to_string();
        
        if path.exists() {
            // Safety: Create a timestamped backup before overwriting.
            let old_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
            let mut bak_name = path.file_name().unwrap_or_default().to_os_string();
            bak_name.push(".");
            bak_name.push(timestamp);
            bak_name.push(".bak");
            let bak = path.with_file_name(bak_name);
            
            if let Ok(_) = fs::copy(path, &bak) {
                message = format!("File overwritten successfully. Previous version ({} bytes) was backed up to {}.", 
                    old_size, bak.display());
            }
        } else {
            // Ensure parent directories exist for new files
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            message = "New file created successfully.".to_string();
        }
        
        fs::write(path, content).map_err(|e| ToolError::IoError(format!("Write failed: {}", e)))?;
        
        Ok(ToolCallResult::success(
            Some(message),
            None,
        ))
    }
}

pub struct EditFile;

#[async_trait]
impl ToolDefinition for EditFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_EDIT_FILE
    }
    fn description(&self) -> &str {
        "Performs exact string replacements in files.\n\n\
        Usage:\n\
        - Ensure you have viewed the full content of the file (e.g., via `read_file` or user-provided context) to confirm exact text and indentation before editing. \n\
        - When editing, ensure you preserve the exact indentation (tabs/spaces). If you used `read_file`, remember its output format: spaces + line number + tab. Everything after that tab is the actual file content to match. Never include any part of the line number prefix in the old_string or new_string.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\
        - The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.\n\
        - Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
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
                    "file_path": { "type": "string", "description": "The absolute path to the file to modify" },
                    "old_string": { "type": "string", "description": "The text to replace" },
                    "new_string": { "type": "string", "description": "The text to replace it with (must be different from old_string)" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences of old_string (default false)", "default": false }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let old_str = params["old_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "old_string is required".to_string(),
            ))?;
        let new_str = params["new_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "new_string is required".to_string(),
            ))?;
        
        if old_str == new_str {
            return Err(ToolError::InvalidParams("old_string and new_string are identical. No changes performed.".into()));
        }

        let replace_all = params["replace_all"].as_bool().unwrap_or(false);
        let content = fs::read_to_string(path_str)
            .map_err(|e| ToolError::IoError(format!("Read failed: {}. Ensure the file exists and is readable.", e)))?;
        
        let matches: Vec<_> = content.match_indices(old_str).collect();
        if matches.is_empty() {
            // Provide more diagnostic info to AI
            let lines_count = content.lines().count();
            return Err(ToolError::ExecutionFailed(format!(
                "The old_string was not found in the file (checked {} lines). \
                Please ensure you copied the text EXACTLY, including all whitespace and indentation. \
                Do NOT include line numbers in your old_string.",
                lines_count
            )));
        }
        
        if !replace_all && matches.len() > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "The old_string is not unique (found {} matches). \
                Please provide more surrounding context in your old_string to uniquely identify the location, \
                or use 'replace_all: true' if you want to replace all occurrences.",
                matches.len()
            )));
        }
        
        let new_content = if replace_all {
            content.replace(old_str, new_str)
        } else {
            content.replacen(old_str, new_str, 1)
        };
        
        fs::write(path_str, new_content)
            .map_err(|e| ToolError::IoError(format!("Edit write failed: {}. Check file permissions.", e)))?;
            
        Ok(ToolCallResult::success(
            Some("File edited successfully.".into()),
            None,
        ))
    }
}

pub struct ListDir;

#[async_trait]
impl ToolDefinition for ListDir {
    fn name(&self) -> &str {
        crate::tools::TOOL_LIST_DIR
    }
    fn description(&self) -> &str {
        "Lists files and directories in a given path. To read a file, use the read_file tool instead."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
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
                    "path": { "type": "string", "description": "Path to list." },
                    "recursive": { "type": "boolean", "default": false }
                },
                "required": ["path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["path"]
            .as_str()
            .ok_or(ToolError::InvalidParams("path is required".to_string()))?;
        let recursive = params["recursive"].as_bool().unwrap_or(false);
        let mut entries = vec![];

        // Use ignore crate to respect .gitignore
        let mut builder = ignore::WalkBuilder::new(path_str);
        builder.standard_filters(true).hidden(false);
        if !recursive {
            builder.max_depth(Some(1));
        }

        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip the root path itself
            if entry.depth() == 0 {
                continue;
            }

            entries.push(entry.path().to_string_lossy().to_string());
            if entries.len() >= 1000 {
                break;
            }
        }

        if entries.is_empty() {
            Ok(ToolCallResult::success(
                Some("Directory is empty or not found.".into()),
                None,
            ))
        } else {
            Ok(ToolCallResult::success(Some(entries.join("\n")), None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use tempfile::{tempdir, NamedTempFile};

    #[tokio::test]
    async fn test_read_file_basic() {
        let tool = ReadFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write some content
        fs::write(&path, "line1\nline2\nline3").unwrap();

        // Read entire file
        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_and_limit() {
        let tool = ReadFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write 10 lines
        let content: Vec<String> = (1..=10).map(|i| format!("line {}", i)).collect();
        fs::write(&path, content.join("\n")).unwrap();

        // Read lines 3-7 (offset=2, limit=5)
        let params = json!({
            "file_path": path,
            "offset": 2,
            "limit": 5
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();

        // Should contain lines 3-7
        assert!(output.contains("line 3"));
        assert!(output.contains("line 7"));
        // Should NOT contain lines 1-2 or 8-10
        assert!(!output.contains("line 1"));
        assert!(!output.contains("line 10"));
    }

    #[tokio::test]
    async fn test_read_file_line_length_limit() {
        let tool = ReadFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write a line that exceeds MAX_LINE_LENGTH (10,000)
        let long_line = "x".repeat(15000);
        fs::write(&path, long_line).unwrap();

        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                ToolError::ExecutionFailed(msg) => assert!(msg.contains("too long")),
                _ => panic!("Expected ExecutionFailed error"),
            }
        }
    }

    #[tokio::test]
    async fn test_read_file_total_size_limit() {
        let tool = ReadFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write content that exceeds MAX_TOTAL_SIZE (1,024,000)
        let line = "x".repeat(1000);
        let content: Vec<String> = (0..1500).map(|_| line.clone()).collect(); // 1,500,000 bytes
        fs::write(&path, content.join("\n")).unwrap();

        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                ToolError::ExecutionFailed(msg) => assert!(msg.contains("exceeds 1MB limit")),
                _ => panic!("Expected ExecutionFailed error"),
            }
        }
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFile;
        let params = json!({
            "file_path": "/nonexistent/path/file.txt"
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::IoError(_)));
    }

    #[tokio::test]
    async fn test_write_file_new() {
        let tool = WriteFile;
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");
        let path_str = path.to_string_lossy().to_string();

        let content = "Hello, World!";
        let params = json!({
            "file_path": path_str,
            "content": content
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "File written successfully.");

        // Verify file was written
        let actual_content = fs::read_to_string(&path).unwrap();
        assert_eq!(actual_content, content);
    }

    #[tokio::test]
    async fn test_write_file_overwrite() {
        let tool = WriteFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write initial content
        fs::write(&path, "original content").unwrap();

        // Overwrite with new content
        let new_content = "new content";
        let params = json!({
            "file_path": path,
            "content": new_content
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "File written successfully.");

        // Verify backup was created
        let backup_path = Path::new(&path).with_extension("bak");
        assert!(backup_path.exists());
        let backup_content = fs::read_to_string(backup_path).unwrap();
        assert_eq!(backup_content, "original content");

        // Verify new content
        let actual_content = fs::read_to_string(&path).unwrap();
        assert_eq!(actual_content, new_content);
    }

    #[tokio::test]
    async fn test_edit_file_basic() {
        let tool = EditFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write initial content
        let initial = "Old content\nMore old content";
        fs::write(&path, initial).unwrap();

        // Replace "Old" with "New"
        let params = json!({
            "file_path": path,
            "old_string": "Old",
            "new_string": "New"
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "File edited successfully.");

        let new_content = fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "New content\nMore old content");
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let tool = EditFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "apple apple apple";
        fs::write(&path, initial).unwrap();

        // Replace all "apple" with "orange"
        let params = json!({
            "file_path": path,
            "old_string": "apple",
            "new_string": "orange",
            "replace_all": true
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "File edited successfully.");

        let new_content = fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "orange orange orange");
    }

    #[tokio::test]
    async fn test_edit_file_non_unique_string() {
        let tool = EditFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "test test test";
        fs::write(&path, initial).unwrap();

        // Try to replace "test" without replace_all - should fail
        let params = json!({
            "file_path": path,
            "old_string": "test",
            "new_string": "updated"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => assert!(msg.contains("not unique")),
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_edit_file_string_not_found() {
        let tool = EditFile;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "some content").unwrap();

        let params = json!({
            "file_path": path,
            "old_string": "nonexistent",
            "new_string": "new"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => assert!(msg.contains("not found")),
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_list_dir_basic() {
        let tool = ListDir;
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        // Create some files and directories
        fs::write(temp_dir.path().join("file1.txt"), "").unwrap();
        fs::write(temp_dir.path().join("file2.txt"), "").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let params = json!({
            "path": path_str
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let entries: Vec<&str> = content.lines().collect();

        assert!(entries.len() >= 3);

        let path_strs: Vec<String> = entries
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(path_strs.contains(&"file1.txt".to_string()));
        assert!(path_strs.contains(&"file2.txt".to_string()));
    }

    #[tokio::test]
    async fn test_list_dir_recursive() {
        let tool = ListDir;
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        // Create nested structure
        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("nested.txt"), "").unwrap();

        let params = json!({
            "path": path_str,
            "recursive": true
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let entries: Vec<&str> = content.lines().collect();

        assert!(entries.len() >= 2);
        assert!(entries.iter().any(|p| p.contains("nested.txt")));
    }

    #[tokio::test]
    async fn test_list_dir_nonexistent() {
        let tool = ListDir;
        let params = json!({
            "path": "/nonexistent/directory"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        assert_eq!(content, "Directory is empty or not found.");
    }
}
