mod git;
mod json;
mod log;
mod node_build;
mod rust_go;
mod test;

pub(crate) use git::is_git_log_command;
pub(crate) use json::detect_json_stdout;
pub(crate) use log::{reduce_priority_log_output, should_reduce_priority_log};
pub(crate) use node_build::is_node_build_command;
pub(crate) use rust_go::is_go_build_or_test_command;

pub(crate) struct CommandOutputReduction {
    pub content: String,
    pub persist_complete_output: bool,
    pub preserve_raw_output: bool,
}

pub(crate) trait CommandOutputReducer {
    fn matches(&self, normalized_command: &str) -> bool;

    fn supports(&self, normalized_command: &str) -> bool {
        self.matches(normalized_command)
    }

    fn reduce(&self, normalized_command: &str, exit_code: i32, raw_content: &str) -> String;

    fn persist_complete_output(&self) -> bool {
        false
    }

    fn preserve_raw_output(&self, _exit_code: i32) -> bool {
        false
    }
}

fn command_output_reducers() -> [&'static dyn CommandOutputReducer; 4] {
    [
        &git::GitReducer,
        &node_build::NodeBuildReducer,
        &rust_go::RustGoReducer,
        &test::TestOutputReducer,
    ]
}

pub(crate) fn supports_command_output_reduction(normalized_command: &str) -> bool {
    command_output_reducers()
        .iter()
        .any(|reducer| reducer.supports(normalized_command))
}

pub(crate) fn reduce_command_output(
    normalized_command: &str,
    exit_code: i32,
    raw_content: &str,
) -> Option<CommandOutputReduction> {
    let reducers = command_output_reducers();
    let reducer = reducers
        .iter()
        .find(|reducer| reducer.matches(normalized_command))?;
    let content = reducer.reduce(normalized_command, exit_code, raw_content);
    let preserve_raw_output = reducer.preserve_raw_output(exit_code);

    if !preserve_raw_output && content.len() >= raw_content.len() {
        return None;
    }

    Some(CommandOutputReduction {
        content,
        persist_complete_output: reducer.persist_complete_output(),
        preserve_raw_output,
    })
}

#[cfg(test)]
mod tests {
    use super::supports_command_output_reduction;

    #[test]
    fn detects_commands_with_specialized_output_reduction() {
        for command in [
            "git status",
            "git -C app diff --stat",
            "CI=1 pnpm build",
            "cargo clippy --all-targets",
            "go test ./...",
            "python -m pytest tests",
            "pnpm vitest run",
        ] {
            assert!(
                supports_command_output_reduction(command),
                "expected support for {command}"
            );
        }
    }

    #[test]
    fn rejects_commands_without_specialized_output_reduction() {
        for command in [
            "git log -1",
            "git --invalid-global-option",
            "git --invalid-global-option status",
            "git -c malformed status",
            "git -c =value status",
            "git -c name= status",
            "cargo run",
            "python script.py",
            "echo done",
            "git status && echo done",
        ] {
            assert!(
                !supports_command_output_reduction(command),
                "unexpected support for {command}"
            );
        }
    }
}
