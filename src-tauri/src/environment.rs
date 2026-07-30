use std::env;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A shell executable that can safely be offered to the workflow terminal UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShellDescriptor {
    pub name: String,
    pub path: String,
}

impl ShellDescriptor {
    fn from_path(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        if !is_executable_file(&path) {
            return None;
        }

        Some(Self {
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
            path: path.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(not(any(unix, windows)))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(target_os = "windows")]
/// Attempts to retrieve the full system PATH environment variable on Windows.
///
/// This function tries multiple shell commands (PowerShell, CMD) to get the complete
/// PATH, as the process's environment might not always reflect the full system PATH.
///
/// # Returns
/// - `Some(String)`: The full PATH string if successfully retrieved.
/// - `None`: If the PATH could not be retrieved using the attempted methods.
fn get_shell_path() -> Option<String> {
    // Windows: Try multiple methods to get full PATH
    let methods = vec![
        // PowerShell
        ("powershell", vec!["-Command", "$env:PATH"]),
        // CMD
        ("cmd", vec!["/C", "echo %PATH%"]),
    ];

    for (shell, args) in methods {
        let mut command = Command::new(shell);
        command.args(&args);
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW

        if let Ok(output) = command.output() {
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
fn login_path_shell_candidates(terminal_shells: Vec<String>) -> Vec<String> {
    #[cfg(unix)]
    {
        let mut shell_paths = terminal_shells;
        for shell in ["/bin/sh", "/usr/bin/sh"] {
            if is_executable_file(Path::new(shell)) && !shell_paths.iter().any(|path| path == shell)
            {
                shell_paths.push(shell.to_string());
            }
        }
        shell_paths
    }

    #[cfg(not(unix))]
    {
        terminal_shells
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn get_shell_path() -> Option<String> {
    // Keep this general environment probe independent of the terminal UI allowlist: a POSIX
    // `sh` remains a valid login-PATH fallback even though it lacks the terminal prompt contract.
    let mut shell_paths = login_path_shell_candidates(
        get_available_shells()
            .into_iter()
            .map(|shell| shell.path)
            .collect(),
    );
    if let Ok(output) = Command::new("which").arg("sh").output() {
        let shell = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() && !shell.is_empty() && !shell_paths.contains(&shell) {
            shell_paths.push(shell);
        }
    }

    for shell_path in shell_paths {
        let try_command =
            |shell_name: &str, args: Vec<&str>| -> Result<Option<String>, std::io::ErrorKind> {
                match Command::new(shell_name).args(&args).output() {
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
            shell_path
        );
        match try_command(&shell_path, vec!["-l", "-i", "-c", "echo $PATH"]) {
            Ok(Some(path)) => {
                log::debug!("Using {} -l -i -c to get PATH", shell_path);
                return Some(path);
            }
            Err(std::io::ErrorKind::NotFound) => continue,
            _ => {}
        }

        match try_command(&shell_path, vec!["-l", "-c", "echo $PATH"]) {
            Ok(Some(path)) => {
                log::debug!("Using {} -l -c to get PATH", shell_path);
                return Some(path);
            }
            Err(std::io::ErrorKind::NotFound) => continue,
            _ => {}
        }
    }
    None
}

fn supports_terminal_shell(shell: &ShellDescriptor) -> bool {
    #[cfg(unix)]
    {
        // Only expose shells with an explicitly verified interactive launch and prompt/OSC 7
        // contract in `terminal.rs`. Keep generic POSIX shells available for PATH discovery.
        matches!(shell.name.as_str(), "bash" | "zsh" | "fish")
    }
    #[cfg(windows)]
    {
        matches!(
            shell.name.to_ascii_lowercase().as_str(),
            "pwsh.exe" | "powershell.exe" | "cmd.exe"
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Returns executable shells available for interactive workflow terminal sessions.
///
/// On Unix this honors `$SHELL`, then `/etc/shells`, conventional locations and PATH.
/// Windows follows its native shell order instead of applying a Unix shell allowlist.
pub(crate) fn get_available_shells() -> Vec<ShellDescriptor> {
    let mut candidates = Vec::new();

    #[cfg(unix)]
    {
        if let Ok(shell) = env::var("SHELL") {
            candidates.push(PathBuf::from(shell));
        }

        if let Ok(shells) = std::fs::read_to_string("/etc/shells") {
            candidates.extend(shells.lines().filter_map(|line| {
                let line = line.trim();
                (!line.is_empty() && !line.starts_with('#')).then(|| PathBuf::from(line))
            }));
        }

        candidates.extend([
            PathBuf::from("/bin/zsh"),
            PathBuf::from("/usr/bin/zsh"),
            PathBuf::from("/bin/bash"),
            PathBuf::from("/usr/bin/bash"),
            PathBuf::from("/bin/fish"),
            PathBuf::from("/usr/bin/fish"),
        ]);

        for name in ["zsh", "bash", "fish"] {
            if let Ok(output) = Command::new("which").arg(name).output() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() && !path.is_empty() {
                    candidates.push(PathBuf::from(path));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        for name in ["pwsh.exe", "powershell.exe", "cmd.exe"] {
            if let Ok(output) = Command::new("where").arg(name).output() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if output.status.success() && !path.is_empty() {
                    candidates.push(PathBuf::from(path));
                }
            }
        }
    }

    let mut shells = Vec::new();
    for candidate in candidates {
        if let Some(shell) = ShellDescriptor::from_path(candidate) {
            if supports_terminal_shell(&shell)
                && !shells
                    .iter()
                    .any(|existing: &ShellDescriptor| existing.path == shell.path)
            {
                shells.push(shell);
            }
        }
    }

    shells
}

fn select_default_shell(
    shells: Vec<ShellDescriptor>,
    #[allow(unused_variables)] user_shell: Option<&str>,
) -> Option<ShellDescriptor> {
    #[cfg(unix)]
    if let Some(user_shell) = user_shell {
        if let Some(shell) = shells.iter().find(|shell| shell.path == user_shell) {
            return Some(shell.clone());
        }
    }

    #[cfg(target_os = "macos")]
    let preferred = ["zsh", "bash"];
    #[cfg(target_os = "linux")]
    let preferred = ["bash", "zsh"];
    #[cfg(windows)]
    let preferred = ["pwsh.exe", "powershell.exe", "cmd.exe"];
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let preferred: [&str; 0] = [];

    preferred
        .iter()
        .find_map(|name| {
            shells
                .iter()
                .find(|shell| shell.name.eq_ignore_ascii_case(name))
                .cloned()
        })
        .or_else(|| shells.into_iter().next())
}

/// Gets the preferred interactive terminal shell for the current platform.
pub(crate) fn get_default_shell() -> Option<ShellDescriptor> {
    let user_shell = env::var("SHELL").ok();
    select_default_shell(get_available_shells(), user_shell.as_deref())
}

/// Builds the minimal environment required by a terminal child without exposing it to the UI.
pub(crate) fn get_terminal_environment() -> Vec<(String, String)> {
    let mut environment = env::vars().collect::<Vec<_>>();
    let original_path = env::var("PATH").unwrap_or_default();
    if let Some(shell_path) = get_shell_path() {
        let merged_path = merge_paths(&original_path, &shell_path);
        set_environment_value(&mut environment, "PATH", merged_path);
    }

    if !environment.iter().any(|(key, _)| key == "HOME") {
        if let Some(home) = dirs::home_dir() {
            set_environment_value(
                &mut environment,
                "HOME",
                home.to_string_lossy().into_owned(),
            );
        }
    }

    environment
}

fn set_environment_value(environment: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some((_, existing_value)) = environment
        .iter_mut()
        .find(|(existing_key, _)| existing_key == key)
    {
        *existing_value = value;
    } else {
        environment.push((key.to_string(), value));
    }
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
fn setup_environment_variables() -> Result<(), String> {
    log::debug!("Setting up environment variables...");

    // Save original PATH as backup
    let original_path = env::var("PATH").unwrap_or_default();
    // log::debug!("Original PATH: {}", original_path);

    // Try to get full PATH
    match get_shell_path() {
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

#[cfg(test)]
mod tests {
    use super::{merge_paths, select_default_shell, supports_terminal_shell, ShellDescriptor};

    #[cfg(unix)]
    #[test]
    fn only_shells_with_verified_terminal_contracts_are_selectable() {
        for name in ["bash", "zsh", "fish"] {
            let shell = ShellDescriptor {
                name: name.to_string(),
                path: format!("/bin/{name}"),
            };
            assert!(supports_terminal_shell(&shell), "{name} must be selectable");
        }

        for name in ["ksh", "mksh", "yash", "sh", "dash"] {
            let shell = ShellDescriptor {
                name: name.to_string(),
                path: format!("/bin/{name}"),
            };
            assert!(
                !supports_terminal_shell(&shell),
                "{name} lacks a verified terminal launch and prompt contract"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn verified_user_default_shell_precedes_platform_fallback() {
        let shells = vec![
            ShellDescriptor {
                name: "bash".to_string(),
                path: "/bin/bash".to_string(),
            },
            ShellDescriptor {
                name: "zsh".to_string(),
                path: "/bin/zsh".to_string(),
            },
            ShellDescriptor {
                name: "fish".to_string(),
                path: "/usr/local/bin/fish".to_string(),
            },
        ];
        let selected = select_default_shell(shells, Some("/usr/local/bin/fish"))
            .expect("a verified default shell should be selected");
        assert_eq!(selected.name, "fish");
    }

    #[cfg(unix)]
    #[test]
    fn unverified_user_default_shell_falls_back_to_platform_shell() {
        let shells = vec![
            ShellDescriptor {
                name: "zsh".to_string(),
                path: "/bin/zsh".to_string(),
            },
            ShellDescriptor {
                name: "bash".to_string(),
                path: "/bin/bash".to_string(),
            },
        ];
        let selected = select_default_shell(shells, Some("/bin/tcsh"))
            .expect("a platform fallback shell should be selected");
        #[cfg(target_os = "macos")]
        assert_eq!(selected.name, "zsh");
        #[cfg(target_os = "linux")]
        assert_eq!(selected.name, "bash");
    }

    #[cfg(unix)]
    #[test]
    fn login_path_candidates_retain_posix_sh_for_path_discovery() {
        let candidates = super::login_path_shell_candidates(Vec::new());
        assert!(candidates.iter().any(|path| path.ends_with("/sh")));
    }

    #[test]
    fn merged_path_prioritizes_login_shell_entries_without_duplicates() {
        let merged = merge_paths("/usr/bin:/bin:/usr/bin", "/custom/bin:/usr/bin");
        assert_eq!(merged, "/custom/bin:/usr/bin:/bin");
    }
}

///
/// This function calls `setup_environment_variables` to configure the system PATH
/// and then `set_additional_env_vars` to ensure other necessary environment variables are present.
pub fn init_environment() {
    log::debug!("Initializing cross-platform environment...");

    if let Err(e) = setup_environment_variables() {
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
