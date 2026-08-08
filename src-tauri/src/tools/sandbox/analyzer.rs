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

    collect_tokens(&tokens, index, stages);
}

fn collect_tokens(tokens: &[String], command_index: usize, stages: &mut Vec<ShellCommandStage>) {
    let Some(command_token) = tokens.get(command_index) else {
        return;
    };
    if collect_wrapped_command(tokens, command_index, stages) {
        return;
    }

    let executable = executable_name(command_token);
    if is_container_selection_neutral(tokens, command_index, command_token) {
        return;
    }

    if matches!(executable.as_str(), "sh" | "bash" | "zsh") {
        if let Some(script) = shell_c_script(tokens, command_index + 1) {
            for nested in split_shell_command_segments(script) {
                collect_stage(&nested, stages);
            }
            return;
        }
    }

    stages.push(ShellCommandStage {
        normalized_command: tokens[command_index..].join(" "),
        executable,
    });
}

fn collect_wrapped_command(
    tokens: &[String],
    command_index: usize,
    stages: &mut Vec<ShellCommandStage>,
) -> bool {
    let Some(command_token) = tokens.get(command_index).map(String::as_str) else {
        return false;
    };

    let nested_command_index = match command_token {
        "env" => env_command_index(tokens, command_index + 1),
        "xargs" => xargs_command_index(tokens, command_index + 1),
        "find" => find_exec_command_index(tokens, command_index + 1),
        "time" => time_command_index(tokens, command_index + 1),
        "nohup" => command_after_options(tokens, command_index + 1, &[]),
        _ => return false,
    };

    if let Some(nested_command_index) = nested_command_index {
        let nested_tokens = &tokens[nested_command_index..];
        collect_tokens(nested_tokens, 0, stages);
    }
    true
}

fn env_command_index(tokens: &[String], mut index: usize) -> Option<usize> {
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--" => return (index + 1 < tokens.len()).then_some(index + 1),
            "-u" | "--unset" | "-C" | "--chdir" => index += 2,
            "-S" | "--split-string" => return None,
            option if option.starts_with('-') || is_environment_assignment(option) => index += 1,
            _ => return Some(index),
        }
    }
    None
}

fn time_command_index(tokens: &[String], index: usize) -> Option<usize> {
    command_after_options(tokens, index, &["-f", "--format", "-o", "--output"])
}

fn command_after_options(
    tokens: &[String],
    mut index: usize,
    value_options: &[&str],
) -> Option<usize> {
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--" => return (index + 1 < tokens.len()).then_some(index + 1),
            option if value_options.contains(&option) => index += 2,
            option if option.starts_with('-') => index += 1,
            _ => return Some(index),
        }
    }
    None
}

fn xargs_command_index(tokens: &[String], mut index: usize) -> Option<usize> {
    while index < tokens.len() {
        match tokens[index].as_str() {
            "-E" | "--eof" | "-I" | "--replace" | "-L" | "--max-lines" | "-n" | "--max-args"
            | "-P" | "--max-procs" | "-s" | "--max-chars" | "-a" | "--arg-file" | "-d"
            | "--delimiter" => index += 2,
            option if option.starts_with('-') => index += 1,
            _ => return Some(index),
        }
    }
    None
}

fn find_exec_command_index(tokens: &[String], start: usize) -> Option<usize> {
    tokens[start..]
        .iter()
        .position(|token| matches!(token.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
        .map(|offset| start + offset + 1)
        .filter(|index| *index < tokens.len())
}

fn is_environment_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(key, _)| {
        !key.is_empty()
            && key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

// This analysis is used only to select the host or sandbox profile; it does not audit shell
// safety or alter shell approval decisions. These commands are excluded from profile routing,
// execute in the selected environment, and remain subject to the separate shell policies.
const CONTAINER_SELECTION_NEUTRAL_COMMANDS: &[&str] = &[
    // Shell control, navigation, and command lookup.
    "cd", "pushd", "popd", "dirs", "pwd", "echo", "printf", "type", "which", "whereis", "read",
    "test", "[", "true", "false", ":", "getopts", "export", "unset", "set", "declare", "local",
    "alias", "unalias", "wait", "exit", "return", "sleep", "seq",
    // Path, environment, and system inspection.
    "basename", "dirname", "realpath", "readlink", "stat", "wc", "printenv", "uname", "whoami",
    "id", // Text-stream helpers.
    "cat", "grep", "egrep", "fgrep", "tr", "cut", "head", "tail", "less", "more", "uniq", "sort",
    "sed", "awk", "tee",
];

fn is_container_selection_neutral(
    tokens: &[String],
    command_index: usize,
    command_token: &str,
) -> bool {
    if has_dynamic_shell_execution(&tokens[command_index..].join(" ")) {
        return false;
    }

    if command_token == "hostname" {
        return tokens.len() == command_index + 1;
    }

    if command_token == "command" {
        return tokens.len() > command_index + 2
            && tokens
                .get(command_index + 1)
                .is_some_and(|option| matches!(option.as_str(), "-v" | "-V"));
    }

    CONTAINER_SELECTION_NEUTRAL_COMMANDS.contains(&command_token)
}

fn has_dynamic_shell_execution(command: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if !in_single_quote
            && (ch == '`'
                || (ch == '$' && command[index + ch.len_utf8()..].starts_with('('))
                || ((ch == '<' || ch == '>') && command[index + ch.len_utf8()..].starts_with('(')))
        {
            return true;
        }
    }

    false
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
    fn ignores_container_selection_neutral_helpers() {
        for command in [
            "command -v python && type python && which python && whereis python",
            "cat file | grep needle | tail -20",
            "less file && more file && sed -i 's/a/b/' file && awk '{ print $1 }' file",
            "[ -f file ] && : && hostname && printenv && sort -o output input",
            "dirs && export APP_ENV=dev && sleep 1 && tee output.log && seq 1 3",
        ] {
            let analysis = analyze_shell_command(command);
            assert!(analysis.stages.is_empty(), "{command}");
        }
    }

    #[test]
    fn routes_wrappers_by_their_nested_command() {
        for (command, expected) in [
            ("env APP_ENV=test python --version", "python --version"),
            ("env APP_ENV=test && python --version", "python --version"),
            ("printf '%s\\n' input | xargs -n 1 python", "python"),
            ("find . -name '*.py' -exec python {} \\;", "python {} ;"),
            ("time -f '%E' python --version", "python --version"),
            ("nohup python --version", "python --version"),
        ] {
            let analysis = analyze_shell_command(command);
            assert_eq!(analysis.stages.len(), 1, "{command}");
            assert_eq!(analysis.stages[0].executable, "python", "{command}");
            assert_eq!(analysis.stages[0].normalized_command, expected, "{command}");
        }
    }

    #[test]
    fn preserves_dynamic_execution_targets_for_routing() {
        for command in ["echo $(pwd)", "$(which php) -l file.php"] {
            let analysis = analyze_shell_command(command);
            assert_eq!(analysis.stages.len(), 1, "{command}");
        }
    }

    #[test]
    fn ignores_variable_expansions_in_container_selection_neutral_helpers() {
        let analysis = analyze_shell_command("printf '%s\\n' \"$HOME\"");
        assert!(analysis.stages.is_empty());
    }

    #[test]
    fn does_not_treat_shell_specific_print_as_an_output_helper() {
        let analysis = analyze_shell_command("print hello");
        assert_eq!(analysis.stages.len(), 1);
        assert_eq!(analysis.stages[0].executable, "print");
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
