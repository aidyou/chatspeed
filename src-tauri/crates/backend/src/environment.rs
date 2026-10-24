use std::collections::HashSet;
use std::env;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock, RwLock};
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
fn get_shell_path() -> Option<String> {
    let deadline = Instant::now() + SHELL_PATH_PROBE_BUDGET;
    let methods = [
        ("powershell", ["-Command", "$env:PATH"]),
        ("cmd", ["/C", "echo %PATH%"]),
    ];

    for (shell, args) in methods {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Some(path) = run_path_probe_with_timeout(shell, &args, remaining) {
            if path != "%PATH%" {
                return Some(path);
            }
        }
    }
    None
}

const PATH_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const PATH_PROBE_OUTPUT_LIMIT: usize = 64 * 1024;
const SHELL_PATH_PROBE_BUDGET: Duration = Duration::from_millis(750);

fn terminate_probe(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

fn run_path_probe_with_timeout(shell: &str, args: &[&str], timeout: Duration) -> Option<String> {
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
    command.creation_flags(0x08000000);

    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout
            .take(PATH_PROBE_OUTPUT_LIMIT as u64)
            .read_to_end(&mut output);
        let _ = output_tx.send(output);
    });

    let deadline = Instant::now() + timeout.min(PATH_PROBE_TIMEOUT);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = output_rx.recv_timeout(Duration::from_millis(50)).ok()?;
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

#[cfg(test)]
fn run_path_probe(shell: &str, args: &[&str]) -> Option<String> {
    run_path_probe_with_timeout(shell, args, PATH_PROBE_TIMEOUT)
}

fn append_fnm_paths(paths: &mut Vec<PathBuf>, root: &Path) {
    let runtime_bin = |installation: PathBuf| {
        if cfg!(windows) {
            installation
        } else {
            installation.join("bin")
        }
    };
    // fnm aliases normally link to an installation. Also accept an alias to a
    // version directory without depending on a live multishell session.
    let default = root.join("aliases/default");
    let default_bins = [
        runtime_bin(default.clone()),
        runtime_bin(default.join("installation")),
    ];
    let has_default = default_bins.iter().any(|path| path.is_dir());
    paths.extend(default_bins);

    // Do not borrow globally installed tools from a different version when a
    // default runtime is available.
    if !has_default {
        if let Ok(entries) = std::fs::read_dir(root.join("node-versions")) {
            let mut versions = entries
                .flatten()
                .take(128)
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            versions.sort();
            for version in versions {
                paths.push(runtime_bin(version.join("installation")));
            }
        }
    }
    paths.push(root.to_path_buf());
}

fn user_tool_paths() -> Vec<PathBuf> {
    user_tool_paths_with(dirs::home_dir(), |name| env::var_os(name))
}

fn user_tool_paths_with(
    home: Option<PathBuf>,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let append_env_path = |paths: &mut Vec<PathBuf>, name: &str, suffix: &[&str]| {
        if let Some(mut path) = var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            for component in suffix {
                path.push(component);
            }
            paths.push(path);
        }
    };
    let mut paths = Vec::new();
    let multishell_suffix: &[&str] = if cfg!(windows) { &[] } else { &["bin"] };
    append_env_path(&mut paths, "FNM_MULTISHELL_PATH", multishell_suffix);

    let mut fnm_roots = Vec::new();
    append_env_path(&mut fnm_roots, "FNM_DIR", &[]);
    append_env_path(&mut fnm_roots, "XDG_DATA_HOME", &["fnm"]);
    #[cfg(windows)]
    {
        append_env_path(&mut fnm_roots, "APPDATA", &["fnm"]);
        append_env_path(&mut fnm_roots, "LOCALAPPDATA", &["fnm"]);
    }
    if let Some(home) = &home {
        #[cfg(target_os = "macos")]
        fnm_roots.push(home.join("Library/Application Support/fnm"));
        fnm_roots.push(home.join(".local/share/fnm"));
        fnm_roots.push(home.join(".fnm"));
    }
    let mut seen = HashSet::new();
    for root in fnm_roots {
        if seen.insert(root.clone()) {
            append_fnm_paths(&mut paths, &root);
        }
    }

    if let Some(home) = &home {
        paths.extend([
            home.join(".cargo/bin"),
            home.join(".local/bin"),
            home.join(".local/share/pnpm"),
            home.join(".config/pnpm"),
            home.join(".bun/bin"),
            home.join(".volta/bin"),
            home.join(".fnm"),
            home.join(".pyenv/bin"),
            home.join(".pyenv/shims"),
            home.join(".goenv/bin"),
            home.join(".goenv/shims"),
            home.join(".rbenv/bin"),
            home.join(".rbenv/shims"),
            home.join(".rvm/bin"),
            home.join(".asdf/bin"),
            home.join(".asdf/shims"),
            home.join(".mise/bin"),
            home.join(".mise/shims"),
            home.join(".local/share/mise/bin"),
            home.join(".local/share/mise/shims"),
            home.join(".local/share/rtx/bin"),
            home.join(".local/share/rtx/shims"),
            home.join(".nvm/current/bin"),
            home.join(".phpenv/bin"),
            home.join(".phpenv/shims"),
            home.join(".config/composer/vendor/bin"),
            home.join(".composer/vendor/bin"),
            home.join(".poetry/bin"),
            home.join("go/bin"),
            home.join(".sdkman/candidates/java/current/bin"),
            home.join(".sdkman/candidates/maven/current/bin"),
            home.join(".sdkman/candidates/gradle/current/bin"),
        ]);

        // Version managers commonly keep the active runtime one level below a
        // stable root. Scan only that bounded level; never execute an init script.
        for (root, suffix) in [
            (home.join(".nvm/versions/node"), PathBuf::from("bin")),
            (home.join(".pyenv/versions"), PathBuf::from("bin")),
            (home.join(".goenv/versions"), PathBuf::from("bin")),
            (home.join(".rbenv/versions"), PathBuf::from("bin")),
            (
                home.join(".sdkman/candidates"),
                PathBuf::from("current/bin"),
            ),
        ] {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten().take(128) {
                    let path = entry.path().join(&suffix);
                    if path.is_dir() {
                        paths.push(path);
                    }
                }
            }
        }
    }

    append_env_path(&mut paths, "CARGO_HOME", &["bin"]);
    append_env_path(&mut paths, "PNPM_HOME", &[]);
    append_env_path(&mut paths, "XDG_DATA_HOME", &["pnpm"]);
    let npm_prefix_suffix: &[&str] = if cfg!(windows) { &[] } else { &["bin"] };
    append_env_path(&mut paths, "NPM_CONFIG_PREFIX", npm_prefix_suffix);
    append_env_path(&mut paths, "GOBIN", &[]);
    if let Some(gopath) = var("GOPATH") {
        for root in env::split_paths(&gopath).filter(|path| !path.as_os_str().is_empty()) {
            paths.push(root.join("bin"));
        }
    }
    append_env_path(&mut paths, "PYENV_ROOT", &["bin"]);
    append_env_path(&mut paths, "PYENV_ROOT", &["shims"]);
    append_env_path(&mut paths, "UV_TOOL_BIN_DIR", &[]);
    append_env_path(&mut paths, "PIPX_BIN_DIR", &[]);
    append_env_path(&mut paths, "POETRY_HOME", &["bin"]);
    if cfg!(windows) {
        append_env_path(&mut paths, "CONDA_PREFIX", &[]);
        append_env_path(&mut paths, "CONDA_PREFIX", &["Scripts"]);
        append_env_path(&mut paths, "CONDA_PREFIX", &["Library", "bin"]);
    } else {
        append_env_path(&mut paths, "CONDA_PREFIX", &["bin"]);
    }
    append_env_path(&mut paths, "PIXI_HOME", &["bin"]);
    append_env_path(&mut paths, "MISE_BIN_PATH", &[]);
    append_env_path(&mut paths, "MISE_DATA_DIR", &["shims"]);
    append_env_path(&mut paths, "ASDF_DATA_DIR", &["shims"]);
    append_env_path(&mut paths, "RBENV_ROOT", &["bin"]);
    append_env_path(&mut paths, "RBENV_ROOT", &["shims"]);
    append_env_path(&mut paths, "RVM_BIN", &[]);
    append_env_path(&mut paths, "COMPOSER_HOME", &["vendor/bin"]);
    append_env_path(&mut paths, "PHP_PEAR_BIN_DIR", &[]);
    append_env_path(&mut paths, "VOLTA_HOME", &["bin"]);
    append_env_path(&mut paths, "NVM_BIN", &[]);
    append_env_path(&mut paths, "NVM_SYMLINK", &[]);
    append_env_path(&mut paths, "SDKMAN_DIR", &["candidates/java/current/bin"]);
    append_env_path(&mut paths, "SDKMAN_DIR", &["candidates/maven/current/bin"]);
    append_env_path(&mut paths, "SDKMAN_DIR", &["candidates/gradle/current/bin"]);

    #[cfg(target_os = "macos")]
    paths.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
        PathBuf::from("/opt/local/bin"),
        PathBuf::from("/opt/local/sbin"),
    ]);

    #[cfg(target_os = "linux")]
    paths.extend([
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
        PathBuf::from("/snap/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/sbin"),
    ]);

    #[cfg(windows)]
    {
        if let Some(home) = home {
            paths.extend([
                home.join("AppData/Roaming/npm"),
                home.join("AppData/Local/pnpm"),
                home.join(".bun/bin"),
                home.join(".volta/bin"),
                home.join(".pyenv/pyenv-win/bin"),
                home.join(".pyenv/pyenv-win/shims"),
                home.join("scoop/shims"),
            ]);
        }
        paths.extend([
            PathBuf::from(r"C:\ProgramData\chocolatey\bin"),
            PathBuf::from(r"C:\Program Files\nodejs"),
        ]);
        if let Some(local_app_data) = var("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            paths.extend([
                local_app_data.join("pnpm"),
                local_app_data.join("Volta/bin"),
                local_app_data.join("Microsoft/WinGet/Links"),
                local_app_data.join("fnm"),
            ]);
        }
        if let Some(app_data) = var("APPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            paths.push(app_data.join("npm"));
        }
        if let Some(nvm_home) = var("NVM_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            paths.push(nvm_home);
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

fn normalize_path_entry(entry: &str) -> Option<PathBuf> {
    let entry = entry.trim().trim_matches('"');
    if entry.is_empty() {
        return None;
    }

    let raw = PathBuf::from(entry);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir().ok()?.join(raw)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir | Component::Normal(_) => normalized.push(component.as_os_str()),
        }
    }

    Some(std::fs::canonicalize(&normalized).unwrap_or(normalized))
}

fn path_identity(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn normalized_path_entries(path_value: &str) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let mut entries = Vec::new();
    let mut identities = HashSet::new();
    for entry in path_value.split(separator) {
        let Some(normalized) = normalize_path_entry(entry) else {
            continue;
        };
        if identities.insert(path_identity(&normalized)) {
            entries.push(normalized);
        }
    }
    entries
}

fn format_path_entries(entries: impl IntoIterator<Item = PathBuf>) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    entries
        .into_iter()
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
    let deadline = Instant::now() + SHELL_PATH_PROBE_BUDGET;
    let mut shell_paths = login_path_shell_candidates(env::var("SHELL").into_iter().collect());

    for shell_path in shell_paths.drain(..) {
        if !is_executable_file(Path::new(&shell_path)) || Instant::now() >= deadline {
            continue;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Some(path) =
            run_path_probe_with_timeout(&shell_path, &["-l", "-c", "echo $PATH"], remaining)
        {
            log::debug!("Using bounded PATH probe: {}", shell_path);
            return Some(path);
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

struct ChildEnvironment {
    values: RwLock<Vec<(String, String)>>,
    probe_complete: tokio::sync::watch::Sender<bool>,
}

static CHILD_ENVIRONMENT: OnceLock<Arc<ChildEnvironment>> = OnceLock::new();

fn start_child_environment(
    values: Vec<(String, String)>,
    probe: impl FnOnce() -> Option<String> + Send + 'static,
) -> Arc<ChildEnvironment> {
    let (probe_complete, _) = tokio::sync::watch::channel(false);
    let environment = Arc::new(ChildEnvironment {
        values: RwLock::new(values),
        probe_complete,
    });
    let background = Arc::clone(&environment);
    let started = std::thread::Builder::new()
        .name("chatspeed-path-probe".to_string())
        .spawn(move || {
            if let Some(shell_path) = probe() {
                if let Ok(mut current) = background.values.write() {
                    let original_path = current
                        .iter()
                        .find(|(key, _)| key == "PATH")
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                    let merged_path = merge_paths(&original_path, &shell_path);
                    set_environment_value(&mut current, "PATH", merged_path);
                }
            }
            background.probe_complete.send_replace(true);
        });
    if started.is_err() {
        environment.probe_complete.send_replace(true);
    }
    environment
}

#[cfg(any(test, not(feature = "desktop")))]
async fn wait_for_path_probe(environment: &ChildEnvironment, budget: Duration) -> bool {
    let mut complete = environment.probe_complete.subscribe();
    tokio::time::timeout(budget, complete.wait_for(|ready| *ready))
        .await
        .is_ok_and(|result| result.is_ok())
}

/// Lets stdio resolve its executable after the one bounded PATH probe has settled.
/// Shell and terminal launches keep their non-blocking fail-open environment.
#[cfg(not(feature = "desktop"))]
pub(crate) async fn wait_for_child_environment() {
    let shared = CHILD_ENVIRONMENT
        .get_or_init(|| start_child_environment(base_child_environment(), get_shell_path));
    if !wait_for_path_probe(shared, Duration::from_secs(1)).await {
        log::warn!("Child PATH probe did not settle within the MCP startup budget");
    }
}

fn base_child_environment() -> Vec<(String, String)> {
    let mut environment = env::vars().collect::<Vec<_>>();
    let original_path = env::var("PATH").unwrap_or_default();
    let discovered_path = discovered_tool_path();
    // Guessed tool directories are fallbacks, not a replacement for an active runtime.
    let merged_path = merge_paths(&discovered_path, &original_path);
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

/// Returns the fail-open environment shared by host shell and MCP children.
pub(crate) fn get_child_environment() -> Vec<(String, String)> {
    let shared = CHILD_ENVIRONMENT
        .get_or_init(|| start_child_environment(base_child_environment(), get_shell_path));

    shared
        .values
        .read()
        .map(|environment| environment.clone())
        .unwrap_or_else(|_| base_child_environment())
}

/// Builds the environment shared by host shell and MCP child processes.
///
/// PATH discovery is deliberately fail-open: the inherited PATH and existing
/// deterministic tool directories are available even when a login shell is
/// missing, slow, or has a broken startup script.
pub(crate) fn get_terminal_environment() -> Vec<(String, String)> {
    get_child_environment()
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

/// Sets up the process PATH from the same fail-open child environment used by
/// host shell and MCP subprocesses. Discovery failures never fail initialization.
fn setup_environment_variables() -> Result<(), String> {
    log::debug!("Setting up environment variables...");
    let environment = get_child_environment();
    if let Some((_, path)) = environment.iter().find(|(key, _)| key == "PATH") {
        if env::var("PATH").as_deref() != Ok(path.as_str()) {
            env::set_var("PATH", path);
            log::debug!("Normalized process PATH for child compatibility");
        }
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
    let mut entries = normalized_path_entries(new);
    let mut identities = entries
        .iter()
        .map(|path| path_identity(path))
        .collect::<HashSet<_>>();
    for path in normalized_path_entries(original) {
        if identities.insert(path_identity(&path)) {
            entries.push(path);
        }
    }
    format_path_entries(entries)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::user_tool_paths;
    use super::{
        dedupe_shells, merge_paths, run_path_probe, select_default_shell, supports_terminal_shell,
        ShellDescriptor,
    };

    #[test]
    fn tool_directory_variables_do_not_gain_an_extra_bin_suffix() {
        let directory = tempfile::tempdir().expect("directory");
        for name in [
            "PNPM_HOME",
            "GOBIN",
            "UV_TOOL_BIN_DIR",
            "PIPX_BIN_DIR",
            "RVM_BIN",
            "PHP_PEAR_BIN_DIR",
            "NVM_BIN",
            "NVM_SYMLINK",
        ] {
            let paths = super::user_tool_paths_with(None, |key| {
                (key == name).then(|| directory.path().as_os_str().to_owned())
            });
            assert!(paths.contains(&directory.path().to_path_buf()), "{name}");
            assert!(!paths.contains(&directory.path().join("bin")), "{name}");
        }
    }

    #[test]
    fn gopath_list_adds_bin_to_each_nonempty_root() {
        let directory = tempfile::tempdir().expect("directory");
        let roots = [
            directory.path().join("go one"),
            directory.path().join("go two"),
        ];
        let value = std::env::join_paths([
            roots[0].as_path(),
            std::path::Path::new(""),
            roots[1].as_path(),
        ])
        .expect("GOPATH");
        let paths =
            super::user_tool_paths_with(None, |key| (key == "GOPATH").then(|| value.clone()));
        for root in roots {
            assert!(paths.contains(&root.join("bin")));
        }
        assert!(!paths.contains(&std::path::PathBuf::from("bin")));
    }

    #[test]
    fn npm_and_conda_prefixes_use_platform_executable_directories() {
        let directory = tempfile::tempdir().expect("directory");
        for name in ["NPM_CONFIG_PREFIX", "CONDA_PREFIX"] {
            let paths = super::user_tool_paths_with(None, |key| {
                (key == name).then(|| directory.path().as_os_str().to_owned())
            });
            if cfg!(windows) {
                assert!(paths.contains(&directory.path().to_path_buf()));
                assert!(!paths.contains(&directory.path().join("bin")));
                if name == "CONDA_PREFIX" {
                    assert!(paths.contains(&directory.path().join("Scripts")));
                    assert!(paths.contains(&directory.path().join("Library/bin")));
                }
            } else {
                assert!(paths.contains(&directory.path().join("bin")));
                assert!(!paths.contains(&directory.path().to_path_buf()));
            }
        }
    }

    #[test]
    fn empty_tool_directory_variables_do_not_add_relative_fallbacks() {
        let unset = super::user_tool_paths_with(None, |_| None);
        let empty = super::user_tool_paths_with(None, |_| Some(std::ffi::OsString::new()));
        assert_eq!(empty, unset);
    }

    #[cfg(unix)]
    #[test]
    fn fnm_discovery_runs_pnpm_with_its_node_without_shell_initialization() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let home = tempfile::tempdir().expect("home");
        let root = home.path().join(".local/share/fnm");
        let installation = root.join("node-versions/v24.10.0/installation");
        let bin = installation.join("bin");
        std::fs::create_dir_all(&bin).expect("runtime bin");
        std::fs::create_dir_all(root.join("aliases")).expect("aliases");
        symlink(&installation, root.join("aliases/default")).expect("default alias");
        for (name, script) in [
            ("node", "#!/bin/sh\nprintf 'fnm-node-ok'\n"),
            ("pnpm", "#!/bin/sh\nexec node\n"),
        ] {
            let tool = bin.join(name);
            std::fs::write(&tool, script).expect("tool");
            std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o755))
                .expect("executable tool");
        }
        let paths = super::user_tool_paths_with(Some(home.path().to_path_buf()), |_| None);
        let path = super::format_path_entries(paths.into_iter().filter(|path| path.is_dir()));
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "pnpm --version"])
            .env("PATH", path)
            .output()
            .expect("non-login shell");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"fnm-node-ok");
    }

    #[cfg(unix)]
    #[test]
    fn fnm_custom_roots_and_multishell_use_runtime_bin_directories() {
        let directory = tempfile::tempdir().expect("directory");
        for (name, suffix) in [("FNM_DIR", ""), ("XDG_DATA_HOME", "fnm")] {
            let root = directory.path().join(suffix);
            let bin = root.join("node-versions/v24.10.0/installation/bin");
            std::fs::create_dir_all(&bin).expect("runtime");
            let paths = super::user_tool_paths_with(None, |key| {
                (key == name).then(|| directory.path().as_os_str().to_owned())
            });
            assert!(paths.contains(&bin), "{name}");
        }
        let paths = super::user_tool_paths_with(None, |key| {
            (key == "FNM_MULTISHELL_PATH").then(|| directory.path().as_os_str().to_owned())
        });
        assert_eq!(paths.first(), Some(&directory.path().join("bin")));
        assert!(!paths.contains(&directory.path().to_path_buf()));
    }

    #[cfg(unix)]
    #[test]
    fn fnm_default_alias_prevents_cross_version_tool_fallback() {
        let root = tempfile::tempdir().expect("root");
        let default = root.path().join("aliases/default/bin");
        let other = root.path().join("node-versions/v25.0.0/installation/bin");
        std::fs::create_dir_all(&default).expect("default");
        std::fs::create_dir_all(&other).expect("other");
        let mut paths = Vec::new();
        super::append_fnm_paths(&mut paths, root.path());
        assert_eq!(paths.first(), Some(&default));
        assert!(!paths.contains(&other));
    }

    #[cfg(unix)]
    #[test]
    fn fnm_version_fallback_is_bounded_and_sorted_without_a_default() {
        let root = tempfile::tempdir().expect("root");
        for version in ["v24.10.0", "v20.0.0"] {
            std::fs::create_dir_all(
                root.path()
                    .join("node-versions")
                    .join(version)
                    .join("installation/bin"),
            )
            .expect("runtime");
        }
        let mut paths = Vec::new();
        super::append_fnm_paths(&mut paths, root.path());
        let existing = paths
            .into_iter()
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        assert_eq!(
            existing[0],
            root.path().join("node-versions/v20.0.0/installation/bin")
        );
        assert_eq!(
            existing[1],
            root.path().join("node-versions/v24.10.0/installation/bin")
        );

        for version in 0..140 {
            std::fs::create_dir_all(
                root.path()
                    .join("node-versions")
                    .join(format!("v{version}")),
            )
            .expect("version");
        }
        let mut paths = Vec::new();
        super::append_fnm_paths(&mut paths, root.path());
        assert_eq!(paths.len(), 128 + 2 + 1);
    }

    #[test]
    fn inherited_path_precedes_discovered_fallbacks() {
        let original = std::env::var("PATH").unwrap_or_default();
        let expected = super::normalized_path_entries(&original);
        let environment = super::base_child_environment();
        let path = &environment
            .iter()
            .find(|(key, _)| key == "PATH")
            .expect("PATH")
            .1;
        assert!(super::normalized_path_entries(path).starts_with(&expected));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fnm_macos_application_support_root_is_discovered() {
        let home = tempfile::tempdir().expect("home");
        let bin = home
            .path()
            .join("Library/Application Support/fnm/node-versions/v24.10.0/installation/bin");
        std::fs::create_dir_all(&bin).expect("runtime");
        let paths = super::user_tool_paths_with(Some(home.path().to_path_buf()), |_| None);
        assert!(paths.contains(&bin));
    }

    #[cfg(windows)]
    #[test]
    fn fnm_windows_runtime_and_multishell_do_not_add_bin() {
        let root = tempfile::tempdir().expect("root");
        let installation = root.path().join("node-versions/v24.10.0/installation");
        std::fs::create_dir_all(&installation).expect("runtime");
        let paths = super::user_tool_paths_with(None, |key| {
            matches!(key, "FNM_DIR" | "FNM_MULTISHELL_PATH")
                .then(|| root.path().as_os_str().to_owned())
        });
        assert_eq!(paths.first(), Some(&root.path().to_path_buf()));
        assert!(paths.contains(&installation));
        assert!(!paths.contains(&installation.join("bin")));
    }

    #[tokio::test]
    async fn child_path_wait_observes_the_completed_probe_before_resolution() {
        let (release, wait) = std::sync::mpsc::channel();
        let environment = super::start_child_environment(
            vec![("PATH".to_string(), "/initial/bin".to_string())],
            move || wait.recv().ok(),
        );
        let budget = std::time::Duration::from_millis(10);
        assert!(!super::wait_for_path_probe(&environment, budget).await);
        assert_eq!(
            environment.values.read().expect("environment")[0].1,
            "/initial/bin"
        );

        release
            .send("/late/bin".to_string())
            .expect("release probe");
        assert!(super::wait_for_path_probe(&environment, std::time::Duration::from_secs(1)).await);
        let values = environment.values.read().expect("settled environment");
        assert_eq!(values[0].1, merge_paths("/initial/bin", "/late/bin"));
        drop(values);
        // Completion is retained even when the probe finishes before a subscriber arrives.
        assert!(super::wait_for_path_probe(&environment, budget).await);
    }

    #[tokio::test]
    async fn child_path_failed_probe_settles_and_keeps_the_base_environment() {
        let initial = vec![("PATH".to_string(), "base-path".to_string())];
        let environment = super::start_child_environment(initial.clone(), || None);
        assert!(super::wait_for_path_probe(&environment, std::time::Duration::from_secs(1)).await);
        assert_eq!(*environment.values.read().expect("environment"), initial);
    }

    #[cfg(unix)]
    #[test]
    fn bounded_probe_does_not_wait_for_stdin_or_sleeping_startup_code() {
        assert_eq!(
            run_path_probe("/bin/sh", &["-c", "sleep 10; echo bad"]),
            None
        );
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
    fn user_tool_paths_include_linux_pnpm_default_directory() {
        let home = dirs::home_dir().expect("test home directory");
        assert!(user_tool_paths().contains(&home.join(".local/share/pnpm")));
    }

    #[cfg(unix)]
    #[test]
    fn merged_path_absolutizes_and_deduplicates_entries() {
        let directory = tempfile::tempdir().expect("path directory");
        let nested = directory.path().join("nested");
        std::fs::create_dir_all(&nested).expect("nested directory");
        let relative = format!("{}/../nested", nested.display());
        let merged = merge_paths(&relative, &nested.to_string_lossy());
        let entries = merged.split(':').collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], nested.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn child_environment_is_available_without_shell_probe_success() {
        let environment = super::get_child_environment();
        assert!(environment.iter().any(|(key, value)| {
            key == "PATH"
                && !value.is_empty()
                && value.split(':').all(|entry| entry.starts_with('/'))
        }));
    }

    #[cfg(unix)]
    #[test]
    fn merged_path_prioritizes_login_shell_entries_without_duplicates() {
        let merged = merge_paths("/usr/bin:/bin:/usr/bin", "/custom/bin:/usr/bin");
        assert_eq!(merged, "/custom/bin:/usr/bin");
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
