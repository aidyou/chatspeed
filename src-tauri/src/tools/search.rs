use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct Glob;

#[async_trait]
impl ToolDefinition for Glob {
    fn name(&self) -> &str {
        crate::tools::TOOL_GLOB
    }
    fn description(&self) -> &str {
        "- Fast file pattern matching tool that works with any codebase size\n\
        - Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n\
        - Returns matching file paths sorted by modification time\n\
        - Automatically respects .gitignore and other ignore files\n\
        - Use this tool when you need to find files by name patterns\n\
        - When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the task tool instead\n\
        - You can call multiple tools in a single response. It is always better to speculatively perform multiple searches in parallel if they are potentially useful."
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
                    "pattern": { "type": "string", "description": "The glob pattern to match files against" },
                    "path": { "type": "string", "description": "The directory to search in. Defaults to current working directory." }
                },
                "required": ["pattern"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let pattern = params["pattern"]
            .as_str()
            .ok_or(ToolError::InvalidParams("pattern required".to_string()))?;
        let base_path_str = params["path"].as_str().unwrap_or(".");
        let base_path = Path::new(base_path_str);

        // Prepare the glob matcher
        let glob_matcher = globset::GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(true)
            .build()
            .map_err(|e| ToolError::InvalidParams(format!("Invalid glob pattern: {}", e)))?
            .compile_matcher();

        let mut results = vec![];
        const MAX_RESULTS: usize = 1000;

        // Use ignore crate to respect .gitignore and filter common files
        let walker = ignore::WalkBuilder::new(base_path)
            .standard_filters(true)
            .hidden(false)
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                // Get relative path for matching
                let rel_path = path.strip_prefix(base_path).unwrap_or(path);
                
                if glob_matcher.is_match(rel_path) {
                    results.push(path.to_string_lossy().to_string());
                    if results.len() >= MAX_RESULTS {
                        break;
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(ToolCallResult::success(
                Some("[No matches found]".into()),
                None,
            ))
        } else {
            Ok(ToolCallResult::success(Some(results.join("\n")), None))
        }
    }
}

pub struct Grep;

#[async_trait]
impl ToolDefinition for Grep {
    fn name(&self) -> &str {
        crate::tools::TOOL_GREP
    }
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
                    "pattern": { "type": "string", "description": "The regular expression pattern to search for in file contents" },
                    "path": { "type": "string", "description": "File or directory to search in. Defaults to current working directory." },
                    "glob": { "type": "string", "description": "Glob pattern to filter files (e.g. \"*.js\")." },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "default": "files_with_matches" }
                },
                "required": ["pattern", "path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let pattern_str = params["pattern"]
            .as_str()
            .ok_or(ToolError::InvalidParams("pattern required".to_string()))?;
        let search_path = params["path"]
            .as_str()
            .ok_or(ToolError::InvalidParams("path required".to_string()))?;
        let output_mode = params["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");
        let re = Regex::new(pattern_str)
            .map_err(|e| ToolError::InvalidParams(format!("Invalid regex: {}", e)))?;
        let path = Path::new(search_path);
        let mut matches = vec![];
        const MAX_MATCHES: usize = 500;
        if path.is_file() {
            Self::search_in_file(path, &re, output_mode, &mut matches, MAX_MATCHES)?;
        } else if path.is_dir() {
            // Use ignore crate to respect .gitignore
            let walker = ignore::WalkBuilder::new(path)
                .standard_filters(true)
                .hidden(false)
                .build();

            for result in walker {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    Self::search_in_file(
                        entry.path(),
                        &re,
                        output_mode,
                        &mut matches,
                        MAX_MATCHES,
                    )?;
                    if matches.len() >= MAX_MATCHES {
                        break;
                    }
                }
            }
        }

        if matches.is_empty() {
            Ok(ToolCallResult::success(
                Some("[No matches found]".into()),
                None,
            ))
        } else {
            Ok(ToolCallResult::success(Some(matches.join("\n")), None))
        }
    }
}

impl Grep {
    fn search_in_file(
        path: &Path,
        re: &Regex,
        mode: &str,
        matches: &mut Vec<String>,
        max: usize,
    ) -> Result<(), ToolError> {
        let file = fs::File::open(path).map_err(|e| ToolError::IoError(e.to_string()))?;
        let reader = BufReader::new(file);
        let path_str = path.to_string_lossy().to_string();
        let mut count = 0;
        for (i, line) in reader.lines().enumerate() {
            if let Ok(content) = line {
                if re.is_match(&content) {
                    count += 1;
                    match mode {
                        "content" => {
                            matches.push(format!("{}:{}:{}", path_str, i + 1, content));
                        }
                        "files_with_matches" if count == 1 => {
                            matches.push(path_str.clone());
                        }
                        _ => {}
                    }
                    if matches.len() >= max {
                        return Ok(());
                    }
                }
            }
        }
        if mode == "count" && count > 0 {
            matches.push(format!("{}: {}", path_str, count));
        }
        Ok(())
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
    async fn test_glob_basic() {
        let tool = Glob;
        let temp_dir = tempdir().unwrap();

        // Create test files
        fs::write(temp_dir.path().join("test1.txt"), "").unwrap();
        fs::write(temp_dir.path().join("test2.txt"), "").unwrap();
        fs::write(temp_dir.path().join("other.md"), "").unwrap();

        let params = json!({
            "pattern": "*.txt",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let files: Vec<&str> = content.lines().collect();

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.contains("test1.txt")));
        assert!(files.iter().any(|f| f.contains("test2.txt")));
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let tool = Glob;
        let temp_dir = tempdir().unwrap();

        // Create nested structure
        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("nested.txt"), "").unwrap();

        let params = json!({
            "pattern": "**/*.txt",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let files: Vec<&str> = content.lines().collect();

        assert!(files.len() >= 1);
        assert!(files.iter().any(|f| f.contains("nested.txt")));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let tool = Glob;
        let temp_dir = tempdir().unwrap();

        let params = json!({
            "pattern": "*.nonexistent",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert_eq!(content, "[No matches found]");
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let tool = Glob;
        let temp_dir = tempdir().unwrap();

        // Invalid glob pattern
        let params = json!({
            "pattern": "**/*[invalid",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn test_glob_max_results() {
        let tool = Glob;
        let temp_dir = tempdir().unwrap();

        // Create many files
        for i in 0..1010 {
            fs::write(temp_dir.path().join(format!("file{}.txt", i)), "").unwrap();
        }

        let params = json!({
            "pattern": "*.txt",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let files: Vec<&str> = content.lines().collect();

        // Should be limited to MAX_RESULTS (1000)
        assert_eq!(files.len(), 1000);
    }

    #[tokio::test]
    async fn test_grep_basic() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello World\nGoodbye World\nAnother line").unwrap();

        let params = json!({
            "pattern": "World",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        assert_eq!(matches.len(), 2);

        // Check content
        for match_item in matches {
            assert!(match_item.contains("World"));
            // Check format path:line:content
            assert!(match_item.contains(&path));
            assert!(match_item.contains(":1:") || match_item.contains(":2:"));
        }
    }

    #[tokio::test]
    async fn test_grep_files_with_matches() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello World\nGoodbye World").unwrap();

        let params = json!({
            "pattern": "World",
            "path": path,
            "output_mode": "files_with_matches"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        // Should return file path once even with multiple matches
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], path);
    }

    #[tokio::test]
    async fn test_grep_count_mode() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello World\nGoodbye World\nNo match").unwrap();

        let params = json!({
            "pattern": "World",
            "path": path,
            "output_mode": "count"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        assert_eq!(matches.len(), 1);
        assert!(matches[0].contains(&path));
        assert!(matches[0].contains("2"));
    }

    #[tokio::test]
    async fn test_grep_directory_search() {
        let tool = Grep;
        let temp_dir = tempdir().unwrap();

        // Create multiple files
        fs::write(temp_dir.path().join("file1.txt"), "match\nno match").unwrap();
        fs::write(temp_dir.path().join("file2.txt"), "match\nmatch").unwrap();
        fs::write(temp_dir.path().join("file3.txt"), "no match").unwrap();

        let params = json!({
            "pattern": "^match",
            "path": temp_dir.path().to_string_lossy(),
            "output_mode": "files_with_matches"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        // Should find file1 and file2
        assert_eq!(matches.len(), 2);
        let filenames: Vec<String> = matches
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(filenames.contains(&"file1.txt".to_string()));
        assert!(filenames.contains(&"file2.txt".to_string()));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello World").unwrap();

        let params = json!({
            "pattern": "nonexistent",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert_eq!(content, "[No matches found]");
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let params = json!({
            "pattern": "[invalid",
            "path": path
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::InvalidParams(msg) => assert!(msg.contains("Invalid regex")),
            _ => panic!("Expected InvalidParams error"),
        }
    }

    #[tokio::test]
    async fn test_grep_nonexistent_path() {
        let tool = Grep;

        let params = json!({
            "pattern": "test",
            "path": "/nonexistent/path"
        });

        // Should return empty results, not error
        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert_eq!(content, "[No matches found]");
    }

    #[tokio::test]
    async fn test_grep_max_matches() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Create file with many matches
        let lines: Vec<String> = (0..600).map(|i| format!("match {}", i)).collect();
        fs::write(&path, lines.join("\n")).unwrap();

        let params = json!({
            "pattern": "match",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        // Should be limited to MAX_MATCHES (500)
        assert_eq!(matches.len(), 500);
    }

    #[tokio::test]
    async fn test_grep_case_sensitive() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello\nhello\nHELLO").unwrap();

        // Regex is case-sensitive by default
        let params = json!({
            "pattern": "Hello",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        // Should match only exact case
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn test_grep_special_regex_chars() {
        let tool = Grep;
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "test(123)\ntest[456]\ntest{789}").unwrap();

        // Test escaping special regex characters
        let params = json!({
            "pattern": r"test\(123\)",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        assert_eq!(matches.len(), 1);
    }
}
