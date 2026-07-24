use super::CommandOutputReducer;
use crate::tools::helper::{
    contains_unquoted_shell_operator, shell_tokens, split_shell_command_segments,
};

pub(crate) struct GitReducer;

pub(crate) fn is_git_log_command(command: &str) -> bool {
    split_shell_command_segments(command)
        .iter()
        .filter_map(|segment| shell_tokens(segment))
        .map(|tokens| {
            tokens
                .into_iter()
                .skip_while(is_environment_assignment)
                .collect::<Vec<_>>()
        })
        .any(|tokens| git_subcommand(&tokens) == Some("log"))
}

impl CommandOutputReducer for GitReducer {
    fn matches(&self, normalized_command: &str) -> bool {
        sole_git_command_tokens(normalized_command).is_some()
    }

    fn reduce(&self, normalized_command: &str, exit_code: i32, raw_content: &str) -> String {
        if exit_code != 0 {
            return raw_content.to_string();
        }

        match git_command(normalized_command) {
            Some(GitCommand::Status) => reduce_status_output(exit_code, raw_content),
            Some(GitCommand::Diff | GitCommand::Show) => reduce_diff_output(exit_code, raw_content),
            Some(GitCommand::Add) => reduce_add_output(exit_code, raw_content),
            Some(GitCommand::Commit) => reduce_commit_output(exit_code, raw_content),
            Some(GitCommand::Push) => reduce_push_output(exit_code, raw_content),
            Some(GitCommand::Pull) => reduce_pull_output(exit_code, raw_content),
            None => raw_content.to_string(),
        }
    }

    fn preserve_raw_output(&self, exit_code: i32) -> bool {
        exit_code != 0
    }
}

enum GitCommand {
    Status,
    Diff,
    Show,
    Add,
    Commit,
    Push,
    Pull,
}

fn git_command(command: &str) -> Option<GitCommand> {
    let tokens = sole_git_command_tokens(command)?;
    match git_subcommand(&tokens) {
        Some("status") => Some(GitCommand::Status),
        Some("diff") => Some(GitCommand::Diff),
        Some("show") => Some(GitCommand::Show),
        Some("add") => Some(GitCommand::Add),
        Some("commit") => Some(GitCommand::Commit),
        Some("push") => Some(GitCommand::Push),
        Some("pull") => Some(GitCommand::Pull),
        _ => None,
    }
}

fn sole_git_command_tokens(command: &str) -> Option<Vec<String>> {
    if contains_unquoted_shell_operator(command) {
        return None;
    }

    let tokens = shell_tokens(command)?
        .into_iter()
        .skip_while(is_environment_assignment)
        .collect::<Vec<_>>();
    (tokens.first().is_some_and(|token| token == "git")).then_some(tokens)
}

fn git_subcommand(tokens: &[String]) -> Option<&str> {
    if tokens.first().is_none_or(|token| token != "git") {
        return None;
    }

    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        if !token.starts_with('-') {
            return Some(token);
        }
        index += 1;
        if matches!(token.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            index += 1;
        }
    }
    None
}

fn is_environment_assignment(token: &String) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn reduce_status_output(exit_code: i32, raw_content: &str) -> String {
    let (stdout, stderr) = split_shell_output(raw_content);
    let mut lines = stdout.lines().filter_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_git_status_hint(trimmed) {
            None
        } else {
            Some(line)
        }
    });
    let mut output = format!("Exit code: {exit_code}\n\nGit status:");

    if let Some(first_line) = lines.next() {
        output.push('\n');
        output.push_str(first_line.trim());
    }
    for line in lines {
        output.push('\n');
        output.push_str(line);
    }
    if !stderr.trim().is_empty() {
        output.push_str("\n\nstderr:\n");
        output.push_str(stderr.trim());
    }
    output
}

fn is_git_status_hint(line: &str) -> bool {
    line.starts_with("(use \"git")
        || line.starts_with("(create/copy files")
        || line.starts_with("(all conflicts fixed:")
        || line.starts_with("(use \"git add")
        || line.starts_with("(use \"git restore")
}

fn reduce_add_output(exit_code: i32, raw_content: &str) -> String {
    if exit_code != 0 {
        return raw_content.to_string();
    }

    "ok".to_string()
}

fn reduce_commit_output(exit_code: i32, raw_content: &str) -> String {
    if exit_code != 0 {
        return raw_content.to_string();
    }

    let (stdout, stderr) = split_shell_output(raw_content);
    let hash = stdout
        .lines()
        .map(str::trim)
        .find_map(extract_commit_hash)
        .or_else(|| stderr.lines().map(str::trim).find_map(extract_commit_hash));
    match hash {
        Some(hash) => format!("Exit code: {exit_code}\n\nGit commit: ok {hash}"),
        None => format!("Exit code: {exit_code}\n\nGit commit: ok"),
    }
}

fn extract_commit_hash(line: &str) -> Option<String> {
    let content = line.strip_prefix('[')?.split_once(']')?.0;
    let hash = content.split_whitespace().next_back()?;
    (hash.len() >= 7 && hash.chars().all(|character| character.is_ascii_hexdigit()))
        .then(|| hash.chars().take(7).collect())
}

fn reduce_push_output(exit_code: i32, raw_content: &str) -> String {
    if exit_code != 0 {
        return raw_content.to_string();
    }

    let (stdout, stderr) = split_shell_output(raw_content);
    let all_output = [stdout, stderr].join("\n");
    let summary = if all_output.contains("Everything up-to-date") {
        "ok (up-to-date)".to_string()
    } else if let Some(reference) = all_output
        .lines()
        .find_map(pushed_reference)
        .or_else(|| all_output.lines().find_map(deleted_push_reference))
    {
        format!("ok {reference}")
    } else {
        "ok".to_string()
    };
    format!("Exit code: {exit_code}\n\nGit push: {summary}")
}

fn pushed_reference(line: &str) -> Option<&str> {
    let (_, reference) = line.split_once(" -> ")?;
    reference.split_whitespace().next()
}

fn deleted_push_reference(line: &str) -> Option<&str> {
    let (_, reference) = line.split_once("[deleted]")?;
    let reference = reference.trim();
    (!reference.is_empty()).then_some(reference)
}

fn reduce_pull_output(exit_code: i32, raw_content: &str) -> String {
    if exit_code != 0 {
        return raw_content.to_string();
    }

    let (stdout, stderr) = split_shell_output(raw_content);
    let all_output = [stdout, stderr].join("\n");
    let summary =
        if all_output.contains("Already up to date") || all_output.contains("Already up-to-date") {
            "ok (up-to-date)".to_string()
        } else if let Some(stat) = all_output.lines().find_map(parse_git_stat) {
            format!("ok {stat}")
        } else {
            "ok".to_string()
        };
    format!("Exit code: {exit_code}\n\nGit pull: {summary}")
}

fn parse_git_stat(line: &str) -> Option<String> {
    if !line.contains("file") || !line.contains("changed") {
        return None;
    }

    let mut files = None;
    let mut insertions = None;
    let mut deletions = None;
    for part in line.split(',') {
        let value = part
            .trim()
            .split_whitespace()
            .next()?
            .parse::<usize>()
            .ok()?;
        if part.contains("file") {
            files = Some(value);
        } else if part.contains("insertion") {
            insertions = Some(value);
        } else if part.contains("deletion") {
            deletions = Some(value);
        }
    }

    let files = files?;
    let mut summary = format!("{files} file{}", if files == 1 { "" } else { "s" });
    if let Some(insertions) = insertions {
        summary.push_str(&format!(" +{insertions}"));
    }
    if let Some(deletions) = deletions {
        summary.push_str(&format!(" -{deletions}"));
    }
    Some(summary)
}

fn reduce_diff_output(exit_code: i32, raw_content: &str) -> String {
    let (stdout, stderr) = split_shell_output(raw_content);
    if !stdout.contains("diff --git ") {
        return raw_content.to_string();
    }

    let mut output = format!("Exit code: {exit_code}");
    let header = stdout
        .lines()
        .take_while(|line| !line.starts_with("diff --git "))
        .filter(|line| !line.trim().is_empty())
        .take(20)
        .collect::<Vec<_>>();
    if !header.is_empty() {
        output.push_str("\n\nCommit metadata:\n");
        output.push_str(&header.join("\n"));
    }

    output.push_str("\n\nChanges:\n");
    let (changes, omitted_context_lines) = compact_diff(stdout);
    output.push_str(&changes);
    if omitted_context_lines > 0 {
        output.push_str(&format!(
            "\n... {omitted_context_lines} unchanged context lines omitted; inspect the saved output or request a narrower diff"
        ));
    }
    if !stderr.trim().is_empty() {
        output.push_str("\n\nstderr:\n");
        output.push_str(stderr.trim());
    }
    output
}

fn split_shell_output(raw_content: &str) -> (&str, &str) {
    let Some((_, content)) = raw_content.split_once("\n\n") else {
        return ("", "");
    };
    let Some(content) = content.strip_prefix("stdout:\n") else {
        return content
            .strip_prefix("stderr:\n")
            .map_or(("", ""), |stderr| ("", stderr));
    };

    content
        .split_once("\n\nstderr:\n")
        .map_or((content, ""), |(stdout, stderr)| (stdout, stderr))
}

fn compact_diff(diff: &str) -> (String, usize) {
    let mut output = Vec::new();
    let mut in_file = false;
    let mut omitted_context_lines = 0;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            output.push(line.to_string());
            in_file = true;
            continue;
        }

        if !in_file {
            continue;
        }
        if line.starts_with(' ') {
            omitted_context_lines += 1;
        } else {
            output.push(format!("  {line}"));
        }
    }

    (output.join("\n"), omitted_context_lines)
}

#[cfg(test)]
mod tests {
    use super::{git_command, is_git_log_command, sole_git_command_tokens, GitReducer};
    use crate::tools::helper::CommandOutputReducer;

    #[test]
    fn recognizes_git_status_diff_and_show_in_shell_segments() {
        for command in [
            "git status",
            "git -C app status --short",
            "GIT_PAGER=cat git show HEAD",
            "git add src/main.rs",
            "git commit -m message",
            "git push origin main",
            "git pull --rebase",
        ] {
            assert!(git_command(command).is_some(), "expected {command}");
        }
        for command in [
            "cd app && git diff HEAD~1",
            "git diff && git status",
            "git show; echo trailing output",
            "git status | cat",
            "git diff |& sed -n '1,10p'",
            "git status |& cat",
            "echo leading output && git status",
        ] {
            assert!(git_command(command).is_none(), "unexpected {command}");
        }
        assert!(git_command("git --invalid-global-option").is_none());
        assert!(sole_git_command_tokens("git --invalid-global-option").is_some());
        assert!(git_command("git log -1").is_none());
        for command in ["git log -1", "git --no-pager log -1", "git -C app log -1"] {
            assert!(is_git_log_command(command), "expected {command}");
        }
        assert!(!is_git_log_command("echo git log"));
        assert!(git_command("echo git diff").is_none());
    }

    #[test]
    fn removes_status_hints_without_dropping_changed_files_or_branch_state() {
        let output = "Exit code: 0\n\nstdout:\nOn branch main\nYour branch is ahead of 'origin/main' by 1 commit.\n\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\tmodified:   src/main.rs\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\tnew file.txt\n\nno changes added to commit";
        let reduced = GitReducer.reduce("git status", 0, output);

        assert!(reduced.contains("On branch main"));
        assert!(reduced.contains("ahead of 'origin/main' by 1 commit"));
        assert!(reduced.contains("modified:   src/main.rs"));
        assert!(reduced.contains("new file.txt"));
        assert!(!reduced.contains("use \"git add"));
        assert!(!reduced.contains("use \"git restore"));
    }

    #[test]
    fn summarizes_successful_write_commands_and_preserves_failures() {
        assert_eq!(
            GitReducer.reduce("git add src/main.rs", 0, "Exit code: 0\n"),
            "ok"
        );
        assert_eq!(
            GitReducer.reduce(
                "git commit -m message",
                0,
                "Exit code: 0\n\nstdout:\n[main abc1234def] message\n 1 file changed"
            ),
            "Exit code: 0\n\nGit commit: ok abc1234"
        );
        assert_eq!(
            GitReducer.reduce(
                "git push",
                0,
                "Exit code: 0\n\nstderr:\nWriting objects: 100%\n   abc1234..def5678  main -> main\n"
            ),
            "Exit code: 0\n\nGit push: ok main"
        );
        assert_eq!(
            GitReducer.reduce(
                "git pull",
                0,
                "Exit code: 0\n\nstdout:\nUpdating abc1234..def5678\n 3 files changed, 10 insertions(+), 2 deletions(-)"
            ),
            "Exit code: 0\n\nGit pull: ok 3 files +10 -2"
        );

        assert_eq!(
            GitReducer.reduce(
                "git push origin --delete feature",
                0,
                "Exit code: 0\n\nstderr:\n - [deleted]         feature"
            ),
            "Exit code: 0\n\nGit push: ok feature"
        );

        let failed =
            "Exit code: 1\n\nstderr:\nremote: Permission denied\nfatal: unable to access remote";
        assert_eq!(GitReducer.reduce("git push", 1, failed), failed);
    }

    #[test]
    fn retains_file_metadata_for_renames_new_files_and_binary_diffs() {
        let output = "Exit code: 0\n\nstdout:\ndiff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+new\ndiff --git a/image.png b/image.png\nindex 1234567..7654321 100644\nBinary files a/image.png and b/image.png differ\n";
        let reduced = GitReducer.reduce("git diff", 0, output);

        for metadata in [
            "diff --git a/old.rs b/new.rs",
            "similarity index 100%",
            "rename from old.rs",
            "rename to new.rs",
            "new file mode 100644",
            "--- /dev/null",
            "+++ b/new.txt",
            "Binary files a/image.png and b/image.png differ",
        ] {
            assert!(reduced.contains(metadata), "missing {metadata}");
        }
    }

    #[test]
    fn retains_non_context_patch_records() {
        let context = (1..=100)
            .map(|line| format!(" unchanged context {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = format!(
            "Exit code: 0\n\nstdout:\ndiff --git a/example.txt b/example.txt\nindex 123..456 100644\n--- a/example.txt\n+++ b/example.txt\n@@ -1,103 +1,103 @@\n---removed content\n+++added content\n\\ No newline at end of file\n{context}\ndiff --git a/blob.bin b/blob.bin\nnew file mode 100644\nindex 0000000..1234567\nGIT binary patch\nliteral 4\nLc${{K*\ndelta 2\nKc$@<O00000\n"
        );
        let reduced = GitReducer.reduce("git diff", 0, &output);

        for record in [
            "---removed content",
            "+++added content",
            "\\ No newline at end of file",
            "GIT binary patch",
            "literal 4",
            "Lc${K*",
            "delta 2",
            "Kc$@<O00000",
        ] {
            assert!(reduced.contains(record), "missing {record}");
        }
        assert!(!reduced.contains("\n   unchanged context 1\n"));
        assert!(reduced.contains("100 unchanged context lines omitted"));
    }

    #[test]
    fn retains_all_change_lines_and_metadata_beyond_previous_limits() {
        let change_lines = (1..=30)
            .flat_map(|line| [format!("-old line {line}"), format!("+new line {line}")])
            .collect::<Vec<_>>();
        let file_sections = (1..=170)
            .map(|file| {
                format!(
                    "diff --git a/file-{file}.txt b/file-{file}.txt\nnew file mode 100644\nindex 0000000..{file:07x}\n--- /dev/null\n+++ b/file-{file}.txt\n@@ -0,0 +1 @@\n+file {file}"
                )
            })
            .collect::<Vec<_>>();
        let output = format!(
            "Exit code: 0\n\nstdout:\ndiff --git a/large.txt b/large.txt\nindex 123..456 100644\n--- a/large.txt\n+++ b/large.txt\n@@ -1,130 +1,130 @@\n{}\n{}",
            change_lines.join("\n"),
            file_sections.join("\n"),
        );
        let reduced = GitReducer.reduce("git diff", 0, &output);

        for expected in [
            "-old line 30",
            "+new line 30",
            "diff --git a/file-170.txt b/file-170.txt",
            "new file mode 100644",
            "+++ b/file-170.txt",
            "+file 170",
        ] {
            assert!(reduced.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn retains_hunks_and_changed_lines_while_compacting_a_diff() {
        let output = "Exit code: 0\n\nstdout:\ndiff --git a/src/main.rs b/src/main.rs\nindex 123..456 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@ fn main() {\n-old\n+new\n unchanged\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -4 +4 @@\n-old_lib\n+new_lib\n";
        let reduced = GitReducer.reduce("git diff", 0, output);

        assert!(reduced.contains("src/main.rs"));
        assert!(reduced.contains("@@ -1,2 +1,2 @@ fn main()"));
        assert!(reduced.contains("-old"));
        assert!(reduced.contains("+new"));
        assert!(reduced.contains("src/lib.rs"));
        assert!(!reduced.contains("\n unchanged\n"));
        assert!(reduced.contains("1 unchanged context lines omitted"));
    }
}
