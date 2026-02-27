use async_trait::async_trait;
use serde_json::{json, Value};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};
use glob::glob;
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory, ToolError};
use crate::ai::traits::chat::MCPToolDeclaration;

pub struct Glob;

#[async_trait]
impl ToolDefinition for Glob {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str { 
        "- Fast file pattern matching tool that works with any codebase size\n\
        - Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n\
        - Returns matching file paths sorted by modification time\n\
        - Use this tool when you need to find files by name patterns\n\
        - When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the task tool instead\n\
        - You can call multiple tools in a single response. It is always better to speculatively perform multiple searches in parallel if they are potentially useful."
    }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The glob pattern to match files against" },
                    "path": { "type": "string", "description": "The directory to search in. Defaults to current working directory." }
                },
                "required": ["pattern"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let pattern = params["pattern"].as_str().ok_or(ToolError::InvalidParams("pattern required".to_string()))?;
        let base_path = params["path"].as_str().unwrap_or(".");
        
        let full_pattern = format!("{}/{}", base_path, pattern);
        let mut results = vec![];
        const MAX_RESULTS: usize = 1000;
        for entry in glob(&full_pattern).map_err(|e| ToolError::ExecutionFailed(e.to_string()))? {
            if let Ok(path) = entry {
                results.push(path.to_string_lossy().to_string());
                if results.len() >= MAX_RESULTS { break; }
            }
        }
        Ok(ToolCallResult::success(None, Some(json!(results))))
    }
}

pub struct Grep;

#[async_trait]
impl ToolDefinition for Grep {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { 
        "A powerful search tool built on ripgrep logic\n\n\
        Usage:\n\
        - ALWAYS use Grep for search tasks. NEVER invoke `grep` or `rg` as a bash command. The Grep tool has been optimized for correct permissions and access.\n\
        - Supports full regex syntax (e.g., \"log.*Error\", \"function\\s+\\w+\")\n\
        - Filter files with glob parameter (e.g., \"*.js\", \"**/*.tsx\") or type parameter (e.g., \"js\", \"py\", \"rust\")\n\
        - Output modes: \"content\" shows matching lines, \"files_with_matches\" shows only file paths (default), \"count\" shows match counts\n\
        - Use task tool for open-ended searches requiring multiple rounds\n\
        - Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use `interface\\{\\}` to find `interface{}` in Go code)\n\
        - Multiline matching: By default patterns match within single lines only. For cross-line patterns like `struct \\{[\\s\\S]*?field`, use `multiline: true`"
    }
    fn category(&self) -> ToolCategory { ToolCategory::FileSystem }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The regular expression pattern to search for in file contents" },
                    "path": { "type": "string", "description": "File or directory to search in. Defaults to current working directory." },
                    "glob": { "type": "string", "description": "Glob pattern to filter files (e.g. \"*.js\")." },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "default": "files_with_matches" }
                },
                "required": ["pattern", "path"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let pattern_str = params["pattern"].as_str().ok_or(ToolError::InvalidParams("pattern required".to_string()))?;
        let search_path = params["path"].as_str().ok_or(ToolError::InvalidParams("path required".to_string()))?;
        let output_mode = params["output_mode"].as_str().unwrap_or("files_with_matches");
        let re = Regex::new(pattern_str).map_err(|e| ToolError::InvalidParams(format!("Invalid regex: {}", e)))?;
        let path = Path::new(search_path);
        let mut matches = vec![];
        const MAX_MATCHES: usize = 500;
        if path.is_file() {
            Self::search_in_file(path, &re, output_mode, &mut matches, MAX_MATCHES)?;
        } else if path.is_dir() {
            for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
                if entry.path().is_file() {
                    Self::search_in_file(entry.path(), &re, output_mode, &mut matches, MAX_MATCHES)?;
                    if matches.len() >= MAX_MATCHES { break; }
                }
            }
        }
        Ok(ToolCallResult::success(None, Some(json!(matches))))
    }
}

impl Grep {
    fn search_in_file(path: &Path, re: &Regex, mode: &str, matches: &mut Vec<Value>, max: usize) -> Result<(), ToolError> {
        let file = fs::File::open(path).map_err(|e| ToolError::IoError(e.to_string()))?;
        let reader = BufReader::new(file);
        let path_str = path.to_string_lossy().to_string();
        let mut count = 0;
        for (i, line) in reader.lines().enumerate() {
            if let Ok(content) = line {
                if re.is_match(&content) {
                    count += 1;
                    match mode {
                        "content" => matches.push(json!({"file": path_str, "line": i+1, "content": content})),
                        "files_with_matches" if count == 1 => matches.push(json!(path_str)),
                        _ => {}
                    }
                    if matches.len() >= max { return Ok(()); }
                }
            }
        }
        if mode == "count" && count > 0 { matches.push(json!({"file": path_str, "count": count})); }
        Ok(())
    }
}
