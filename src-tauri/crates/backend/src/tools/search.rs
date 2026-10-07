use crate::ai::traits::chat::MCPToolDeclaration;
use crate::libs::ai_temp::{display_ai_temp_path, resolve_ai_temp_path};
use crate::tools::llm_output::{preview_grep_lines_for_llm, preview_path_lines_for_llm};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
#[cfg(test)]
use crate::workflow::react::security::CHATSPEED_IGNORE_FILE;
use crate::workflow::react::security::{workspace_walk_builder, PathGuard};
use async_trait::async_trait;
use globset::{Glob as GlobPattern, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::VecDeque;
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

fn validate_search_path(
    path: &Path,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<(), ToolError> {
    if let Some(path_guard) = path_guard {
        let guard = path_guard
            .read()
            .map_err(|e| ToolError::ExecutionFailed(format!("PathGuard lock poisoned: {}", e)))?;
        guard
            .validate_for_listing(path, false)
            .map_err(|e| ToolError::ExecutionFailed(format!("Security Error: {}", e)))?;
    }
    Ok(())
}

fn is_search_path_allowed(
    path: &Path,
    is_dir: bool,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<bool, ToolError> {
    let Some(path_guard) = path_guard else {
        return Ok(true);
    };
    let guard = path_guard
        .read()
        .map_err(|error| ToolError::ExecutionFailed(format!("PathGuard lock poisoned: {error}")))?;
    guard
        .is_chatspeed_allowed(path, is_dir)
        .map_err(|error| ToolError::ExecutionFailed(format!("Security Error: {error}")))
}

fn configure_search_walker(base_path: &Path) -> ignore::Walk {
    workspace_walk_builder(base_path).build()
}

#[derive(Clone, Default)]
pub struct Grep {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl Grep {
    #[cfg(any(test, not(feature = "desktop")))]
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
        "Search files two ways: content search (pass `pattern`) or path-only search (omit `pattern`, use `glob`).\n\n\
        Usage:\n\
        - Use this instead of bash `grep`, `rg`, `find`, or `ls`.\n\
        - `pattern` is Rust regex; look-around and backreferences are unsupported, use alternation (\"foo|bar|baz\") for several terms.\n\
        - `glob` filters by path/name in both modes; brace alternatives are written verbatim, e.g. \"**/{AGENTS.md,CONSTITUTION.md}\" — do not add characters around the braces.\n\
        - Path-only search returns matching file paths only, never contents: use it to discover files by extension, directory, or file name, and always pass `glob` or every file matches.\n\
        - Path-only search never returns directories. To inspect a directory's immediate children, use list_dir.\n\
        - Content search returns matching lines; set `context_lines` to get surrounding lines instead of reading the file afterwards.\n\
        - Results are capped: check `truncated`, then narrow `path`, `glob`, or `pattern`.\n\
        - Only the declared parameters exist; shell flags such as -A/-B/-C/-n/-i and head_limit are ignored.\n\
        - Use read_file when you need a whole file."
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
                    "pattern": { "type": "string", "description": "Rust regex matched inside file contents. Omit it for path-only search: only `glob` is applied and contents are not read." },
                    "path": { "type": "string", "description": "File or directory to search. Relative for the primary working directory, absolute for other authorized directories." },
                    "glob": { "type": "string", "description": "Path/name filter, used in both modes. Brace alternatives verbatim: \"**/{AGENTS.md,CONSTITUTION.md}\", with no characters added around the braces." },
                    "context_lines": { "type": "integer", "description": "Surrounding lines to include with each content match, replacing -A/-B/-C. Default 0; content mode only." },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "default": "content", "description": "content: matching lines with file and line number (default). files_with_matches: paths of files that match. count: per-file match counts. Content mode only." }
                },
                "required": ["path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let pattern_str = params["pattern"].as_str().unwrap_or("").trim().to_string();
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
        let glob_str = params["glob"]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let glob_set = Self::build_glob_set(glob_str, pattern_str.is_empty())?;
        let context_lines = Self::parse_context_lines(&params)?;
        let path = resolve_tool_path(search_path, self.path_guard.as_ref());
        validate_search_path(&path, self.path_guard.as_ref())?;
        let display_path = display_path_for_tool_output(&path, self.path_guard.as_ref());

        if !path.exists() {
            return Err(ToolError::IoError(format!(
                "Search path not found: {}. Verify the path with list_dir before searching.",
                display_path
            )));
        }

        // An omitted or blank `pattern` selects path-only search. This is how the tool
        // that was previously exposed separately as `glob` is expressed now.
        if pattern_str.is_empty() {
            return self.search_paths_only(&path, &display_path, glob_str, glob_set.as_ref());
        }

        let re = Regex::new(&pattern_str)
            .map_err(|e| ToolError::InvalidParams(format!("Invalid regex: {}", e)))?;
        let mut matches = vec![];
        let mut match_count = 0_usize;
        let mut truncated = false;

        if path.is_file() {
            if is_search_path_allowed(&path, false, self.path_guard.as_ref())?
                && Self::matches_glob(&path, path.parent(), glob_set.as_ref())
                && Self::is_searchable_text_file(&path)
            {
                truncated = Self::search_in_file(
                    &path,
                    &re,
                    output_mode,
                    context_lines,
                    &mut matches,
                    &mut match_count,
                    Self::MAX_MATCHES,
                    self.path_guard.as_ref(),
                )?;
            }
        } else if path.is_dir() {
            // Let the ignore walker merge .gitignore and .csignore so excluded
            // directories are pruned before their contents are visited.
            let walker = configure_search_walker(&path);

            for result in walker {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    if !is_search_path_allowed(entry.path(), false, self.path_guard.as_ref())? {
                        continue;
                    }
                    if !Self::matches_glob(entry.path(), Some(&path), glob_set.as_ref()) {
                        continue;
                    }
                    if !Self::is_searchable_text_file(entry.path()) {
                        continue;
                    }
                    let limit_reached = Self::search_in_file(
                        entry.path(),
                        &re,
                        output_mode,
                        context_lines,
                        &mut matches,
                        &mut match_count,
                        Self::MAX_MATCHES,
                        self.path_guard.as_ref(),
                    )?;
                    if limit_reached {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        let result_path = result_path_for_tool_output(&path);
        if matches.is_empty() {
            Ok(ToolCallResult::success(
                Some("[No matches found]".into()),
                Some(json!({
                    "pattern": pattern_str,
                    "path": result_path,
                    "display_path": display_path,
                    "output_mode": output_mode,
                    "context_lines": context_lines,
                    "count": 0,
                    "match_count": 0,
                    "truncated": false,
                    "max_matches": Self::MAX_MATCHES,
                    "llm_content": "[No matches found]"
                })),
            ))
        } else {
            let mut llm_content =
                preview_grep_lines_for_llm(&matches, output_mode).unwrap_or_default();
            if truncated {
                llm_content.push_str(&format!(
                    "\n<SYSTEM_REMINDER>Search stopped after {} entries. Narrow path, glob, or pattern before treating the result as exhaustive.</SYSTEM_REMINDER>",
                    Self::MAX_MATCHES
                ));
            }
            Ok(ToolCallResult::success(
                Some(matches.join("\n")),
                Some(json!({
                    "pattern": pattern_str,
                    "path": result_path,
                    "display_path": display_path,
                    "output_mode": output_mode,
                    "context_lines": context_lines,
                    "count": matches.len(),
                    "match_count": match_count,
                    "truncated": truncated,
                    "max_matches": Self::MAX_MATCHES,
                    "llm_content": llm_content
                })),
            ))
        }
    }
}

impl Grep {
    const MAX_MATCHES: usize = 500;
    const MAX_PATH_MATCHES: usize = 1000;
    const MAX_CONTENT_MATCH_CHARS: usize = 500;
    const TEXT_SNIFF_BYTES: usize = 8192;

    fn parse_context_lines(params: &Value) -> Result<usize, ToolError> {
        match params.get("context_lines") {
            None | Some(Value::Null) => Ok(0),
            Some(Value::Number(value)) => value
                .as_u64()
                .filter(|value| *value <= 1000)
                .map(|value| value as usize)
                .ok_or_else(|| {
                    ToolError::InvalidParams(
                        "context_lines must be an integer from 0 to 1000".to_string(),
                    )
                }),
            _ => Err(ToolError::InvalidParams(
                "context_lines must be an integer from 0 to 1000".to_string(),
            )),
        }
    }

    fn search_paths_only(
        &self,
        path: &Path,
        display_path: &str,
        glob: Option<&str>,
        glob_set: Option<&GlobSet>,
    ) -> NativeToolResult {
        let mut paths = Vec::new();
        let mut truncated = false;
        let root = if path.is_file() {
            path.parent()
        } else {
            Some(path)
        };
        let entries: Box<dyn Iterator<Item = PathBuf>> = if path.is_file() {
            Box::new(std::iter::once(path.to_path_buf()))
        } else {
            Box::new(
                configure_search_walker(path)
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        entry
                            .file_type()
                            .is_some_and(|kind| kind.is_file())
                            .then(|| entry.into_path())
                    }),
            )
        };

        for entry_path in entries {
            if !is_search_path_allowed(&entry_path, false, self.path_guard.as_ref())? {
                continue;
            }
            if !Self::matches_glob(&entry_path, root, glob_set) {
                continue;
            }
            paths.push(display_path_for_tool_output(
                &entry_path,
                self.path_guard.as_ref(),
            ));
            if paths.len() >= Self::MAX_PATH_MATCHES {
                truncated = true;
                break;
            }
        }

        paths.sort();
        let content = if paths.is_empty() {
            "[No matches found]".to_string()
        } else {
            paths.join("\n")
        };
        let mut llm_content = if paths.is_empty() {
            content.clone()
        } else {
            preview_path_lines_for_llm(&paths).unwrap_or_default()
        };
        if truncated {
            llm_content.push_str("\n<SYSTEM_REMINDER>Search stopped after 1000 paths. Narrow path or glob before treating the result as exhaustive.</SYSTEM_REMINDER>");
        }
        Ok(ToolCallResult::success(
            Some(content),
            Some(json!({
                "path": result_path_for_tool_output(path),
                "display_path": display_path,
                "glob": glob,
                "output_mode": "files_with_matches",
                "count": paths.len(),
                "truncated": truncated,
                "max_matches": Self::MAX_PATH_MATCHES,
                "llm_content": llm_content,
            })),
        ))
    }
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

    fn build_glob_set(glob: Option<&str>, path_only: bool) -> Result<Option<GlobSet>, ToolError> {
        let Some(glob) = glob.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };

        let mut builder = GlobSetBuilder::new();
        let pattern = if path_only {
            globset::GlobBuilder::new(glob)
                .case_insensitive(true)
                .literal_separator(true)
                .build()
        } else {
            GlobPattern::new(glob)
        }
        .map_err(|e| ToolError::InvalidParams(format!("Invalid glob: {}", e)))?;
        builder.add(pattern);
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
        context_lines: usize,
        matches: &mut Vec<String>,
        match_count: &mut usize,
        max: usize,
        path_guard: Option<&Arc<RwLock<PathGuard>>>,
    ) -> Result<bool, ToolError> {
        let file = fs::File::open(path).map_err(|e| ToolError::IoError(e.to_string()))?;
        let reader = BufReader::new(file);
        let mut count = 0;
        let mut pending: VecDeque<(usize, String, bool)> = VecDeque::new();
        let mut last_match_line = 0_usize;
        let display_path = display_path_for_tool_output(path, path_guard);
        for (i, line) in reader.lines().enumerate() {
            if let Ok(content) = line {
                let line_number = i + 1;
                let found = re.find(&content);
                if found.is_some() {
                    count += 1;
                    *match_count += 1;
                    if mode == "files_with_matches" {
                        matches.push(display_path);
                        return Ok(matches.len() >= max);
                    }
                    if mode == "content" {
                        last_match_line = line_number;
                        for (previous_number, _, included) in pending.iter_mut() {
                            if line_number - *previous_number <= context_lines {
                                *included = true;
                            }
                        }
                    }
                }
                if mode != "content" {
                    continue;
                }
                let included = found.is_some()
                    || (last_match_line > 0 && line_number - last_match_line <= context_lines);
                let display_content = Self::format_content_match(
                    &content,
                    found.map_or(0, |matched| matched.start()),
                );
                pending.push_back((line_number, display_content, included));
                if pending.len() > context_lines {
                    if let Some((number, text, included)) = pending.pop_front() {
                        if included {
                            matches.push(format!("{display_path}:{number}:{text}"));
                            if matches.len() >= max {
                                return Ok(true);
                            }
                        }
                    }
                }
            }
        }
        while let Some((number, text, included)) = pending.pop_front() {
            if included {
                matches.push(format!("{display_path}:{number}:{text}"));
                if matches.len() >= max {
                    return Ok(true);
                }
            }
        }
        if mode == "count" && count > 0 {
            matches.push(format!("{display_path}: {count}"));
        }
        Ok(matches.len() >= max)
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
    use tempfile::{tempdir_in, NamedTempFile};

    /// Creates a fixture directory inside ChatSpeed's stable temp root.
    ///
    /// Tool paths resolve through `resolve_ai_temp_path`, which maps the model-facing `/tmp/...`
    /// namespace into the stable root. A random `/tmp` tempdir would be remapped away from its
    /// real location, so fixtures live inside the stable root where the created path and the
    /// resolved path stay identical.
    fn stable_temp_dir() -> tempfile::TempDir {
        tempdir_in(crate::libs::ai_temp::ai_temp_physical_root_unchecked())
            .expect("failed to create a stable temp directory")
    }

    /// Creates a fixture file inside ChatSpeed's stable temp root. See `stable_temp_dir`.
    fn stable_temp_file() -> NamedTempFile {
        NamedTempFile::new_in(crate::libs::ai_temp::ai_temp_physical_root_unchecked())
            .expect("failed to create a stable temp file")
    }

    fn make_relative_test_dir() -> (tempfile::TempDir, PathBuf) {
        let root = primary_directory(None);
        let temp_dir = tempdir_in(&root).unwrap();
        let relative = temp_dir.path().strip_prefix(&root).unwrap().to_path_buf();
        (temp_dir, relative)
    }

    #[tokio::test]
    async fn test_glob_basic() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

        // Create test files
        fs::write(temp_dir.path().join("test1.txt"), "").unwrap();
        fs::write(temp_dir.path().join("test2.txt"), "").unwrap();
        fs::write(temp_dir.path().join("other.md"), "").unwrap();

        let params = json!({
            "glob": "*.txt",
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
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

        // Create nested structure
        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("nested.txt"), "").unwrap();

        let params = json!({
            "glob": "**/*.txt",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let files: Vec<&str> = content.lines().collect();

        assert!(files.len() >= 1);
        assert!(files.iter().any(|f| f.contains("nested.txt")));
    }

    #[tokio::test]
    async fn search_walker_merges_csignore_without_reopening_other_gitignored_directories() {
        let temp_dir = stable_temp_dir();
        let root = temp_dir.path().canonicalize().unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir(root.join("dev_data")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(
            root.join(".gitignore"),
            "dev_data/\ntarget/\nnode_modules/\n",
        )
        .unwrap();
        fs::write(
            root.join(CHATSPEED_IGNORE_FILE),
            "!dev_data/\n!dev_data/**\n",
        )
        .unwrap();
        fs::write(root.join("dev_data/fixture.txt"), "needle").unwrap();
        fs::write(root.join("target/ignored.txt"), "needle").unwrap();
        fs::write(root.join("node_modules/ignored.txt"), "needle").unwrap();

        let visited_paths = configure_search_walker(&root)
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_path_buf())
            .collect::<Vec<_>>();
        assert!(visited_paths.contains(&root.join("dev_data")));
        assert!(visited_paths.contains(&root.join("dev_data/fixture.txt")));
        assert!(!visited_paths.contains(&root.join("target")));
        assert!(!visited_paths.contains(&root.join("target/ignored.txt")));
        assert!(!visited_paths.contains(&root.join("node_modules")));
        assert!(!visited_paths.contains(&root.join("node_modules/ignored.txt")));

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![root.clone()],
            vec![],
            vec![],
        )));

        let glob_result = Grep::new(Some(guard.clone()))
            .call(json!({
                "glob": "**/*.txt",
                "path": root.to_string_lossy()
            }))
            .await
            .unwrap()
            .content
            .unwrap();
        assert!(glob_result.contains("dev_data/fixture.txt"));
        assert!(!glob_result.contains("target/ignored.txt"));
        assert!(!glob_result.contains("node_modules/ignored.txt"));

        let grep_result = Grep::new(Some(guard))
            .call(json!({
                "pattern": "needle",
                "path": root.to_string_lossy(),
                "output_mode": "files_with_matches"
            }))
            .await
            .unwrap()
            .content
            .unwrap();
        assert!(grep_result.contains("dev_data/fixture.txt"));
        assert!(!grep_result.contains("target/ignored.txt"));
        assert!(!grep_result.contains("node_modules/ignored.txt"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

        let params = json!({
            "glob": "*.nonexistent",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert_eq!(content, "[No matches found]");
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

        // Invalid glob pattern
        let params = json!({
            "glob": "**/*[invalid",
            "path": temp_dir.path().to_string_lossy()
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn test_glob_max_results() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

        // Create many files
        for i in 0..1010 {
            fs::write(temp_dir.path().join(format!("file{}.txt", i)), "").unwrap();
        }

        let params = json!({
            "glob": "*.txt",
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
        let tool = Grep::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);
        fs::create_dir(absolute_root.join("src")).unwrap();
        fs::write(absolute_root.join("src").join("lib.rs"), "").unwrap();

        let result = tool
            .call(json!({
                "glob": "**/*.rs",
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
        let tool = Grep::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);

        for i in 0..220 {
            fs::write(absolute_root.join(format!("file-{:03}.rs", i)), "").unwrap();
        }

        let result = tool
            .call(json!({
                "glob": "**/*.rs",
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

    #[test]
    fn merged_search_schema_preserves_tested_tool_guidance() {
        let tool = Grep::default();
        let declaration = tool.tool_calling_spec();
        let description = tool.description();
        let properties = &declaration.input_schema["properties"];

        // Model tests needed this sentence to choose path search over list_dir for extensions.
        assert!(description.contains("discover files by extension, directory, or file name"));
        // Brace syntax was often corrupted when the verbatim instruction was removed.
        assert!(description.contains("brace alternatives are written verbatim"));
        assert!(properties["glob"]["description"]
            .as_str()
            .unwrap()
            .contains("with no characters added around the braces"));
        // The explicit parameter replaced hallucinated shell -A/-B/-C arguments in model tests.
        assert_eq!(properties["context_lines"]["type"], "integer");
        assert_eq!(declaration.input_schema["required"], json!(["path"]));
    }

    #[tokio::test]
    async fn path_only_search_matches_binary_files_without_reading_contents() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();
        fs::write(temp_dir.path().join("payload.bin"), b"needle\0binary").unwrap();
        fs::write(temp_dir.path().join("other.txt"), "needle").unwrap();

        let result = tool
            .call(json!({"path": temp_dir.path(), "glob": "*.bin"}))
            .await
            .unwrap();
        let content = result.content.unwrap();
        assert!(content.ends_with("payload.bin"));
        assert!(!content.contains("needle"));
        assert_eq!(result.structured_content.unwrap()["count"], 1);
    }

    #[tokio::test]
    async fn path_only_search_keeps_brace_alternatives_verbatim() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();
        fs::write(temp_dir.path().join("AGENTS.md"), "rules").unwrap();
        fs::write(temp_dir.path().join("CONSTITUTION.md"), "rules").unwrap();
        fs::write(temp_dir.path().join("other.md"), "rules").unwrap();

        let result = tool
            .call(json!({
                "path": temp_dir.path(),
                "glob": "**/{AGENTS.md,CONSTITUTION.md}"
            }))
            .await
            .unwrap();
        let content = result.content.unwrap();
        assert!(content.contains("AGENTS.md"));
        assert!(content.contains("CONSTITUTION.md"));
        assert!(!content.contains("other.md"));
    }

    #[tokio::test]
    async fn content_search_includes_surrounding_lines_once() {
        let tool = Grep::default();
        let temp_file = stable_temp_file();
        fs::write(
            temp_file.path(),
            "before\nmatch one\nbetween\nmatch two\nafter\noutside",
        )
        .unwrap();

        let result = tool
            .call(json!({
                "path": temp_file.path(),
                "pattern": "match",
                "context_lines": 1
            }))
            .await
            .unwrap();
        let content = result.content.unwrap();
        let lines = content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains(":1:before"));
        assert!(lines[4].contains(":5:after"));
        assert!(!content.contains(":6:outside"));
        assert_eq!(result.structured_content.unwrap()["match_count"], 2);
    }

    #[tokio::test]
    async fn content_pattern_and_path_glob_remain_distinct() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();
        fs::write(temp_dir.path().join("source.rs"), "target symbol").unwrap();
        fs::write(temp_dir.path().join("notes.md"), "target symbol").unwrap();

        let result = tool
            .call(json!({
                "path": temp_dir.path(),
                "pattern": "target\\s+symbol",
                "glob": "*.rs"
            }))
            .await
            .unwrap();
        let content = result.content.unwrap();
        assert!(content.contains("source.rs"));
        assert!(!content.contains("notes.md"));
        assert_eq!(result.structured_content.unwrap()["match_count"], 1);
    }

    #[tokio::test]
    async fn test_grep_basic() {
        let tool = Grep::default();
        let temp_file = stable_temp_file();
        let path = temp_file.path().to_string_lossy().to_string();
        let display_path = display_path_for_tool_output(temp_file.path(), None);

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
            assert!(match_item.contains(&display_path));
            assert!(match_item.contains(":1:") || match_item.contains(":2:"));
        }
    }

    #[tokio::test]
    async fn test_grep_files_with_matches() {
        let tool = Grep::default();
        let temp_file = stable_temp_file();
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
        let display_path = display_path_for_tool_output(temp_file.path(), None);

        // Should return file path once even with multiple matches
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], display_path);
    }

    #[tokio::test]
    async fn test_grep_defaults_to_content_mode() {
        let tool = Grep::default();
        let temp_file = stable_temp_file();
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
        let temp_file = stable_temp_file();
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
        let temp_file = stable_temp_file();
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
        let display_path = display_path_for_tool_output(temp_file.path(), None);

        assert_eq!(matches.len(), 1);
        assert!(matches[0].contains(&display_path));
        assert!(matches[0].contains("2"));
    }

    #[tokio::test]
    async fn test_grep_directory_search() {
        let tool = Grep::default();
        let temp_dir = stable_temp_dir();

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
        let temp_file = stable_temp_file();
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
        let temp_dir = stable_temp_dir();

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
        let temp_file = stable_temp_file();
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
        let temp_file = stable_temp_file();
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
        let temp_file = stable_temp_file();
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
        let structured = result.structured_content.unwrap();

        // Should be limited to MAX_MATCHES (500)
        assert_eq!(matches.len(), 500);
        assert_eq!(structured["count"].as_u64(), Some(500));
        assert_eq!(structured["truncated"].as_bool(), Some(true));
        assert_eq!(structured["max_matches"].as_u64(), Some(500));
        assert!(structured["llm_content"]
            .as_str()
            .is_some_and(|content| content.contains("Search stopped after 500 entries")));
    }

    #[tokio::test]
    async fn test_grep_case_sensitive() {
        let tool = Grep::default();
        let temp_file = stable_temp_file();
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
        let temp_dir = stable_temp_dir();
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
        let temp_dir = stable_temp_dir();
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
        let temp_file = stable_temp_file();
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
