use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::git_diff::{
    display_path_for_tool_output, ensure_git_repository, parse_status_porcelain_z,
    resolve_commit_revision, resolve_requested_paths, run_git_command,
};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::security::PathGuard;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

const DEFAULT_MAX_COUNT: usize = 20;
const MAX_COUNT_LIMIT: usize = 100;
const DEFAULT_MAX_FILES: usize = 20;
const MAX_FILES_LIMIT: usize = 100;
const DEFAULT_MAX_LINES: usize = 160;
const MAX_LINES_LIMIT: usize = 1_000;

#[derive(Clone, Default)]
pub struct GitInspect {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl GitInspect {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

fn read_requested_paths(params: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = params["path"]
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        paths.push(path.to_string());
    }
    if let Some(items) = params["paths"].as_array() {
        paths.extend(
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
        );
    }
    paths
}

fn clamp_param(params: &Value, name: &str, default: usize, maximum: usize) -> usize {
    params[name]
        .as_u64()
        .unwrap_or(default as u64)
        .clamp(1, maximum as u64) as usize
}

fn truncate_lines(content: &str, max_lines: usize) -> String {
    let mut lines = content.lines();
    let mut output = lines
        .by_ref()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    if lines.next().is_some() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str("[output truncated]");
    }
    output
}

fn current_branch(workspace_root: &PathBuf) -> String {
    let args = vec!["branch".to_string(), "--show-current".to_string()];
    if let Ok((0, stdout, _)) = run_git_command(workspace_root, &args) {
        let branch = stdout.trim();
        if !branch.is_empty() {
            return branch.to_string();
        }
    }
    "HEAD (detached or unborn)".to_string()
}

fn local_branches(workspace_root: &PathBuf) -> Result<Vec<String>, ToolError> {
    let args = vec![
        "for-each-ref".to_string(),
        "--format=%(refname:short)".to_string(),
        "refs/heads".to_string(),
    ];
    let (code, stdout, stderr) = run_git_command(workspace_root, &args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git branch listing failed: {}",
            stderr.trim()
        )));
    }
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .take(MAX_COUNT_LIMIT)
        .map(str::to_string)
        .collect())
}

fn status_for_root(
    workspace_root: &PathBuf,
    pathspecs: Vec<String>,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
    max_files: usize,
) -> Result<Value, ToolError> {
    let mut args = vec![
        "status".to_string(),
        "--porcelain=v1".to_string(),
        "-z".to_string(),
        "--branch".to_string(),
        "--untracked-files=all".to_string(),
        "--".to_string(),
    ];
    args.extend(pathspecs);
    let (code, stdout, stderr) = run_git_command(workspace_root, &args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git status failed: {}",
            stderr.trim()
        )));
    }

    let mut records = stdout.split('\0');
    let branch_header = records.next().unwrap_or_default();
    let mut branch = current_branch(workspace_root);
    let branches = local_branches(workspace_root)?;
    let mut upstream = None;
    let mut ahead = 0_u64;
    let mut behind = 0_u64;
    if let Some(head) = branch_header.strip_prefix("## ") {
        let (name, tracking) = head.split_once("...").unwrap_or((head, ""));
        if !name.trim().is_empty() {
            branch = name.trim().to_string();
        }
        if let Some((remote, counts)) = tracking.split_once(' ') {
            upstream = Some(remote.to_string());
            if let Some(counts) = counts
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                for count in counts.split(", ") {
                    if let Some(value) = count.strip_prefix("ahead ") {
                        ahead = value.parse().unwrap_or(0);
                    }
                    if let Some(value) = count.strip_prefix("behind ") {
                        behind = value.parse().unwrap_or(0);
                    }
                }
            }
        }
    }
    let status_output = records.collect::<Vec<_>>().join("\0");
    let files = parse_status_porcelain_z(&status_output)?
        .into_iter()
        .take(max_files)
        .map(|(status, path)| json!({ "path": path, "status": status }))
        .collect::<Vec<_>>();

    Ok(json!({
        "workspace_root": display_path_for_tool_output(workspace_root, path_guard),
        "branch": branch,
        "branches": branches,
        "upstream": upstream,
        "ahead": ahead,
        "behind": behind,
        "files": files,
    }))
}

fn parse_name_status_z(output: &str) -> Result<Vec<(String, String)>, ToolError> {
    let mut records = output.split('\0');
    let mut entries = Vec::new();
    while let Some(status) = records.next() {
        if status.is_empty() {
            continue;
        }
        let first_path = records.next().ok_or_else(|| {
            ToolError::ExecutionFailed("git show returned incomplete name-status data".to_string())
        })?;
        let path = if status.contains('R') || status.contains('C') {
            records.next().ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "git show returned incomplete rename or copy data".to_string(),
                )
            })?
        } else {
            first_path
        };
        entries.push((status.to_string(), path.to_string()));
    }
    Ok(entries)
}

fn log_for_root(
    workspace_root: &PathBuf,
    pathspecs: Vec<String>,
    revision: &str,
    max_count: usize,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<Value, ToolError> {
    let revision = resolve_commit_revision(workspace_root, revision)?;
    let mut args = vec![
        "log".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        format!("--max-count={max_count}"),
        "--format=%H%x1f%P%x1f%an%x1f%aI%x1f%s%x1e".to_string(),
        revision.clone(),
        "--".to_string(),
    ];
    args.extend(pathspecs);
    let (code, stdout, stderr) = run_git_command(workspace_root, &args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git log failed: {}",
            stderr.trim()
        )));
    }
    let commits = stdout
        .split('\x1e')
        .filter_map(|record| {
            let fields = record.trim().split('\x1f').collect::<Vec<_>>();
            (fields.len() == 5).then(|| {
                json!({
                    "id": fields[0],
                    "parents": fields[1].split_whitespace().collect::<Vec<_>>(),
                    "author": fields[2],
                    "authored_at": fields[3],
                    "subject": fields[4],
                })
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "workspace_root": display_path_for_tool_output(workspace_root, path_guard),
        "revision": revision,
        "commits": commits,
    }))
}

fn blame_for_root(
    workspace_root: &PathBuf,
    pathspecs: Vec<String>,
    line_start: usize,
    line_end: usize,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<Value, ToolError> {
    if pathspecs.len() != 1 || pathspecs[0] == "." {
        return Err(ToolError::ExecutionFailed(
            "blame requires exactly one authorized file path".to_string(),
        ));
    }
    let path = workspace_root.join(&pathspecs[0]);
    if !path.is_file() {
        return Err(ToolError::ExecutionFailed(
            "blame requires a file path, not a directory".to_string(),
        ));
    }
    let args = vec![
        "blame".to_string(),
        "--line-porcelain".to_string(),
        format!("-L{line_start},{line_end}"),
        "--".to_string(),
        pathspecs[0].clone(),
    ];
    let (code, stdout, stderr) = run_git_command(workspace_root, &args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git blame failed: {}",
            stderr.trim()
        )));
    }

    let mut lines = Vec::new();
    let mut commit = String::new();
    let mut source = None;
    for line in stdout.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            lines.push(json!({
                "commit": commit.clone(),
                "source": content,
            }));
            source = None;
            continue;
        }
        if let Some((candidate, _)) = line.split_once(' ') {
            if candidate.len() == 40
                && candidate
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                commit = candidate.to_string();
                source = Some(candidate);
            }
        }
    }
    if source.is_some() {
        return Err(ToolError::ExecutionFailed(
            "git blame returned incomplete line data".to_string(),
        ));
    }

    Ok(json!({
        "workspace_root": display_path_for_tool_output(workspace_root, path_guard),
        "path": pathspecs[0],
        "line_start": line_start,
        "line_end": line_end,
        "lines": lines,
    }))
}

fn merge_base_for_root(
    workspace_root: &PathBuf,
    revision: &str,
    other_revision: &str,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<Value, ToolError> {
    let revision = resolve_commit_revision(workspace_root, revision)?;
    let other_revision = resolve_commit_revision(workspace_root, other_revision)?;
    let args = vec![
        "merge-base".to_string(),
        revision.clone(),
        other_revision.clone(),
    ];
    let (code, stdout, stderr) = run_git_command(workspace_root, &args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git merge-base failed: {}",
            stderr.trim()
        )));
    }
    let merge_base = stdout.trim();
    if merge_base.len() != 40
        || !merge_base
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ToolError::ExecutionFailed(
            "git merge-base did not return a commit".to_string(),
        ));
    }
    Ok(json!({
        "workspace_root": display_path_for_tool_output(workspace_root, path_guard),
        "revision": revision,
        "other_revision": other_revision,
        "merge_base": merge_base,
    }))
}

fn show_for_root(
    workspace_root: &PathBuf,
    pathspecs: Vec<String>,
    revision: &str,
    max_files: usize,
    max_lines: usize,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> Result<Value, ToolError> {
    let revision = resolve_commit_revision(workspace_root, revision)?;
    let metadata_args = vec![
        "show".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        "--no-patch".to_string(),
        "--format=%H%x1f%P%x1f%an%x1f%aI%x1f%s".to_string(),
        revision.clone(),
    ];
    let (code, metadata, stderr) = run_git_command(workspace_root, &metadata_args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git show failed: {}",
            stderr.trim()
        )));
    }
    let fields = metadata.trim().split('\x1f').collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(ToolError::ExecutionFailed(
            "git show returned malformed commit metadata".to_string(),
        ));
    }

    let mut name_status_args = vec![
        "show".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        "--format=".to_string(),
        "--name-status".to_string(),
        "-z".to_string(),
        revision.clone(),
        "--".to_string(),
    ];
    name_status_args.extend(pathspecs.clone());
    let (code, name_status, stderr) = run_git_command(workspace_root, &name_status_args)?;
    if code != 0 {
        return Err(ToolError::ExecutionFailed(format!(
            "git show name-status failed: {}",
            stderr.trim()
        )));
    }
    let selected_files = parse_name_status_z(&name_status)?
        .into_iter()
        .take(max_files)
        .collect::<Vec<_>>();
    let files = selected_files
        .iter()
        .map(|(status, path)| json!({ "path": path, "status": status }))
        .collect::<Vec<_>>();

    let patch = if selected_files.is_empty() {
        String::new()
    } else {
        let mut patch_args = vec![
            "show".to_string(),
            "--no-ext-diff".to_string(),
            "--no-textconv".to_string(),
            "--format=".to_string(),
            "--unified=3".to_string(),
            revision.clone(),
            "--".to_string(),
        ];
        patch_args.extend(selected_files.into_iter().map(|(_, path)| path));
        let (code, patch, stderr) = run_git_command(workspace_root, &patch_args)?;
        if code != 0 {
            return Err(ToolError::ExecutionFailed(format!(
                "git show patch failed: {}",
                stderr.trim()
            )));
        }
        truncate_lines(&patch, max_lines)
    };

    Ok(json!({
        "workspace_root": display_path_for_tool_output(workspace_root, path_guard),
        "commit": {
            "id": fields[0],
            "parents": fields[1].split_whitespace().collect::<Vec<_>>(),
            "author": fields[2],
            "authored_at": fields[3],
            "subject": fields[4],
        },
        "files": files,
        "patch": patch,
    }))
}

#[async_trait]
impl ToolDefinition for GitInspect {
    fn name(&self) -> &str {
        crate::tools::TOOL_GIT_INSPECT
    }

    fn description(&self) -> &str {
        "Safe, read-only Git inspection for child agents. Supports only fixed status, log, show, blame, and merge_base operations within authorized workspace roots. Use status before reviewing local changes and branches, merge_base to select a comparison base, log and show for bounded history and patches, and blame only for one bounded file range. It does not expose bash or arbitrary Git arguments."
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
                "required": ["operation"],
                "properties": {
                    "operation": { "type": "string", "enum": ["status", "log", "show", "blame", "merge_base"] },
                    "revision": { "type": "string", "description": "Commit revision for log, show, or merge_base. Defaults to HEAD for log and merge_base; required for show." },
                    "other_revision": { "type": "string", "description": "Second commit revision required for merge_base." },
                    "path": { "type": "string", "description": "Optional file or directory inside authorized workspace roots. blame requires exactly one file path." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Optional authorized path filters; not supported by blame." },
                    "line_start": { "type": "integer", "description": "First line for blame. Required with line_end; range is limited to 200 lines." },
                    "line_end": { "type": "integer", "description": "Last line for blame. Required with line_start; range is limited to 200 lines." },
                    "max_count": { "type": "integer", "default": 20, "description": "Bounded history count for log (1-100)." },
                    "max_files": { "type": "integer", "default": 20, "description": "Bounded file count for status/show (1-100)." },
                    "max_lines": { "type": "integer", "default": 160, "description": "Bounded patch lines for show (1-1000)." }
                }
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let operation = params["operation"].as_str().unwrap_or_default();
        if !matches!(
            operation,
            "status" | "log" | "show" | "blame" | "merge_base"
        ) {
            return Err(ToolError::ExecutionFailed(
                "operation must be one of: status, log, show, blame, merge_base".to_string(),
            ));
        }
        let revision = params["revision"].as_str().unwrap_or("HEAD").trim();
        if operation == "show" && revision.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "revision is required for show".to_string(),
            ));
        }
        let other_revision = params["other_revision"].as_str().unwrap_or_default().trim();
        if operation == "merge_base" && other_revision.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "other_revision is required for merge_base".to_string(),
            ));
        }
        let line_start = params["line_start"].as_u64().unwrap_or(0) as usize;
        let line_end = params["line_end"].as_u64().unwrap_or(0) as usize;
        if operation == "blame"
            && (line_start == 0 || line_end < line_start || line_end - line_start >= 200)
        {
            return Err(ToolError::ExecutionFailed(
                "blame requires a valid line_start..line_end range of at most 200 lines"
                    .to_string(),
            ));
        }

        let requested_paths = read_requested_paths(&params);
        let explicit_paths = !requested_paths.is_empty();
        let path_groups = resolve_requested_paths(&requested_paths, self.path_guard.as_ref())?;
        if operation == "blame"
            && (requested_paths.len() != 1
                || path_groups.len() != 1
                || path_groups.values().any(|paths| paths.len() != 1))
        {
            return Err(ToolError::ExecutionFailed(
                "blame requires exactly one authorized file path in one workspace root".to_string(),
            ));
        }
        let max_count = clamp_param(&params, "max_count", DEFAULT_MAX_COUNT, MAX_COUNT_LIMIT);
        let max_files = clamp_param(&params, "max_files", DEFAULT_MAX_FILES, MAX_FILES_LIMIT);
        let max_lines = clamp_param(&params, "max_lines", DEFAULT_MAX_LINES, MAX_LINES_LIMIT);
        let mut repositories = Vec::new();

        for (workspace_root, pathspecs) in path_groups {
            match ensure_git_repository(&workspace_root) {
                Ok(()) => {}
                Err(_) if !explicit_paths => continue,
                Err(err) => return Err(err),
            }
            let pathspecs = pathspecs.into_iter().collect::<Vec<_>>();
            let result = match operation {
                "status" => status_for_root(
                    &workspace_root,
                    pathspecs,
                    self.path_guard.as_ref(),
                    max_files,
                ),
                "log" => log_for_root(
                    &workspace_root,
                    pathspecs,
                    revision,
                    max_count,
                    self.path_guard.as_ref(),
                ),
                "show" => show_for_root(
                    &workspace_root,
                    pathspecs,
                    revision,
                    max_files,
                    max_lines,
                    self.path_guard.as_ref(),
                ),
                "blame" => blame_for_root(
                    &workspace_root,
                    pathspecs,
                    line_start,
                    line_end,
                    self.path_guard.as_ref(),
                ),
                "merge_base" => merge_base_for_root(
                    &workspace_root,
                    revision,
                    other_revision,
                    self.path_guard.as_ref(),
                ),
                _ => unreachable!(),
            }?;
            repositories.push(result);
        }

        let summary = format!(
            "Git {} completed for {} authorized repository root(s)",
            operation,
            repositories.len()
        );
        Ok(ToolCallResult::success(
            Some(summary.clone()),
            Some(json!({
                "operation": operation,
                "repositories": repositories,
                "summary": summary,
                "llm_content": summary,
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_name_status_z, GitInspect};
    use crate::tools::ToolDefinition;
    use crate::workflow::react::security::PathGuard;
    use serde_json::json;
    use std::fs;
    use std::process::Command;
    use std::sync::{Arc, RwLock};
    use tempfile::tempdir;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        assert!(Command::new("git")
            .arg("init")
            .arg(dir.path())
            .status()
            .expect("git init")
            .success());
        for args in [
            ["config", "user.email", "test@example.com"],
            ["config", "user.name", "Test User"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .expect("git config")
                .success());
        }
        dir
    }

    fn tool_for(path: &std::path::Path) -> GitInspect {
        GitInspect::new(Some(Arc::new(RwLock::new(PathGuard::new(
            vec![path.to_path_buf()],
            vec![],
            vec![],
        )))))
    }

    fn commit_file(path: &std::path::Path, name: &str, content: &str, message: &str) {
        fs::write(path.join(name), content).expect("write");
        assert!(Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", name])
            .status()
            .expect("git add")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", message])
            .status()
            .expect("git commit")
            .success());
    }

    #[test]
    fn name_status_z_preserves_special_paths_and_rename_destinations() {
        let entries = parse_name_status_z(
            "M\0line\nname\0R100\0old\tname\0new -> name\0C100\0copy-source\0copy\nname\0",
        )
        .expect("parse name-status");
        assert_eq!(
            entries,
            vec![
                ("M".to_string(), "line\nname".to_string()),
                ("R100".to_string(), "new -> name".to_string()),
                ("C100".to_string(), "copy\nname".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn git_inspect_returns_status_and_log() {
        let dir = init_repo();
        commit_file(dir.path(), "src.txt", "one\n", "initial");
        fs::write(dir.path().join("src.txt"), "one\ntwo\n").expect("modify");
        let tool = tool_for(dir.path());

        let status = tool
            .call(json!({ "operation": "status" }))
            .await
            .expect("status");
        assert_eq!(
            status.structured_content.expect("status data")["repositories"][0]["files"][0]["path"],
            "src.txt"
        );

        let log = tool
            .call(json!({ "operation": "log", "max_count": 1 }))
            .await
            .expect("log");
        assert_eq!(
            log.structured_content.expect("log data")["repositories"][0]["commits"][0]["subject"],
            "initial"
        );
    }

    #[tokio::test]
    async fn git_inspect_bounds_blame_and_resolves_merge_base() {
        let dir = init_repo();
        commit_file(dir.path(), "src.txt", "one\ntwo\n", "initial");
        commit_file(dir.path(), "src.txt", "one\ntwo\nthree\n", "follow up");
        let tool = tool_for(dir.path());

        let blame = tool
            .call(json!({
                "operation": "blame",
                "path": "src.txt",
                "line_start": 1,
                "line_end": 2,
            }))
            .await
            .expect("blame");
        assert_eq!(
            blame.structured_content.expect("blame data")["repositories"][0]["lines"]
                .as_array()
                .expect("blame lines")
                .len(),
            2
        );

        let merge_base = tool
            .call(json!({
                "operation": "merge_base",
                "revision": "HEAD",
                "other_revision": "HEAD~1",
            }))
            .await
            .expect("merge base");
        assert_eq!(
            merge_base.structured_content.expect("merge base data")["repositories"][0]
                ["merge_base"],
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["rev-parse", "HEAD~1"])
                .output()
                .expect("resolve parent")
                .stdout
                .iter()
                .map(|byte| *byte as char)
                .collect::<String>()
                .trim()
        );

        let err = tool
            .call(json!({
                "operation": "blame",
                "path": "src.txt",
                "line_start": 1,
                "line_end": 201,
            }))
            .await
            .expect_err("oversized blame range");
        assert!(err.to_string().contains("at most 200 lines"));
    }

    #[tokio::test]
    async fn git_inspect_shows_resolved_commit_and_rejects_option_revision() {
        let dir = init_repo();
        commit_file(dir.path(), "src.txt", "one\n", "initial");
        let tool = tool_for(dir.path());

        let show = tool
            .call(json!({ "operation": "show", "revision": "HEAD" }))
            .await
            .expect("show");
        assert_eq!(
            show.structured_content.expect("show data")["repositories"][0]["commit"]["subject"],
            "initial"
        );

        let empty_show = tool
            .call(json!({
                "operation": "show",
                "revision": "HEAD",
                "path": "missing.txt",
            }))
            .await
            .expect("empty path-filtered show");
        let empty_show_data = empty_show.structured_content.expect("empty show data");
        assert_eq!(empty_show_data["repositories"][0]["files"], json!([]));
        assert_eq!(empty_show_data["repositories"][0]["patch"], "");

        let err = tool
            .call(json!({ "operation": "show", "revision": "--help" }))
            .await
            .expect_err("invalid revision");
        assert!(err.to_string().contains("not an option"));
    }
}
