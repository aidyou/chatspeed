use super::types::{ShellCommandAnalysis, ShellCommandStage};
use crate::tools::helper::{leading_command_index, shell_tokens, split_shell_command_segments};

pub(crate) fn analyze_shell_command(command: &str) -> ShellCommandAnalysis {
    let mut stages = Vec::new();
    for segment in split_shell_command_segments(command) {
        collect_stage(&segment, &mut stages);
    }

    ShellCommandAnalysis { stages }
}

fn collect_stage(segment: &str, stages: &mut Vec<ShellCommandStage>) {
    let Some(tokens) = shell_tokens(segment) else {
        return;
    };
    let index = leading_command_index(&tokens);
    if index >= tokens.len() {
        return;
    }

    let executable = executable_name(&tokens[index]);
    if matches!(
        executable.as_str(),
        "cd" | "pushd" | "popd" | "tee" | "xargs"
    ) {
        return;
    }

    if matches!(executable.as_str(), "sh" | "bash" | "zsh") {
        if let Some(script) = shell_c_script(&tokens, index + 1) {
            for nested in split_shell_command_segments(script) {
                collect_stage(&nested, stages);
            }
            return;
        }
    }

    stages.push(ShellCommandStage {
        normalized_command: tokens[index..].join(" "),
        executable,
    });
}

fn executable_name(token: &str) -> String {
    token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .to_ascii_lowercase()
}

fn shell_c_script(tokens: &[String], start: usize) -> Option<&str> {
    let mut index = start;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "-c" => return tokens.get(index + 1).map(String::as_str),
            option if option.starts_with('-') => index += 1,
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_stage_boundaries_and_does_not_promote_arguments() {
        let analysis = analyze_shell_command(
            "NODE_ENV=test pnpm tauri --version && node 'test file.js' && sh -c 'cargo test --lib'",
        );
        assert_eq!(analysis.stages.len(), 3);
        assert_eq!(analysis.stages[0].executable, "pnpm");
        assert_eq!(analysis.stages[1].executable, "node");
        assert_eq!(analysis.stages[2].executable, "cargo");
        assert_eq!(
            analysis.stages[0].normalized_command,
            "pnpm tauri --version"
        );
        assert_eq!(analysis.stages[1].normalized_command, "node test file.js");
        assert_eq!(analysis.stages[2].normalized_command, "cargo test --lib");
    }

    #[test]
    fn skips_navigation_and_keeps_only_executable_match_units() {
        let analysis = analyze_shell_command("git log -n 3 && cd src-tauri/src && cargo check");
        assert_eq!(
            analysis
                .stages
                .iter()
                .map(|stage| stage.normalized_command.as_str())
                .collect::<Vec<_>>(),
            vec!["git log -n 3", "cargo check"]
        );
    }

    #[test]
    fn ordinary_arguments_are_kept_in_their_stage() {
        for command in ["node test xxx.js", "pnpm test", "npm run test"] {
            let analysis = analyze_shell_command(command);
            assert_eq!(analysis.stages.len(), 1, "{command}");
            assert_eq!(
                analysis.stages[0].executable,
                command.split_whitespace().next().unwrap()
            );
        }
    }
}
