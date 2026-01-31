use rmcp::model::ListToolsResult;
use serde_json::{json, Value};
use std::{env, path::PathBuf, sync::Arc};
use tokio::process::Command as TokioCommand;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

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
            output_schema: tool.output_schema.as_ref().map(|o| json!(o)).clone(),
            disabled: false,
        });
    }
    tools
}

/// Attempts to find an executable by name using a multi-step cross-platform strategy.
/// 1. Uses system-specific commands (`command -v` on Unix, `where` on Windows) for initial lookup.
/// 2. Checks if the command name is an absolute or relative path to an existing file.
/// 3. Searches the current process's PATH environment variable.
/// 4. On Unix-like systems, falls back to searching via the user's login shell environment.
pub async fn find_executable_in_common_paths(command_name: &str) -> Option<PathBuf> {
    // Log the HOME environment variable, as it's crucial for login shells finding profiles.
    match env::var("HOME") {
        Ok(home) => log::debug!("Current HOME environment variable: {}", home),
        Err(_) => log::warn!("HOME environment variable is not set."),
    }

    log::debug!(
        "Attempting to find executable for command: \"{}\"",
        command_name
    );

    // Step 1: Direct command judgment using system utilities.
    // This tries to resolve the command_name using the OS's standard lookup tools.
    log::debug!("Step 1: Trying system utilities...");
    #[cfg(target_family = "unix")]
    {
        let cmd_to_exec = format!("command -v {}", command_name);
        log::debug!("Step 1 (Unix): Executing sh -c \"{}\"", cmd_to_exec);
        let mut cmd = TokioCommand::new("sh");
        cmd.arg("-c").arg(&cmd_to_exec);

        // Ensure no window is created on Windows (if cross-compiling)
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        match cmd.output().await {
            Ok(output) => {
                log::debug!(
                    "Step 1 (Unix) sh -c \"{}\" | Status: {:?}, STDOUT: \"{}\", STDERR: \"{}\"",
                    cmd_to_exec,
                    output.status,
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                if output.status.success() {
                    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    // `command -v` for an external command should output its path.
                    // We ensure it's a single, non-empty line and points to a file.
                    if !path_str.is_empty() && !path_str.contains('\n') {
                        let potential_path = PathBuf::from(&path_str);
                        log::debug!(
                            "Step 1 (Unix): Potential path from command -v: {}",
                            potential_path.display()
                        );
                        if potential_path.is_file() {
                            log::info!(
                                "Step 1 (Unix): Found executable via sh -c \"command -v\": {}",
                                potential_path.display()
                            );
                            return Some(potential_path);
                        } else {
                            log::debug!(
                                "Step 1 (Unix): Path from command -v is not a file: {}",
                                potential_path.display()
                            );
                        }
                    } else {
                        log::debug!("Step 1 (Unix): command -v output was empty or multi-line.");
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Step 1 (Unix): Failed to execute sh -c \"{}\": {}",
                    cmd_to_exec,
                    e
                );
            }
        }
    }

    #[cfg(target_family = "windows")]
    {
        log::debug!("Step 1 (Windows): Executing where {}", command_name);
        let mut cmd = TokioCommand::new("where");
        cmd.arg(command_name);

        // Hide the command window on Windows
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        match cmd.output().await {
            Ok(output) => {
                log::debug!(
                    "Step 1 (Windows) where {} | Status: {:?}, STDOUT: \"{}\", STDERR: \"{}\"",
                    command_name,
                    output.status,
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                if output.status.success() {
                    if let Some(first_path_str) =
                        String::from_utf8_lossy(&output.stdout).lines().next()
                    {
                        let trimmed_path_str = first_path_str.trim();
                        if !trimmed_path_str.is_empty() {
                            let potential_path = PathBuf::from(trimmed_path_str);
                            log::debug!(
                                "Step 1 (Windows): Potential path from where: {}",
                                potential_path.display()
                            );
                            if potential_path.is_file() {
                                log::info!(
                                    "Step 1 (Windows): Found executable via where: {}",
                                    potential_path.display()
                                );
                                return Some(potential_path);
                            } else {
                                log::debug!(
                                    "Step 1 (Windows): Path from where is not a file: {}",
                                    potential_path.display()
                                );
                            }
                        } else {
                            log::debug!(
                                "Step 1 (Windows): where output line was empty after trim."
                            );
                        }
                    } else {
                        log::debug!("Step 1 (Windows): where output was empty.");
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Step 1 (Windows): Failed to execute where {}: {}",
                    command_name,
                    e
                );
            }
        }
    }

    // Step 2: Check if the command name is already a path to an existing file.
    log::debug!(
        "Step 2: Checking if \"{}\" is a direct file path...",
        command_name
    );
    let path = PathBuf::from(command_name);
    if path.is_file() {
        log::info!(
            "Step 2: Command \"{}\" is a direct path to an existing file: {}",
            command_name,
            path.display()
        );
        return Some(path);
    } else {
        log::debug!(
            "Step 2: \"{}\" is not a direct file path or file does not exist.",
            command_name
        );
    }

    // Step 3: Search the current process's PATH environment variable.
    log::debug!("Step 3: Searching current process's PATH...");
    if let Some(paths_var) = env::var_os("PATH") {
        log::debug!("Step 3: PATH environment variable: {:?}", paths_var);
        for path_entry in env::split_paths(&paths_var) {
            log::debug!("Step 3: Checking in PATH entry: {}", path_entry.display());
            let candidate = path_entry.join(command_name);
            if candidate.is_file() {
                log::info!("Step 3: Found executable in PATH: {}", candidate.display());
                return Some(candidate);
            }

            #[cfg(target_family = "windows")]
            {
                let candidate_exe = path_entry.join(format!("{}.exe", command_name));
                if candidate_exe.is_file() {
                    log::info!(
                        "Step 3: Found executable with .exe in PATH: {}",
                        candidate_exe.display()
                    );
                    return Some(candidate_exe);
                }
            }
        }
        log::debug!(
            "Step 3: Command \"{}\" not found in any PATH entry.",
            command_name
        );
    } else {
        log::warn!("Step 3: PATH environment variable not found.");
    }

    // Step 4: On Unix-like systems, fall back to searching via the user's login shell.
    log::debug!("Step 4: Trying Unix login shells...");
    #[cfg(target_family = "unix")]
    {
        let shells = vec!["zsh", "bash", "fish", "ksh", "sh"];
        for shell in shells {
            // Modified command to also print the PATH inside the shell
            // The single quotes here are for the shell command string construction, not Rust's log macro.
            let command_to_run_in_shell = format!(
                "echo \"Attempting to find '{}' with shell '{}'\"; echo \"SHELL_PATH_START\"; echo \"$PATH\"; echo \"SHELL_PATH_END\"; command -v {} 2>/dev/null",
                command_name, shell, command_name
            );
            log::debug!(
                "Step 4: Trying shell {} with: {} -l -c \"{}\"",
                shell,
                shell,
                command_to_run_in_shell
            );
            let mut command = TokioCommand::new(shell);
            command.arg("-l").arg("-c").arg(&command_to_run_in_shell);

            // Ensure no window is created on Windows (if cross-compiling)
            #[cfg(windows)]
            {
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }

            match command.output().await {
                Ok(output) => {
                    log::debug!(
                        "Step 4 Shell {} -l -c \"...\" | Status: {:?}, STDOUT: \"{}\", STDERR: \"{}\"",
                        shell,
                        output.status,
                        String::from_utf8_lossy(&output.stdout).trim(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    if output.status.success() {
                        // Parse the output to extract the path from `command -v`
                        // `command -v` output is expected to be the last non-empty line if successful
                        let stdout_str = String::from_utf8_lossy(&output.stdout);
                        let lines: Vec<&str> = stdout_str.trim().lines().collect();

                        // Log the PATH reported by the shell
                        if let Some(path_start_index) =
                            lines.iter().position(|&l| l == "SHELL_PATH_START")
                        {
                            if let Some(path_end_index) =
                                lines.iter().position(|&l| l == "SHELL_PATH_END")
                            {
                                if path_start_index < path_end_index
                                    && path_end_index > path_start_index + 1
                                {
                                    // PATH might be multi-line if `echo "$PATH"` was complex, join with ':'
                                    let shell_path =
                                        lines[path_start_index + 1..path_end_index].join(":");
                                    log::debug!(
                                        "Step 4: PATH reported by shell {}: {}",
                                        shell,
                                        shell_path
                                    );
                                }
                            }
                        }

                        if let Some(found_path_str) = lines.last() {
                            if !found_path_str.is_empty()
                                && !found_path_str.contains("SHELL_PATH_END")
                                && !found_path_str.contains("SHELL_PATH_START")
                            {
                                // Ensure it's the command output
                                let potential_path = PathBuf::from(found_path_str.trim());
                                log::debug!(
                                    "Step 4: Potential path from shell {} (command -v): {}",
                                    shell,
                                    potential_path.display()
                                );
                                if potential_path.is_file() {
                                    log::info!(
                                        "Step 4: Found executable via login shell {}: {}",
                                        shell,
                                        potential_path.display()
                                    );
                                    return Some(potential_path);
                                } else {
                                    log::debug!("Step 4: Path from shell {} (\"{}\") is not a file or not found by command -v.", shell, potential_path.display());
                                }
                            } else {
                                log::debug!("Step 4: Login shell {} command -v output was empty or part of debug strings.", shell);
                            }
                        } else {
                            log::debug!("Step 4: Login shell {} command -v output produced no usable lines.", shell);
                        }
                    } else {
                        log::info!(
                            "Step 4: Shell '{}' executed, but the inner command failed (e.g., '{}' not found in its path). Trying next shell.",
                            shell, command_name
                        );
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        log::info!(
                            "Step 4: Shell '{}' not found, trying next one. Error: {}",
                            shell,
                            e
                        );
                    } else {
                        log::warn!(
                            "Step 4: Failed to run login shell command {} -l -c \"{}\": {}",
                            shell,
                            command_to_run_in_shell,
                            e
                        );
                    }
                }
            }
        }
        log::debug!(
            "Step 4: Command \"{}\" not found via any attempted login shells.",
            command_name
        );
    }

    log::warn!(
        "All steps failed to find executable for command: \"{}\"",
        command_name
    );
    None
}
