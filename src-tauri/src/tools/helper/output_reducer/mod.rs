mod json;
mod log;
mod node_build;
mod rust_go;

pub(crate) use json::detect_json_stdout;
pub(crate) use log::{reduce_priority_log_output, should_reduce_priority_log};
pub(crate) use node_build::is_node_build_command;
pub(crate) use rust_go::is_go_build_or_test_command;

pub(crate) struct CommandOutputReduction {
    pub content: String,
    pub persist_complete_output: bool,
}

pub(crate) trait CommandOutputReducer {
    fn matches(&self, normalized_command: &str) -> bool;

    fn reduce(&self, normalized_command: &str, exit_code: i32, raw_content: &str) -> String;

    fn persist_complete_output(&self) -> bool {
        false
    }
}

pub(crate) fn reduce_command_output(
    normalized_command: &str,
    exit_code: i32,
    raw_content: &str,
) -> Option<CommandOutputReduction> {
    let reducers: [&dyn CommandOutputReducer; 2] =
        [&node_build::NodeBuildReducer, &rust_go::RustGoReducer];
    let reducer = reducers
        .iter()
        .find(|reducer| reducer.matches(normalized_command))?;

    Some(CommandOutputReduction {
        content: reducer.reduce(normalized_command, exit_code, raw_content),
        persist_complete_output: reducer.persist_complete_output(),
    })
}
