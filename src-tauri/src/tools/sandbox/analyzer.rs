use crate::tools::helper::{leading_command_index, shell_tokens, split_shell_command_segments};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellStageCapabilities {
    pub normalized_command: String,
    pub executable: String,
    pub capabilities: BTreeSet<String>,
    pub invocation_tags: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellCommandAnalysis {
    pub stages: Vec<ShellStageCapabilities>,
    pub required_capabilities: BTreeSet<String>,
}

pub fn analyze_shell_command(command: &str) -> ShellCommandAnalysis {
    analyze_shell_command_at_depth(command, 0)
}

fn analyze_shell_command_at_depth(command: &str, depth: u8) -> ShellCommandAnalysis {
    let mut analysis = ShellCommandAnalysis::default();
    for segment in split_shell_command_segments(command) {
        let Some(tokens) = shell_tokens(&segment) else {
            continue;
        };
        let index = leading_command_index(&tokens);
        let Some(executable) = tokens.get(index) else {
            continue;
        };
        if matches!(executable.as_str(), "cd" | "pushd" | "popd") {
            continue;
        }

        let executable = executable
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(executable)
            .to_ascii_lowercase();
        let mut capabilities = executable_capabilities(&executable);
        let mut invocation_tags = BTreeSet::new();
        if is_tauri_invocation(&executable, &tokens[index + 1..]) {
            capabilities.insert("rust".to_string());
            capabilities.insert("tauri".to_string());
            invocation_tags.insert("tauri".to_string());
        }
        analysis
            .required_capabilities
            .extend(capabilities.iter().cloned());
        analysis.stages.push(ShellStageCapabilities {
            normalized_command: tokens[index..].join(" "),
            executable: executable.clone(),
            capabilities,
            invocation_tags,
        });
        if depth < 4 && matches!(executable.as_str(), "sh" | "bash" | "zsh") {
            if let Some(script) = shell_c_script(&tokens[index + 1..]) {
                let nested = analyze_shell_command_at_depth(script, depth + 1);
                analysis
                    .required_capabilities
                    .extend(nested.required_capabilities);
                analysis.stages.extend(nested.stages);
            }
        }
    }
    analysis
}

fn shell_c_script(arguments: &[String]) -> Option<&str> {
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-c" => return arguments.get(index + 1).map(String::as_str),
            option if option.starts_with('-') => index += 1,
            _ => return None,
        }
    }
    None
}

fn executable_capabilities(executable: &str) -> BTreeSet<String> {
    let capabilities: &[&str] = match executable {
        "bash" | "sh" | "zsh" => &["bash"],
        "python" | "python3" | "pip" | "pip3" => &["python"],
        "node" | "npm" | "pnpm" | "yarn" | "npx" => &["node"],
        "cargo" | "rustc" | "rustup" | "rustfmt" | "rustdoc" | "cargo-fmt" | "cargo-clippy" => {
            &["rust"]
        }
        "tauri" => &["rust", "tauri"],
        "git" => &["git"],
        "go" => &["go"],
        "php" | "composer" => &["php"],
        _ => &[],
    };
    capabilities
        .iter()
        .map(|capability| (*capability).to_string())
        .collect()
}

fn is_tauri_invocation(executable: &str, arguments: &[String]) -> bool {
    if executable == "tauri" {
        return true;
    }
    if !matches!(executable, "npm" | "pnpm" | "yarn" | "npx" | "cargo") {
        return false;
    }
    let args = arguments
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .map(String::as_str)
        .collect::<Vec<_>>();
    matches!(args.as_slice(), ["tauri", ..] | ["run", "tauri", ..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_tauri_without_promoting_normal_arguments() {
        for command in [
            "pnpm tauri build",
            "npm run tauri dev",
            "yarn tauri build",
            "npx tauri build",
            "cargo tauri build",
            "tauri build",
        ] {
            let analysis = analyze_shell_command(command);
            assert_eq!(
                analysis.required_capabilities,
                BTreeSet::from(["node".to_string(), "rust".to_string(), "tauri".to_string()])
                    .into_iter()
                    .filter(|capability| command.starts_with("pnpm")
                        || command.starts_with("npm")
                        || command.starts_with("yarn")
                        || command.starts_with("npx")
                        || capability != "node")
                    .collect(),
                "{command}"
            );
        }

        assert_eq!(
            analyze_shell_command("bash -c 'echo ready'").required_capabilities,
            BTreeSet::from(["bash".to_string()])
        );
        assert_eq!(
            analyze_shell_command("node test example.js").required_capabilities,
            BTreeSet::from(["node".to_string()])
        );
        assert_eq!(
            analyze_shell_command("pnpm test").required_capabilities,
            BTreeSet::from(["node".to_string()])
        );
    }
}
