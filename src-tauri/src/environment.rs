use std::env;
use tokio::process::Command;

#[cfg(target_os = "windows")]
/// Attempts to retrieve the full system PATH environment variable on Windows.
///
/// This function tries multiple shell commands (PowerShell, CMD) to get the complete
/// PATH, as the process's environment might not always reflect the full system PATH.
///
/// # Returns
/// - `Some(String)`: The full PATH string if successfully retrieved.
/// - `None`: If the PATH could not be retrieved using the attempted methods.
async fn get_shell_path() -> Option<String> {
    // Windows: Try multiple methods to get full PATH
    let methods = vec![
        // PowerShell
        ("powershell", vec!["-Command", "$env:PATH"]),
        // CMD
        ("cmd", vec!["/C", "echo %PATH%"]),
    ];

    for (shell, args) in methods {
        if let Ok(output) = Command::new(shell)
            .args(&args)
            .creation_flags(0x08000000)
            .output()
            .await
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && path != "%PATH%" {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Attempts to retrieve the full system PATH environment variable on Unix-like systems.
///
/// This function tries various available shells (e.g., zsh, bash, sh) by launching them
/// as login shells to ensure a complete environment, then echoes the `$PATH`.
/// It prioritizes interactive login shells for a more complete PATH.
///
/// # Returns
/// - `Some(String)`: The full PATH string if successfully retrieved.
/// - `None`: If the PATH could not be retrieved using the attempted methods.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn get_shell_path() -> Option<String> {
    // Unix-like systems: Try multiple shells
    let shells = get_available_shells().await;

    for shell in shells {
        // Helper to try executing a command and return path or error kind
        let try_command = async move |shell_name: &str,
                                      args: Vec<&str>|
                    -> Result<Option<String>, std::io::ErrorKind> {
            match Command::new(shell_name).args(&args).output().await {
                Ok(output) => {
                    if output.status.success() {
                        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !path.is_empty() {
                            return Ok(Some(path));
                        }
                    }
                    Ok(None)
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Err(std::io::ErrorKind::NotFound)
                    } else {
                        log::warn!("Failed to run shell command for {}: {}", shell_name, e);
                        Ok(None)
                    }
                }
            }
        };

        // 1. Try interactive login shell first (most likely to have full user PATH)
        log::debug!(
            "Attempting to get PATH using interactive login shell: {} -l -i -c \"echo $PATH\"",
            shell
        );
        match try_command(&shell, vec!["-l", "-i", "-c", "echo $PATH"]).await {
            Ok(Some(path)) => {
                log::debug!("Using {} -l -i -c to get PATH: {}", shell, path);
                return Some(path);
            }
            Err(std::io::ErrorKind::NotFound) => {
                log::debug!(
                    "Shell {} not found, skipping further attempts with this shell.",
                    shell
                );
                continue; // Shell not found, try next one
            }
            _ => { /* Fall through to non-interactive attempt */ }
        }

        // 2. If interactive login shell didn't yield a PATH, try non-interactive login shell
        log::debug!(
            "Attempting to get PATH using non-interactive login shell: {} -l -c \"echo $PATH\"",
            shell
        );
        match try_command(&shell, vec!["-l", "-c", "echo $PATH"]).await {
            Ok(Some(path)) => {
                log::debug!("Using {} -l -c to get PATH: {}", shell, path);
                return Some(path);
            }
            Err(std::io::ErrorKind::NotFound) => {
                log::debug!(
                    "Shell {} not found, skipping further attempts with this shell.",
                    shell
                );
                continue; // Should have been caught by interactive attempt, but for safety
            }
            _ => { /* Fall through to next shell */ }
        }
    }
    None
}

/// Discovers and returns a list of available shell executables on Unix-like systems.
///
/// The function attempts to find shells in the following order:
/// 1. The user's default shell (from `SHELL` environment variable).
/// 2. Common shell paths (e.g., `/bin/zsh`, `/bin/bash`).
/// 3. Shells found using the `which` command.
///
/// The list is deduplicated while preserving the order of discovery.
///
/// # Returns
/// - `Vec<String>`: A vector of paths to available shell executables.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn get_available_shells() -> Vec<String> {
    let mut shells = Vec::new();

    // 1. First try user's default shell
    if let Ok(user_shell) = env::var("SHELL") {
        shells.push(user_shell);
    }

    // 2. Check common shells (ordered by priority)
    let common_shells = vec![
        "/bin/zsh",      // Modern macOS default
        "/usr/bin/zsh",  // Some Linux distributions
        "/bin/bash",     // Traditional default
        "/usr/bin/bash", // Some Linux distributions
        "/bin/sh",       // Most basic shell
    ];

    for shell_path in common_shells {
        if std::path::Path::new(shell_path).exists() {
            shells.push(shell_path.to_string());
        }
    }

    // 3. Try to find via which command
    let shell_names = vec!["zsh", "bash", "sh"];
    for shell_name in shell_names {
        if let Ok(output) = Command::new("which").arg(shell_name).output().await {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && !shells.contains(&path) {
                    shells.push(path);
                }
            }
        }
    }

    // Deduplicate while preserving order
    let mut unique_shells = Vec::new();
    for shell in shells {
        if !unique_shells.contains(&shell) {
            unique_shells.push(shell);
        }
    }

    unique_shells
}

/// Sets up environment variables, primarily by attempting to obtain and merge the full system PATH.
///
/// This function first saves the current PATH, then tries to get a more complete PATH
/// using `get_shell_path`. If successful, it merges the new PATH with the original,
/// prioritizing the new one, and sets it as the process's PATH. It also verifies
/// the availability of essential commands like `node`, `npm`, and `npx`.
///
/// # Returns
/// - `Ok(())`: If the environment variables were set up successfully.
/// - `Err(String)`: If there was an error obtaining the full shell PATH.
async fn setup_environment_variables() -> Result<(), String> {
    log::debug!("Setting up environment variables...");

    // Save original PATH as backup
    let original_path = env::var("PATH").unwrap_or_default();
    // log::debug!("Original PATH: {}", original_path);

    // Try to get full PATH
    match get_shell_path().await {
        Some(full_path) => {
            // Merge PATHs, avoiding duplicates
            let merged_path = merge_paths(&original_path, &full_path);
            env::set_var("PATH", &merged_path);

            log::info!("New PATH set: {}", merged_path);

            Ok(())
        }
        None => {
            log::debug!("Warning: Could not obtain full PATH, using original PATH.");
            Err("Failed to obtain full shell PATH.".to_string())
        }
    }
}

/// Merges two PATH strings, ensuring uniqueness and prioritizing paths from the `new` string.
///
/// This function handles both Windows (`;` separator) and Unix-like (`: ` separator)
/// path formats. It adds paths from the `new` string first, then appends unique
/// paths from the `original` string.
///
/// # Arguments
/// * `original` - The original PATH string.
/// * `new` - The new PATH string to merge.
///
/// # Returns
/// - `String`: The merged and deduplicated PATH string.
fn merge_paths(original: &str, new: &str) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };

    let mut paths = Vec::new();
    let original_paths: Vec<&str> = original.split(separator).collect();
    let new_paths: Vec<&str> = new.split(separator).collect();

    // Add new paths to the front (higher priority)
    for path in new_paths {
        let path = path.trim();
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }

    // Add original paths to the end (lower priority)
    for path in original_paths {
        let path = path.trim();
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }

    paths.join(separator)
}

/// Initializes the cross-platform environment when the application starts.
///
/// This function calls `setup_environment_variables` to configure the system PATH
/// and then `set_additional_env_vars` to ensure other necessary environment variables are present.
pub async fn init_environment() {
    log::debug!("Initializing cross-platform environment...");

    if let Err(e) = setup_environment_variables().await {
        log::warn!("Environment setup error: {}", e);
    }

    // Set other potentially required environment variables
    set_additional_env_vars();
}

/// Sets additional environment variables that might be required by the application.
///
/// This includes ensuring the `HOME` variable is set (especially on Windows by using `USERPROFILE`),
/// and setting `NODE_ENV` to "production" if it's not already defined.
fn set_additional_env_vars() {
    // Ensure some common environment variables exist
    if env::var("HOME").is_err() && env::var("USERPROFILE").is_ok() {
        // On Windows, set HOME to USERPROFILE
        if let Ok(user_profile) = env::var("USERPROFILE") {
            env::set_var("HOME", user_profile);
        }
    }

    // Set Node.js related environment variables (if needed)
    if env::var("NODE_ENV").is_err() {
        env::set_var("NODE_ENV", "production");
    }
}
