use crate::ai::traits::chat::MCPToolDeclaration;
use crate::libs::ai_temp::{display_ai_temp_path, resolve_ai_temp_path};
use crate::tools::llm_output::{preview_grep_lines_for_llm, preview_path_lines_for_llm};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::security::PathGuard;
use async_trait::async_trait;
use globset::{Glob as GlobPattern, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn primary_directory(path_guard: Option<&Arc<RwLock<PathGuard>>>) -> PathBuf {
    path_guard
        .and_then(|guard| guard.read().ok())
        .and_then(|guard| guard.get_primary_root().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .and_then(|path| fs::canonicalize(&path).ok().or(Some(path)))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_tool_path(path_str: &str, path_guard: Option<&Arc<RwLock<PathGuard>>>) -> PathBuf {
    let path = resolve_ai_temp_path(Path::new(path_str));
    if path.is_absolute() {
        path
    } else {
        primary_directory(path_guard).join(path)
    }
}

fn display_path_for_tool_output(
    path: &Path,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> String {
    if let Some(display_path) = display_ai_temp_path(path) {
        return display_path;
    }

    let primary_dir = primary_directory(path_guard);
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(relative) = canonical_path.strip_prefix(&primary_dir) {
        if relative.as_os_str().is_empty() {
            ".".to_string()
        } else {
            relative.to_string_lossy().to_string()
        }
    } else {
        path.to_string_lossy().to_string()
    }
}

fn result_path_for_tool_output(path: &Path) -> String {
    display_ai_temp_path(path).unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[derive(Clone, Default)]
pub struct Glob {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl Glob {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for Glob {
    fn name(&self) -> &str {
        crate::tools::TOOL_GLOB
    }
    fn description(&self) -> &str {
        "- Fast file pattern matching tool that works with any codebase size\n\
        - Finds files by path/name patterns; it does not search file contents\n\
        - Supports glob patterns like \"**/*.js\", \"src/**/*.ts\", or \"**/{Cargo.toml,package.json}\"\n\
        - Returns matching file paths, capped at 1000 matches\n\
        - Paths under the primary working directory are shown as relative paths; matches in other authorized directories remain absolute\n\
        - Automatically respects .gitignore and other ignore files\n\
        - Use this tool before grep/read_file when you need to discover likely files by extension, directory, or filename\n\
        - When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the sub_agent_run tool instead\n\
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
                    "pattern": { "type": "string", "description": "The glob pattern to match files against, relative to the search path. Examples: \"**/*.rs\", \"src/**/*.ts\", \"**/{Cargo.toml,package.json}\"." },
                    "path": { "type": "string", "description": "The directory to search in. Use a relative path for the primary working directory; use an absolute path for other authorized directories. Defaults to the primary working directory." }
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
        let base_path = resolve_tool_path(base_path_str, self.path_guard.as_ref());
        let display_base_path = display_path_for_tool_output(&base_path, self.path_guard.as_ref());

        // Prepare the glob matcher
        let glob_matcher = globset::GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(true)
            .build()
            .map_err(|e| ToolError::InvalidParams(format!("Invalid glob pattern: {}", e)))?
            .compile_matcher();

        let mut results = vec![];
        const MAX_RESULTS: usize = 1000;
        let mut truncated = false;

        // Use ignore crate to respect .gitignore and filter common files
        let walker = ignore::WalkBuilder::new(&base_path)
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
                let rel_path = path.strip_prefix(&base_path).unwrap_or(path);

                if glob_matcher.is_match(rel_path) {
                    results.push(display_path_for_tool_output(path, self.path_guard.as_ref()));
                    if results.len() >= MAX_RESULTS {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(ToolCallResult::success(
                Some("[No matches found]".into()),
                Some(json!({
                    "pattern": pattern,
                    "path": result_path_for_tool_output(&base_path),
                    "display_path": display_base_path,
                    "count": 0,
                    "truncated": false,
                    "llm_content": "[No matches found]"
                })),
            ))
        } else {
            let count = results.len();
            Ok(ToolCallResult::success(
                Some(results.join("\n")),
                Some(json!({
                    "pattern": pattern,
                    "path": result_path_for_tool_output(&base_path),
                    "display_path": display_base_path,
                    "count": count,
                    "truncated": truncated,
                    "max_results": MAX_RESULTS,
                    "llm_content": preview_path_lines_for_llm(&results)
                })),
            ))
        }
    }
}

#[derive(Clone, Default)]
pub struct Grep {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl Grep {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

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
        - Supports compound searches with regex alternation (e.g., \"foo|bar|baz\", \"create_workflow|workflow_start|finalAudit\")\n\
        - Filter files with glob parameter (e.g., \"*.js\", \"**/*.tsx\", \"src-tauri/src/**/*.rs\")\n\
        - Skips common binary, media, archive, document, and executable files such as .so, .dll, .exe, .zip, .rar, .png, .jpg, and .pdf. Only text-like files are searched.\n\
        - Output modes: \"content\" shows matching lines with file and line number (default), \"files_with_matches\" shows only file paths, \"count\" shows match counts\n\
        - In content mode, very long matching lines are truncated from the first match position to keep output readable\n\
        - For efficient exploration, search several related terms at once, use content mode to get line numbers, then read only targeted files/ranges.\n\
        - Use files_with_matches only for broad searches where content mode would be too noisy; follow up with content mode before reading files.\n\
        - Use sub_agent_run for open-ended searches requiring multiple rounds\n\
        - Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use `interface\\{\\}` to find `interface{}` in Go code)\n\
        - Patterns match within single lines."
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
                    "pattern": { "type": "string", "description": "The regular expression pattern to search for in file contents. Use alternation (foo|bar|baz) for compound searches." },
                    "path": { "type": "string", "description": "File or directory to search in. Use a relative path for the primary working directory; use an absolute path for other authorized directories. Defaults to the primary working directory." },
                    "glob": { "type": "string", "description": "Glob pattern to filter files (e.g. \"*.js\", \"**/*.tsx\", \"src/**/*.rs\")." },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "default": "content", "description": "Output format. content is the default and returns file:line:matched_content. files_with_matches returns only file paths for broad noise-reduction searches. count returns match counts per file." }
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
        let output_mode = params["output_mode"].as_str().unwrap_or("content");
        if !matches!(output_mode, "content" | "files_with_matches" | "count") {
            return Err(ToolError::InvalidParams(format!(
                "output_mode must be one of content, files_with_matches, or count; got '{}'",
                output_mode
            )));
        }
        let glob_set = Self::build_glob_set(params["glob"].as_str())?;
        let re = Regex::new(pattern_str)
            .map_err(|e| ToolError::InvalidParams(format!("Invalid regex: {}", e)))?;
        let path = resolve_tool_path(search_path, self.path_guard.as_ref());
        let display_path = display_path_for_tool_output(&path, self.path_guard.as_ref());
        let mut matches = vec![];
        const MAX_MATCHES: usize = 500;

        if !path.exists() {
            return Err(ToolError::IoError(format!(
                "Search path not found: {}. Use list_dir/glob to verify the correct path.",
                display_path
            )));
        }

        if path.is_file() {
            if Self::matches_glob(&path, path.parent(), glob_set.as_ref())
                && Self::is_searchable_text_file(&path)
            {
                Self::search_in_file(
                    &path,
                    &re,
                    output_mode,
                    &mut matches,
                    MAX_MATCHES,
                    self.path_guard.as_ref(),
                )?;
            }
        } else if path.is_dir() {
            // Use ignore crate to respect .gitignore
            let walker = ignore::WalkBuilder::new(&path)
                .standard_filters(true)
                .hidden(false)
                .build();

            for result in walker {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    if !Self::matches_glob(entry.path(), Some(&path), glob_set.as_ref()) {
                        continue;
                    }
                    if !Self::is_searchable_text_file(entry.path()) {
                        continue;
                    }
                    Self::search_in_file(
                        entry.path(),
                        &re,
                        output_mode,
                        &mut matches,
                        MAX_MATCHES,
                        self.path_guard.as_ref(),
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
                Some(json!({
                    "llm_content": "[No matches found]"
                })),
            ))
        } else {
            Ok(ToolCallResult::success(
                Some(matches.join("\n")),
                Some(json!({
                    "llm_content": preview_grep_lines_for_llm(&matches, output_mode)
                })),
            ))
        }
    }
}

impl Grep {
    const MAX_CONTENT_MATCH_CHARS: usize = 500;
    const TEXT_SNIFF_BYTES: usize = 8192;
    const SKIPPED_BINARY_EXTENSIONS: &'static [&'static str] = &[
        "7z", "a", "apk", "avi", "bin", "bmp", "bz2", "class", "cur", "dat", "deb", "dib", "dll",
        "dmg", "doc", "docm", "docx", "dylib", "ear", "elc", "eot", "epub", "exe", "flac", "gif",
        "gz", "icns", "ico", "img", "iso", "jar", "jpeg", "jpg", "lib", "lz", "lz4", "m4a", "mkv",
        "mov", "mp3", "mp4", "mpeg", "mpg", "msi", "o", "obj", "ogg", "otf", "pdf", "pkg", "png",
        "ppt", "pptx", "pyc", "pyd", "rar", "so", "sqlite", "tar", "tif", "tiff", "ttf", "war",
        "wav", "webm", "webp", "woff", "woff2", "xls", "xlsb", "xlsm", "xlsx", "xz", "zip", "zst",
        "br", "cab", "cer", "crt", "der", "heic", "heif", "lockb", "parquet", "wasm", "woff",
        "woff2", "psd", "ai", "sketch", "blend", "db", "db3", "sqlite3", "rmeta", "rlib",
    ];

    fn build_glob_set(glob: Option<&str>) -> Result<Option<GlobSet>, ToolError> {
        let Some(glob) = glob.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };

        let mut builder = GlobSetBuilder::new();
        builder.add(
            GlobPattern::new(glob)
                .map_err(|e| ToolError::InvalidParams(format!("Invalid glob: {}", e)))?,
        );
        Ok(Some(builder.build().map_err(|e| {
            ToolError::InvalidParams(format!("Invalid glob: {}", e))
        })?))
    }

    fn matches_glob(path: &Path, root: Option<&Path>, glob_set: Option<&GlobSet>) -> bool {
        let Some(glob_set) = glob_set else {
            return true;
        };

        if glob_set.is_match(path) {
            return true;
        }

        root.and_then(|root| path.strip_prefix(root).ok())
            .map_or(false, |relative| glob_set.is_match(relative))
    }

    fn is_searchable_text_file(path: &Path) -> bool {
        if Self::has_blocked_binary_extension(path) {
            return false;
        }

        Self::looks_like_text_by_content(path)
    }

    fn has_blocked_binary_extension(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                let ext = ext.to_ascii_lowercase();
                Self::SKIPPED_BINARY_EXTENSIONS
                    .iter()
                    .any(|blocked| *blocked == ext)
            })
            .unwrap_or(false)
    }

    fn looks_like_text_by_content(path: &Path) -> bool {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return false,
        };

        let mut buffer = [0_u8; Self::TEXT_SNIFF_BYTES];
        let bytes_read = match file.read(&mut buffer) {
            Ok(bytes_read) => bytes_read,
            Err(_) => return false,
        };

        if bytes_read == 0 {
            return true;
        }

        let sample = &buffer[..bytes_read];
        if sample.contains(&0) {
            return false;
        }

        if std::str::from_utf8(sample).is_ok() {
            return true;
        }

        let suspicious = sample
            .iter()
            .filter(|&&b| {
                b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t' && b != 0x0C && b != 0x08
            })
            .count();

        let ratio = suspicious as f32 / sample.len() as f32;
        ratio < 0.01
    }

    fn search_in_file(
        path: &Path,
        re: &Regex,
        mode: &str,
        matches: &mut Vec<String>,
        max: usize,
        path_guard: Option<&Arc<RwLock<PathGuard>>>,
    ) -> Result<(), ToolError> {
        let file = fs::File::open(path).map_err(|e| ToolError::IoError(e.to_string()))?;
        let reader = BufReader::new(file);
        let mut count = 0;
        for (i, line) in reader.lines().enumerate() {
            if let Ok(content) = line {
                if let Some(matched) = re.find(&content) {
                    count += 1;
                    match mode {
                        "content" => {
                            let display_content =
                                Self::format_content_match(&content, matched.start());
                            matches.push(format!(
                                "{}:{}:{}",
                                display_path_for_tool_output(path, path_guard),
                                i + 1,
                                display_content
                            ));
                        }
                        "files_with_matches" if count == 1 => {
                            matches.push(display_path_for_tool_output(path, path_guard));
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
            matches.push(format!(
                "{}: {}",
                display_path_for_tool_output(path, path_guard),
                count
            ));
        }
        Ok(())
    }

    fn format_content_match(content: &str, match_start: usize) -> String {
        if content.chars().count() <= Self::MAX_CONTENT_MATCH_CHARS {
            return content.to_string();
        }

        let prefix_chars = content[..match_start].chars().count();
        let suffix = content.get(match_start..).unwrap_or(content);
        let suffix_chars = suffix.chars().count();
        let mut snippet = suffix
            .chars()
            .take(Self::MAX_CONTENT_MATCH_CHARS)
            .collect::<String>();
        if match_start > 0 {
            snippet = format!("[offset={} chars] {}", prefix_chars, snippet);
        }
        if suffix_chars > Self::MAX_CONTENT_MATCH_CHARS {
            snippet.push_str(&format!(
                " [remain={} chars]",
                suffix_chars - Self::MAX_CONTENT_MATCH_CHARS
            ));
        }
        snippet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::{tempdir, tempdir_in, NamedTempFile};

    fn make_relative_test_dir() -> (tempfile::TempDir, PathBuf) {
        let root = primary_directory(None);
        let temp_dir = tempdir_in(&root).unwrap();
        let relative = temp_dir.path().strip_prefix(&root).unwrap().to_path_buf();
        (temp_dir, relative)
    }

    #[tokio::test]
    async fn test_glob_basic() {
        let tool = Glob::default();
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
        let tool = Glob::default();
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
        let tool = Glob::default();
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
        let tool = Glob::default();
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
        let tool = Glob::default();
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
    async fn test_glob_supports_relative_path_and_returns_relative_matches() {
        let tool = Glob::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);
        fs::create_dir(absolute_root.join("src")).unwrap();
        fs::write(absolute_root.join("src").join("lib.rs"), "").unwrap();

        let result = tool
            .call(json!({
                "pattern": "**/*.rs",
                "path": relative_root.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        let content = result.content.unwrap();
        assert!(content.contains(&format!("{}/src/lib.rs", relative_root.to_string_lossy())));
        assert!(!content.contains(&primary_directory(None).to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn test_glob_includes_llm_content_preview() {
        let tool = Glob::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);

        for i in 0..220 {
            fs::write(absolute_root.join(format!("file-{:03}.rs", i)), "").unwrap();
        }

        let result = tool
            .call(json!({
                "pattern": "**/*.rs",
                "path": relative_root.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        let structured = result.structured_content.unwrap();
        let llm_content = structured["llm_content"].as_str().unwrap_or_default();

        assert!(llm_content.contains("file-000.rs"));
        assert!(llm_content.contains("truncated 20 additional lines"));
        assert!(!llm_content.contains("file-219.rs"));
    }

    #[tokio::test]
    async fn test_grep_basic() {
        let tool = Grep::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();
        let display_path = display_ai_temp_path(Path::new(&path)).unwrap();

        fs::write(&path, "Hello World\nGoodbye World\nAnother line").unwrap();

        let params = json!({
            "pattern": "World",
            "path": display_path,
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
            assert!(match_item.contains(&display_path));
            assert!(match_item.contains(":1:") || match_item.contains(":2:"));
        }
    }

    #[tokio::test]
    async fn test_grep_files_with_matches() {
        let tool = Grep::default();
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
        let display_path = display_ai_temp_path(Path::new(&path)).unwrap();

        // Should return file path once even with multiple matches
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], display_path);
    }

    #[tokio::test]
    async fn test_grep_defaults_to_content_mode() {
        let tool = Grep::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "Hello World").unwrap();

        let params = json!({
            "pattern": "World",
            "path": path
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert!(content.contains(":1:Hello World"));
    }

    #[tokio::test]
    async fn test_grep_content_truncates_long_lines_from_match() {
        let tool = Grep::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let long_line = format!("{}TARGET{}", "a".repeat(2000), "b".repeat(2000));
        fs::write(&path, long_line).unwrap();

        let params = json!({
            "pattern": "TARGET",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert!(content.contains("[offset=2000 chars] TARGET"));
        assert!(content.contains("[remain=1506 chars]"));
        assert!(!content.contains(&"a".repeat(100)));
        assert!(content.len() < 800);
    }

    #[tokio::test]
    async fn test_grep_count_mode() {
        let tool = Grep::default();
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
        let display_path = display_ai_temp_path(Path::new(&path)).unwrap();

        assert_eq!(matches.len(), 1);
        assert!(matches[0].contains(&display_path));
        assert!(matches[0].contains("2"));
    }

    #[tokio::test]
    async fn test_grep_directory_search() {
        let tool = Grep::default();
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
    async fn test_grep_compound_regex_search() {
        let tool = Grep::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "create_workflow\nworkflow_start\nunrelated").unwrap();

        let params = json!({
            "pattern": "create_workflow|workflow_start|finalAudit",
            "path": path,
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        assert_eq!(matches.len(), 2);
        assert!(content.contains("create_workflow"));
        assert!(content.contains("workflow_start"));
    }

    #[tokio::test]
    async fn test_grep_glob_filter() {
        let tool = Grep::default();
        let temp_dir = tempdir().unwrap();

        fs::write(temp_dir.path().join("file.rs"), "target_symbol").unwrap();
        fs::write(temp_dir.path().join("file.ts"), "target_symbol").unwrap();

        let params = json!({
            "pattern": "target_symbol",
            "path": temp_dir.path().to_string_lossy(),
            "glob": "**/*.rs",
            "output_mode": "files_with_matches"
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let matches: Vec<&str> = content.lines().collect();

        assert_eq!(matches.len(), 1);
        assert!(matches[0].ends_with("file.rs"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let tool = Grep::default();
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
        let tool = Grep::default();
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
        let tool = Grep::default();

        let params = json!({
            "pattern": "test",
            "path": "/nonexistent/path"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::IoError(msg) => assert!(msg.contains("Search path not found")),
            _ => panic!("Expected IoError error"),
        }
    }

    #[tokio::test]
    async fn test_grep_max_matches() {
        let tool = Grep::default();
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
        let tool = Grep::default();
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
    async fn test_grep_skips_binary_extension_files() {
        let tool = Grep::default();
        let temp_dir = tempdir().unwrap();
        let binary_path = temp_dir.path().join("libexample.so");

        fs::write(&binary_path, b"plain text with calibration symbol").unwrap();

        let params = json!({
            "pattern": "calibration",
            "path": temp_dir.path().to_string_lossy(),
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "[No matches found]");
    }

    #[tokio::test]
    async fn test_grep_skips_binary_content_without_extension() {
        let tool = Grep::default();
        let temp_dir = tempdir().unwrap();
        let binary_path = temp_dir.path().join("payload");

        fs::write(&binary_path, b"\0\0binary\0calibration\0").unwrap();

        let params = json!({
            "pattern": "calibration",
            "path": temp_dir.path().to_string_lossy(),
            "output_mode": "content"
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "[No matches found]");
    }

    #[tokio::test]
    async fn test_grep_special_regex_chars() {
        let tool = Grep::default();
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

    #[tokio::test]
    async fn test_grep_supports_relative_path_and_returns_relative_matches() {
        let tool = Grep::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);
        fs::create_dir(absolute_root.join("src")).unwrap();
        fs::write(
            absolute_root.join("src").join("main.rs"),
            "fn important_symbol() {}\n",
        )
        .unwrap();

        let result = tool
            .call(json!({
                "pattern": "important_symbol",
                "path": relative_root.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        let content = result.content.unwrap();
        assert!(content.contains(&format!(
            "{}/src/main.rs:1:",
            relative_root.to_string_lossy()
        )));
        assert!(!content.contains(&primary_directory(None).to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn test_grep_includes_llm_content_preview() {
        let tool = Grep::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);
        let file_path = absolute_root.join("matches.rs");

        let content = (1..=130)
            .map(|i| format!("target_match_{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        let result = tool
            .call(json!({
                "pattern": "target_match_",
                "path": relative_root.to_string_lossy().to_string(),
                "output_mode": "content"
            }))
            .await
            .unwrap();

        let structured = result.structured_content.unwrap();
        let llm_content = structured["llm_content"].as_str().unwrap_or_default();

        assert!(llm_content.contains("target_match_1"));
        assert!(llm_content.contains("target_match_120"));
        assert!(!llm_content.contains("target_match_130"));
    }
}
