use std::env;
use std::io::Read;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// A shell executable that can safely be offered to the workflow terminal UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellDescriptor {
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

    /// Resolves symlinks so callers can tell that `/bin/bash` and `/usr/bin/bash` are one shell.
    fn resolved_path(&self) -> PathBuf {
        std::fs::canonicalize(&self.path).unwrap_or_else(|_| PathBuf::from(&self.path))
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
        if let Some(path) = run_path_probe(shell, &args) {
            if path != "%PATH%" {
                return Some(path);
            }
        }
    }
    None
}

const PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PATH_PROBE_OUTPUT_LIMIT: usize = 64 * 1024;

fn terminate_probe(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

fn run_path_probe(shell: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(shell);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.take(PATH_PROBE_OUTPUT_LIMIT as u64).read_to_end(&mut output);
        let _ = output_tx.send(output);
    });

    let deadline = Instant::now() + PATH_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = output_rx
                    .recv_timeout(Duration::from_millis(100))
                    .ok()?;
                if !status.success() || output.is_empty() {
                    return None;
                }
                let path = String::from_utf8_lossy(&output).trim().to_string();
                return (!path.is_empty()).then_some(path);
            }
            Ok(None) if Instant::now() < deadline => sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                terminate_probe(&mut child);
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn user_tool_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let mut paths = Vec::new();
    if let Some(home) = &home {
        let cargo_home = env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cargo"));
        paths.push(cargo_home.join("bin"));
        paths.push(home.join(".local/bin"));
        paths.push(home.join(".bun/bin"));
    }

    #[cfg(windows)]
    if let Some(home) = home {
        paths.push(home.join("AppData/Roaming/npm"));
        paths.push(home.join("AppData/Local/pnpm"));
        paths.push(home.join(".bun/bin"));

        for path in [
            env::var_os("APPDATA").map(PathBuf::from),
            env::var_os("LOCALAPPDATA").map(PathBuf::from),
            env::var_os("PNPM_HOME").map(PathBuf::from),
            env::var_os("NPM_CONFIG_PREFIX")
                .map(PathBuf::from)
                .map(|path| path.join("bin")),
        ]
        .into_iter()
        .flatten()
        {
            paths.push(path);
        }

        let mut roots = vec![home.join("AppData/Local/fnm"), home.join(".fnm")];
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            roots.push(local_app_data.join("fnm"));
        }
        if let Some(root) = env::var_os("FNM_DIR").map(PathBuf::from) {
            roots.insert(0, root);
        }
        if let Some(root) = env::var_os("FNM_MULTISHELL_PATH").map(PathBuf::from) {
            paths.push(root);
        }
        for root in roots {
            let default_bin = root.join("aliases/default/installation");
            if default_bin.is_dir() {
                paths.push(default_bin);
            }
            let Ok(entries) = std::fs::read_dir(root.join("node-versions")) else {
                continue;
            };
            let mut version_bins = entries
                .flatten()
                .map(|entry| entry.path().join("installation"))
                .filter(|bin| bin.is_dir())
                .collect::<Vec<_>>();
            version_bins.sort();
            paths.extend(version_bins);
        }
    }
    paths
}

fn discovered_tool_path() -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    user_tool_paths()
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn login_path_shell_candidates(terminal_shells: Vec<String>) -> Vec<String> {
    let mut shell_paths = terminal_shells;
    for shell in ["/bin/sh", "/usr/bin/sh"] {
        if is_executable_file(Path::new(shell)) && !shell_paths.iter().any(|path| path == shell) {
            shell_paths.push(shell.to_string());
        }
    }
    shell_paths
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
        let probes: &[&[&str]] = &[
            &["-l", "-c", "echo $PATH"][..],
            &["-c", "echo $PATH"][..],
        ];
        for args in probes {
            if let Some(path) = run_path_probe(&shell_path, args) {
                log::debug!("Using bounded PATH probe: {} {:?}", shell_path, args);
                return Some(path);
            }
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn get_shell_path() -> Option<String> {
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
pub fn get_available_shells() -> Vec<ShellDescriptor> {
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
            let mut command = Command::new("where");
            command.arg(name);
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            if let Ok(output) = command.output() {
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
            if supports_terminal_shell(&shell) {
                shells.push(shell);
            }
        }
    }

    dedupe_shells(shells)
}

/// Keeps the first candidate of every distinct executable.
///
/// Candidates are deliberately spelled both as `/bin/<shell>` and `/usr/bin/<shell>`; on merged-usr
/// systems those are the same executable, and keeping both would list one shell twice.
fn dedupe_shells(shells: impl IntoIterator<Item = ShellDescriptor>) -> Vec<ShellDescriptor> {
    let mut resolved: Vec<PathBuf> = Vec::new();
    let mut unique: Vec<ShellDescriptor> = Vec::new();
    for shell in shells {
        let identity = shell.resolved_path();
        if resolved.contains(&identity) {
            continue;
        }
        resolved.push(identity);
        unique.push(shell);
    }
    unique
}

#[cfg(any(test, not(feature = "desktop")))]
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
#[cfg(not(feature = "desktop"))]
pub(crate) fn get_default_shell() -> Option<ShellDescriptor> {
    let user_shell = env::var("SHELL").ok();
    select_default_shell(get_available_shells(), user_shell.as_deref())
}

/// Builds the minimal environment required by a terminal child without exposing it to the UI.
/// The PATH includes the inherited environment, bounded login-shell discovery, and
/// deterministic user tool directories such as Cargo and fnm installations.
#[cfg(not(feature = "desktop"))]
pub(crate) fn get_terminal_environment() -> Vec<(String, String)> {
    let mut environment = env::vars().collect::<Vec<_>>();
    let original_path = env::var("PATH").unwrap_or_default();
    let discovered_path = discovered_tool_path();
    let shell_path = get_shell_path().unwrap_or_default();
    let merged_path = merge_paths(
        &original_path,
        &merge_paths(&discovered_path, &shell_path),
    );
    set_environment_value(&mut environment, "PATH", merged_path);

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

#[cfg(not(feature = "desktop"))]
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

    let original_path = env::var("PATH").unwrap_or_default();
    let discovered_path = discovered_tool_path();
    let shell_path = get_shell_path().unwrap_or_default();
    let merged_path = merge_paths(
        &original_path,
        &merge_paths(&discovered_path, &shell_path),
    );
    if merged_path != original_path {
        env::set_var("PATH", &merged_path);
        log::info!("New PATH set: {}", merged_path);
    }
    Ok(())
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
    use super::{
        dedupe_shells, merge_paths, run_path_probe, select_default_shell, supports_terminal_shell,
        ShellDescriptor,
    };

    #[cfg(unix)]
    #[test]
    fn bounded_probe_does_not_wait_for_stdin_or_sleeping_startup_code() {
        assert_eq!(run_path_probe("/bin/sh", &["-c", "sleep 10; echo bad"]), None);
    }

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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn login_path_candidates_retain_posix_sh_for_path_discovery() {
        let candidates = super::login_path_shell_candidates(Vec::new());
        assert!(candidates.iter().any(|path| path.ends_with("/sh")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_uses_windows_separator_and_deduplicates_entries() {
        let merged = merge_paths(r"C:\Windows;C:\Tools", r"C:\UserBin;C:\Windows");
        assert_eq!(merged, r"C:\UserBin;C:\Windows;C:\Tools");
    }
    #[cfg(unix)]
    #[test]
    fn merged_path_prioritizes_login_shell_entries_without_duplicates() {
        let merged = merge_paths("/usr/bin:/bin:/usr/bin", "/custom/bin:/usr/bin");
        assert_eq!(merged, "/custom/bin:/usr/bin:/bin");
    }

    #[cfg(unix)]
    #[test]
    fn merged_path_resolves_user_tools_in_non_login_host_shell() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("tool directory");
        for name in ["cargo", "pnpm"] {
            let tool = directory.path().join(name);
            std::fs::write(&tool, "#!/bin/sh\nprintf 'user-tool-ok'\n").expect("tool stub");
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
                .expect("executable stub");
        }
        let merged = merge_paths("/usr/bin:/bin", &directory.path().to_string_lossy());
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "cargo --version && pnpm --version"])
            .env("PATH", merged)
            .output()
            .expect("host shell");
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(output.stdout, b"user-tool-okuser-tool-ok");
    }

    #[cfg(unix)]
    #[test]
    fn shells_resolving_to_the_same_executable_are_listed_once() {
        use std::os::unix::fs::symlink;

        let directory =
            std::env::temp_dir().join(format!("chatspeed-shell-dedupe-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let shell_path = directory.join("bash");
        std::fs::write(&shell_path, "").expect("shell stub");
        let alias_path = directory.join("bash-alias");
        symlink(&shell_path, &alias_path).expect("aliased shell stub");

        let shells = dedupe_shells(vec![
            ShellDescriptor {
                name: "bash".to_string(),
                path: shell_path.to_string_lossy().into_owned(),
            },
            ShellDescriptor {
                name: "bash".to_string(),
                path: alias_path.to_string_lossy().into_owned(),
            },
        ]);

        assert_eq!(shells.len(), 1, "one executable must be offered once");
        std::fs::remove_dir_all(&directory).expect("temporary directory cleanup");
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
