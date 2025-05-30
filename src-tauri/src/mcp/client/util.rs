use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rmcp::model::ListToolsResult;
use serde_json::Value;

use crate::ai::traits::chat::MCPToolDeclaration;

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
            disabled: false,
        });
    }
    tools
}

/// Tries to find an executable by checking common installation paths.
/// Returns the absolute path to the executable if found.
pub async fn find_executable_in_common_paths(command_name: &str) -> Option<PathBuf> {
    // 1. Check if command_name is already an absolute path and executable
    let p = Path::new(command_name);
    if p.is_absolute() && p.exists() && is_executable::is_executable(p) {
        return Some(p.to_path_buf());
    }

    // 2. Try `which` crate first (checks system PATH and common suffixes like .exe, .cmd on Windows)
    if let Ok(path_from_which) = which::which(command_name) {
        if path_from_which.exists() && is_executable::is_executable(&path_from_which) {
            log::debug!(
                "Found executable '{}' via `which` at: {}",
                command_name,
                path_from_which.display()
            );
            return Some(path_from_which);
        }
    }

    // 3. Define common paths to search
    let mut search_directories: Vec<PathBuf> = Vec::new();

    // Add paths from current PATH environment variable
    if let Ok(sys_path) = std::env::var("PATH") {
        for p_str in std::env::split_paths(&sys_path) {
            search_directories.push(p_str);
        }
    }

    #[cfg(windows)]
    {
        search_directories.push(PathBuf::from("C:\\Program Files\\nodejs"));
        search_directories.push(PathBuf::from("C:\\Program Files (x86)\\nodejs"));
        if let Some(appdata_local) = dirs::data_local_dir() {
            search_directories.push(appdata_local.join("Programs\\nodejs")); // For user-specific MSI installs
        }
        // NVM for Windows: NVM_HOME usually points to AppData\Roaming\nvm
        // Node versions are typically in NVM_HOME\<version>
        if let Ok(nvm_home_str) = std::env::var("NVM_HOME") {
            let nvm_home = PathBuf::from(nvm_home_str);
            if nvm_home.is_dir() {
                if let Ok(entries) = std::fs::read_dir(nvm_home) {
                    for entry in entries.flatten() {
                        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                            search_directories.push(entry.path()); // e.g., C:\Users\user\AppData\Roaming\nvm\v18.0.0
                        }
                    }
                }
            }
        }
        // Scoop: C:\Users\<User>\scoop\shims
        if let Some(home) = dirs::home_dir() {
            search_directories.push(home.join("scoop").join("shims"));
        }
        // Chocolatey: C:\ProgramData\chocolatey\bin
        if let Some(program_data) = dirs::data_dir().or_else(dirs::data_local_dir) {
            // data_dir is C:\ProgramData
            search_directories.push(program_data.join("chocolatey").join("bin"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        search_directories.push(PathBuf::from("/usr/local/bin"));
        search_directories.push(PathBuf::from("/opt/homebrew/bin")); // For Apple Silicon Homebrew
        if let Some(home_dir) = dirs::home_dir() {
            // NVM
            let nvm_base = home_dir.join(".nvm/versions/node");
            if nvm_base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(nvm_base) {
                    for entry in entries.flatten() {
                        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                            search_directories.push(entry.path().join("bin"));
                        }
                    }
                }
            }
            // FNM, ASDF, Volta would have similar patterns under home_dir
            search_directories.push(home_dir.join(".fnm"));
            search_directories.push(home_dir.join(".asdf").join("shims"));
            search_directories.push(home_dir.join(".volta").join("bin"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        search_directories.push(PathBuf::from("/usr/bin"));
        search_directories.push(PathBuf::from("/usr/local/bin"));
        search_directories.push(PathBuf::from("/snap/bin"));
        if let Some(home_dir) = dirs::home_dir() {
            // NVM, FNM, ASDF, Volta similar to macOS
            let nvm_base = home_dir.join(".nvm/versions/node");
            if nvm_base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(nvm_base) {
                    for entry in entries.flatten() {
                        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                            search_directories.push(entry.path().join("bin"));
                        }
                    }
                }
            }
            search_directories.push(home_dir.join(".fnm"));
            search_directories.push(home_dir.join(".asdf").join("shims"));
            search_directories.push(home_dir.join(".volta").join("bin"));
        }
    }

    // Deduplicate paths before searching
    let mut unique_search_dirs = std::collections::HashSet::new();

    for dir in search_directories.into_iter().filter(|d| d.is_dir()) {
        if unique_search_dirs.insert(dir.clone()) {
            let candidate = dir.join(command_name);
            if candidate.exists() && is_executable::is_executable(&candidate) {
                log::debug!(
                    "Found executable '{}' in common path: {}",
                    command_name,
                    candidate.display()
                );
                return Some(candidate);
            }

            // On Windows, also check for .cmd, .bat, .exe
            #[cfg(windows)]
            {
                for ext in ["cmd", "bat", "exe"].iter() {
                    candidate = dir.join(format!("{}.{}", command_name, ext));
                    if candidate.exists() && is_executable::is_executable(&candidate) {
                        log::debug!(
                            "Found executable '{}' (with .{}) in common path: {}",
                            command_name,
                            ext,
                            candidate.display()
                        );
                        return Some(candidate);
                    }
                }
            }
        }
    }

    log::warn!(
        "Executable '{}' not found after checking `which` and common paths.",
        command_name
    );
    None
}
