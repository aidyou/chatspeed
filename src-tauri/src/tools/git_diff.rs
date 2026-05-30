use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::llm_output::preview_path_lines_for_llm;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::security::PathGuard;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

fn primary_directory(path_guard: Option<&Arc<RwLock<PathGuard>>>) -> Result<PathBuf, ToolError> {
    path_guard
        .and_then(|guard| guard.read().ok())
        .and_then(|guard| guard.get_primary_root().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| ToolError::ExecutionFailed("No workspace root is configured".to_string()))
}

fn workspace_directories(
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<Vec<PathBuf>, ToolError> {
    if let Some(guard) = path_guard.and_then(|guard| guard.read().ok()) {
        let roots = guard.workspace_roots();
        if !roots.is_empty() {
            return Ok(roots);
        }
    }

    std::env::current_dir()
        .map(|dir| vec![dir])
        .map_err(|_| ToolError::ExecutionFailed("No workspace root is configured".to_string()))
}

fn display_path_for_tool_output(
    path: &Path,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> String {
    let Some(primary_dir) = path_guard
        .and_then(|guard| guard.read().ok())
        .and_then(|guard| guard.get_primary_root().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
    else {
        return path.to_string_lossy().to_string();
    };

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

fn find_workspace_root(path: &Path, workspace_roots: &[PathBuf]) -> Option<PathBuf> {
    workspace_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn validate_path(
    target: &Path,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<PathBuf, ToolError> {
    path_guard
        .and_then(|guard| guard.read().ok())
        .ok_or_else(|| ToolError::ExecutionFailed("Path guard is unavailable".to_string()))?
        .validate(target, false, false, false)
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))
}

fn resolve_requested_paths(
    requested_paths: &[String],
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<BTreeMap<PathBuf, BTreeSet<String>>, ToolError> {
    let workspace_roots = workspace_directories(path_guard)?;
    let primary_dir = primary_directory(path_guard)?;
    let mut grouped = BTreeMap::new();

    if requested_paths.is_empty() {
        for root in workspace_roots {
            grouped
                .entry(root)
                .or_insert_with(BTreeSet::new)
                .insert(".".to_string());
        }
        return Ok(grouped);
    }

    for path_str in requested_paths {
        let path = Path::new(path_str);
        if path.is_absolute() {
            let validated = validate_path(path, path_guard)?;
            let workspace_root =
                find_workspace_root(&validated, &workspace_roots).ok_or_else(|| {
                    ToolError::ExecutionFailed(format!(
                        "Path '{}' is outside the authorized workspace roots",
                        path_str
                    ))
                })?;
            let relative = validated
                .strip_prefix(&workspace_root)
                .map_err(|_| {
                    ToolError::ExecutionFailed(format!(
                        "Path '{}' is outside the authorized workspace roots",
                        path_str
                    ))
                })?
                .to_string_lossy()
                .to_string();
            grouped
                .entry(workspace_root)
                .or_insert_with(BTreeSet::new)
                .insert(if relative.is_empty() {
                    ".".to_string()
                } else {
                    relative
                });
            continue;
        }

        let mut matched_existing_root = false;
        for workspace_root in &workspace_roots {
            let candidate = workspace_root.join(path);
            if !candidate.exists() {
                continue;
            }
            let validated = validate_path(&candidate, path_guard)?;
            let relative = validated
                .strip_prefix(workspace_root)
                .map_err(|_| {
                    ToolError::ExecutionFailed(format!(
                        "Path '{}' is outside the authorized workspace roots",
                        path_str
                    ))
                })?
                .to_string_lossy()
                .to_string();
            grouped
                .entry(workspace_root.clone())
                .or_insert_with(BTreeSet::new)
                .insert(if relative.is_empty() {
                    ".".to_string()
                } else {
                    relative
                });
            matched_existing_root = true;
        }

        if matched_existing_root {
            continue;
        }

        let fallback = primary_dir.join(path);
        let validated = validate_path(&fallback, path_guard)?;
        let workspace_root =
            find_workspace_root(&validated, &workspace_roots).ok_or_else(|| {
                ToolError::ExecutionFailed(format!(
                    "Path '{}' is outside the authorized workspace roots",
                    path_str
                ))
            })?;
        let relative = validated
            .strip_prefix(&workspace_root)
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "Path '{}' is outside the authorized workspace roots",
                    path_str
                ))
            })?
            .to_string_lossy()
            .to_string();
        grouped
            .entry(workspace_root)
            .or_insert_with(BTreeSet::new)
            .insert(if relative.is_empty() {
                ".".to_string()
            } else {
                relative
            });
    }

    Ok(grouped)
}

fn run_git_command(
    workspace_root: &Path,
    args: &[String],
) -> Result<(i32, String, String), ToolError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to run git: {}", e)))?;

    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

fn ensure_git_repository(workspace_root: &Path) -> Result<(), ToolError> {
    let (code, stdout, stderr) = run_git_command(
        workspace_root,
        &["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
    )?;
    if code != 0 || stdout.trim() != "true" {
        let detail = stderr.trim();
        return Err(ToolError::ExecutionFailed(if detail.is_empty() {
            format!(
                "Primary workspace '{}' is not inside a Git repository",
                workspace_root.to_string_lossy()
            )
        } else {
            format!("Git repository check failed: {}", detail)
        }));
    }
    Ok(())
}

fn git_current_branch(workspace_root: &Path) -> Result<String, ToolError> {
    let (code, stdout, _) = run_git_command(
        workspace_root,
        &["branch".to_string(), "--show-current".to_string()],
    )?;
    if code == 0 {
        let branch = stdout.trim();
        if !branch.is_empty() {
            return Ok(branch.to_string());
        }
    }

    let (code, stdout, stderr) = run_git_command(
        workspace_root,
        &[
            "symbolic-ref".to_string(),
            "--short".to_string(),
            "HEAD".to_string(),
        ],
    )?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "Failed to determine current branch: {}",
            stderr.trim()
        )));
    }

    Ok(stdout.trim().to_string())
}

fn git_branches(workspace_root: &Path) -> Result<Vec<String>, ToolError> {
    let (code, stdout, stderr) = run_git_command(
        workspace_root,
        &[
            "branch".to_string(),
            "--format=%(refname:short)".to_string(),
        ],
    )?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "Failed to list branches: {}",
            stderr.trim()
        )));
    }

    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn truncate_patch(patch: &str, max_lines: usize) -> String {
    let mut lines = patch.lines();
    let collected = lines.by_ref().take(max_lines).collect::<Vec<_>>();
    let mut output = collected.join("\n");
    if lines.next().is_some() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str("[patch truncated]");
    }
    output
}

fn build_untracked_patch(
    abs_path: &Path,
    relative_path: &str,
    max_lines: usize,
) -> (String, usize) {
    let content = fs::read_to_string(abs_path).unwrap_or_default();
    let mut patch = String::new();
    patch.push_str(&format!(
        "diff --git a/{} b/{}\n",
        relative_path, relative_path
    ));
    patch.push_str("new file mode 100644\n");
    patch.push_str("--- /dev/null\n");
    patch.push_str(&format!("+++ b/{}\n", relative_path));
    let line_count = count_lines(&content);
    patch.push_str(&format!("@@ -0,0 +1,{} @@\n", line_count));
    for line in content.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    (truncate_patch(&patch, max_lines), line_count)
}

#[derive(Clone, Default)]
pub struct GitDiff {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl GitDiff {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for GitDiff {
    fn name(&self) -> &str {
        crate::tools::TOOL_GIT_DIFF
    }

    fn description(&self) -> &str {
        "Read-only Git diff for the authorized workspace roots.\n\n\
        Usage:\n\
        - Shows tracked modifications and staged changes without using bash.\n\
        - Restricted to authorized workspace roots and paths inside them.\n\
        - When no path filter is provided, scans every authorized workspace root that is a Git repository.\n\
        - Supports optional path filters to narrow the diff to specific files or directories.\n\
        - Includes synthetic patches for untracked new files so reviewers can inspect created files.\n\
        - Returns current branch and visible local branches so agents can choose a `base` revision safely.\n\
        - Prefer this tool over shelling out to `git diff` from child agents."
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
                    "base": { "type": "string", "description": "Git revision to diff against. Defaults to HEAD. The tool returns current branch and visible local branches to help choose this value." },
                    "path": { "type": "string", "description": "Optional single file or directory inside the authorized workspace roots to diff. Absolute paths may target any authorized workspace root." },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of files or directories inside the authorized workspace roots to diff. Absolute paths may target any authorized workspace root."
                    },
                    "staged": { "type": "boolean", "default": false, "description": "If true, diff staged changes against the base revision." },
                    "max_files": { "type": "integer", "default": 20, "description": "Maximum number of changed files to return." },
                    "max_lines_per_file": { "type": "integer", "default": 160, "description": "Maximum number of patch lines to return per file." }
                }
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let staged = params["staged"].as_bool().unwrap_or(false);
        let base = params["base"].as_str().unwrap_or("HEAD").trim();
        let max_files = params["max_files"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
        let max_lines_per_file = params["max_lines_per_file"]
            .as_u64()
            .unwrap_or(160)
            .clamp(20, 1000) as usize;

        let mut requested_paths = Vec::new();
        if let Some(path) = params["path"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            requested_paths.push(path.to_string());
        }
        if let Some(paths) = params["paths"].as_array() {
            for path in paths.iter().filter_map(|value| value.as_str()) {
                let path = path.trim();
                if !path.is_empty() {
                    requested_paths.push(path.to_string());
                }
            }
        }
        let requested_by_root =
            resolve_requested_paths(&requested_paths, self.path_guard.as_ref())?;
        let explicit_paths = !requested_paths.is_empty();
        let mut repositories = Vec::new();
        let mut all_files = Vec::new();
        let mut all_patch_sections = Vec::new();
        let mut llm_file_paths = Vec::new();

        for (workspace_root, pathspecs) in requested_by_root {
            match ensure_git_repository(&workspace_root) {
                Ok(()) => {}
                Err(_) if !explicit_paths => continue,
                Err(err) => return Err(err),
            }

            let pathspecs = pathspecs.into_iter().collect::<Vec<_>>();
            let current_branch = git_current_branch(&workspace_root)?;
            let branches = git_branches(&workspace_root)?;
            let mut status_args = vec![
                "status".to_string(),
                "--porcelain".to_string(),
                "--untracked-files=all".to_string(),
                "--".to_string(),
            ];
            status_args.extend(pathspecs.clone());
            let (status_code, status_stdout, status_stderr) =
                run_git_command(&workspace_root, &status_args)?;
            if status_code != 0 {
                return Err(ToolError::ExecutionFailed(format!(
                    "git status failed: {}",
                    status_stderr.trim()
                )));
            }

            let mut repo_files = Vec::new();
            let mut repo_patch_sections = Vec::new();
            for line in status_stdout.lines().take(max_files) {
                if line.len() < 3 {
                    continue;
                }
                let status_code = line[..2].trim().to_string();
                let path_text = line[3..].trim();
                let relative_path = path_text
                    .split(" -> ")
                    .last()
                    .unwrap_or(path_text)
                    .trim()
                    .to_string();
                let abs_path = workspace_root.join(&relative_path);

                if status_code == "??" {
                    let (patch, added) =
                        build_untracked_patch(&abs_path, &relative_path, max_lines_per_file);
                    let file = json!({
                        "workspace_root": display_path_for_tool_output(&workspace_root, self.path_guard.as_ref()),
                        "path": relative_path,
                        "status": "untracked",
                        "added": added,
                        "deleted": 0
                    });
                    repo_files.push(file.clone());
                    all_files.push(file);
                    repo_patch_sections.push(patch);
                    llm_file_paths.push(display_path_for_tool_output(
                        &workspace_root.join(&relative_path),
                        self.path_guard.as_ref(),
                    ));
                    continue;
                }

                let mut diff_args = vec![
                    "diff".to_string(),
                    "--no-ext-diff".to_string(),
                    "--unified=3".to_string(),
                ];
                if staged {
                    diff_args.push("--cached".to_string());
                }
                diff_args.push(base.to_string());
                diff_args.push("--".to_string());
                diff_args.push(relative_path.clone());
                let (diff_code, diff_stdout, diff_stderr) =
                    run_git_command(&workspace_root, &diff_args)?;
                if diff_code != 0 {
                    return Err(ToolError::ExecutionFailed(format!(
                        "git diff failed for '{}': {}",
                        relative_path,
                        diff_stderr.trim()
                    )));
                }

                let mut numstat_args = vec!["diff".to_string(), "--numstat".to_string()];
                if staged {
                    numstat_args.push("--cached".to_string());
                }
                numstat_args.push(base.to_string());
                numstat_args.push("--".to_string());
                numstat_args.push(relative_path.clone());
                let (_, numstat_stdout, _) = run_git_command(&workspace_root, &numstat_args)?;
                let (added, deleted) = numstat_stdout
                    .lines()
                    .next()
                    .and_then(|entry| {
                        let parts = entry.split('\t').collect::<Vec<_>>();
                        if parts.len() >= 2 {
                            Some((
                                parts[0].parse::<usize>().ok(),
                                parts[1].parse::<usize>().ok(),
                            ))
                        } else {
                            None
                        }
                    })
                    .unwrap_or((None, None));

                let file = json!({
                    "workspace_root": display_path_for_tool_output(&workspace_root, self.path_guard.as_ref()),
                    "path": relative_path,
                    "status": status_code,
                    "added": added,
                    "deleted": deleted
                });
                repo_files.push(file.clone());
                all_files.push(file);
                let truncated = truncate_patch(&diff_stdout, max_lines_per_file);
                if !truncated.trim().is_empty() {
                    repo_patch_sections.push(truncated);
                }
                llm_file_paths.push(display_path_for_tool_output(
                    &workspace_root.join(&relative_path),
                    self.path_guard.as_ref(),
                ));
            }

            let repo_patch = repo_patch_sections.join("\n");
            let repo_summary = if repo_files.is_empty() {
                "No diff found".to_string()
            } else {
                format!(
                    "{} changed file(s) against {}{}",
                    repo_files.len(),
                    base,
                    if staged { " (staged)" } else { "" }
                )
            };

            all_patch_sections.extend(repo_patch_sections);
            repositories.push(json!({
                "workspace_root": display_path_for_tool_output(&workspace_root, self.path_guard.as_ref()),
                "current_branch": current_branch,
                "branches": branches,
                "base": base,
                "staged": staged,
                "files": repo_files,
                "patch": repo_patch,
                "summary": repo_summary
            }));
        }

        if repositories.is_empty() || all_files.is_empty() {
            return Ok(ToolCallResult::success(
                Some("[No diff found]".to_string()),
                Some(json!({
                    "base": base,
                    "staged": staged,
                    "repositories": repositories,
                    "files": [],
                    "patch": "",
                    "summary": "No diff found",
                    "llm_content": "[No diff found]"
                })),
            ));
        }

        let patch = all_patch_sections.join("\n");
        let summary = format!(
            "{} changed file(s) across {} repository root(s) against {}{}",
            all_files.len(),
            repositories.len(),
            base,
            if staged { " (staged)" } else { "" }
        );

        Ok(ToolCallResult::success(
            Some(patch.clone()),
            Some(json!({
                "base": base,
                "staged": staged,
                "repositories": repositories,
                "files": all_files,
                "patch": patch,
                "summary": summary,
                "llm_content": preview_path_lines_for_llm(&llm_file_paths)
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::GitDiff;
    use crate::tools::ToolDefinition;
    use crate::workflow::react::security::PathGuard;
    use serde_json::json;
    use std::fs;
    use std::process::Command;
    use std::sync::{Arc, RwLock};
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let status = Command::new("git")
            .arg("init")
            .arg(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "user.email", "test@example.com"])
            .status()
            .expect("git config email");
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "user.name", "Test User"])
            .status()
            .expect("git config name");
        assert!(status.success());
        dir
    }

    #[tokio::test]
    async fn git_diff_returns_tracked_modification() {
        let dir = init_repo();
        let file = dir.path().join("src.txt");
        fs::write(&file, "hello\n").expect("write seed");
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["add", "src.txt"])
            .status()
            .expect("git add")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["commit", "-m", "init"])
            .status()
            .expect("git commit")
            .success());
        fs::write(&file, "hello\nworld\n").expect("write modified");

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![dir.path().to_path_buf()],
            vec![],
            vec![],
        )));
        let tool = GitDiff::new(Some(guard));
        let result = tool
            .call(json!({ "path": "src.txt" }))
            .await
            .expect("git diff");
        let patch = result.content.unwrap_or_default();
        assert!(patch.contains("diff --git"));
        assert!(patch.contains("+world"));
    }

    #[tokio::test]
    async fn git_diff_includes_untracked_file_patch() {
        let dir = init_repo();
        fs::write(dir.path().join("new.txt"), "alpha\nbeta\n").expect("write untracked");

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![dir.path().to_path_buf()],
            vec![],
            vec![],
        )));
        let tool = GitDiff::new(Some(guard));
        let result = tool
            .call(json!({ "path": "new.txt" }))
            .await
            .expect("git diff");
        let patch = result.content.unwrap_or_default();
        assert!(patch.contains("new file mode 100644"));
        assert!(patch.contains("+alpha"));
        assert!(patch.contains("+beta"));
    }

    #[tokio::test]
    async fn git_diff_scans_multiple_workspace_roots_and_returns_branches() {
        let dir_one = init_repo();
        let dir_two = init_repo();

        let file_one = dir_one.path().join("one.txt");
        fs::write(&file_one, "one\n").expect("write file one");
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir_one.path())
            .args(["add", "one.txt"])
            .status()
            .expect("git add one")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir_one.path())
            .args(["commit", "-m", "init one"])
            .status()
            .expect("git commit one")
            .success());
        fs::write(&file_one, "one\nupdated\n").expect("modify file one");

        let file_two = dir_two.path().join("two.txt");
        fs::write(&file_two, "two\n").expect("write file two");
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir_two.path())
            .args(["add", "two.txt"])
            .status()
            .expect("git add two")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(dir_two.path())
            .args(["commit", "-m", "init two"])
            .status()
            .expect("git commit two")
            .success());
        fs::write(&file_two, "two\nupdated\n").expect("modify file two");

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![dir_one.path().to_path_buf(), dir_two.path().to_path_buf()],
            vec![],
            vec![],
        )));
        let tool = GitDiff::new(Some(guard));
        let result = tool.call(json!({})).await.expect("git diff");
        let data = result.structured_content.expect("structured content");
        let repositories = data["repositories"].as_array().expect("repositories");
        assert_eq!(repositories.len(), 2);
        for repository in repositories {
            assert!(repository["current_branch"].as_str().is_some());
            assert!(repository["branches"].as_array().is_some());
        }
        let patch = result.content.unwrap_or_default();
        assert!(patch.contains("one.txt"));
        assert!(patch.contains("two.txt"));
    }
}
