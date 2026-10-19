use rmcp::model::ListToolsResult;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{ai::traits::chat::MCPToolDeclaration, environment::get_child_environment};

/// get_tools converts ListToolsResult to Vec<MCPToolDeclaration>
///
/// # Arguments
/// * `list_tools_result` - The ListToolsResult to convert
///
/// # Returns
/// * `Vec<MCPToolDeclaration>` - The converted Vec<MCPToolDeclaration>
pub fn get_tools(list_tools_result: &ListToolsResult) -> Vec<MCPToolDeclaration> {
    let mut tools = vec![];
    for tool in list_tools_result.tools.iter() {
        tools.push(MCPToolDeclaration {
            name: tool.name.to_string(),
            description: tool.description.clone().unwrap_or_default().to_string(),
            input_schema: Value::Object(
                Arc::try_unwrap(tool.input_schema.clone()).unwrap_or_else(|arc| (*arc).clone()),
            ),
            output_schema: tool.output_schema.as_ref().map(|o| json!(o)).clone(),
            disabled: false,
            scope: Some(crate::tools::ToolScope::Both),
        });
    }
    tools
}

/// Resolves direct paths before searching the shared child environment's PATH.
/// A miss leaves process startup free to try the original command; no external
/// lookup or login-shell probe is performed here.
pub async fn find_executable_in_common_paths(command_name: &str) -> Option<PathBuf> {
    if command_name.is_empty() {
        return None;
    }

    if let Some(path) = executable_candidate(Path::new(command_name)) {
        return Some(path);
    }

    let environment = get_child_environment();
    let (_, paths) = environment.iter().find(|(key, _)| {
        #[cfg(windows)]
        {
            key.eq_ignore_ascii_case("PATH")
        }
        #[cfg(not(windows))]
        {
            key == "PATH"
        }
    })?;

    for directory in std::env::split_paths(std::ffi::OsStr::new(paths)) {
        if let Some(path) = executable_candidate(&directory.join(command_name)) {
            return Some(path);
        }
    }

    None
}

fn executable_candidate(path: &Path) -> Option<PathBuf> {
    if is_executable_file(path) {
        return Some(path.to_path_buf());
    }

    #[cfg(windows)]
    for extension in [".exe", ".cmd", ".bat"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(extension);
        let candidate = PathBuf::from(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
