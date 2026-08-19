use crate::environment::{
    get_available_shells, get_default_shell, get_terminal_environment, ShellDescriptor,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 200;
const WORKFLOW_WINDOW_LABEL: &str = "workflow";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TerminalShell {
    pub name: String,
    pub path: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TerminalSessionMetadata {
    pub session_id: String,
    pub shell_name: String,
    pub shell_path: String,
    pub cwd: String,
    pub alive: bool,
}

#[derive(Clone, Debug, Serialize)]
struct TerminalOutputEvent {
    session_id: String,
    data_base64: String,
}

#[derive(Clone, Debug, Serialize)]
struct TerminalExitEvent {
    session_id: String,
    exit_code: Option<u32>,
}

#[derive(Default)]
struct TerminalRegistry {
    sessions: HashMap<String, TerminalSession>,
}

struct TerminalResources {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
}

struct TerminalSession {
    metadata: TerminalSessionMetadata,
    // Natural process exit releases PTY resources immediately while retaining metadata so the
    // frontend can present an exited tab until the user explicitly closes it.
    resources: Option<TerminalResources>,
}

/// Owns all interactive PTY processes associated with the workflow window.
///
/// The manager deliberately has no dependency on `WorkflowManager`: these are direct user
/// terminals with the user's normal shell permissions, not AI shell-tool executions.
pub(crate) struct TerminalManager {
    app_handle: AppHandle,
    sessions: Arc<Mutex<TerminalRegistry>>,
}

impl TerminalManager {
    pub(crate) fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            sessions: Arc::new(Mutex::new(TerminalRegistry::default())),
        }
    }

    pub(crate) fn list_shells(&self) -> Vec<TerminalShell> {
        let default_path = get_default_shell().map(|shell| shell.path);
        get_available_shells()
            .into_iter()
            .map(|shell| TerminalShell {
                is_default: default_path.as_deref() == Some(shell.path.as_str()),
                name: shell.name,
                path: shell.path,
            })
            .collect()
    }

    pub(crate) fn list_sessions(&self) -> Vec<TerminalSessionMetadata> {
        self.sessions
            .lock()
            .sessions
            .values()
            .map(|session| session.metadata.clone())
            .collect()
    }

    pub(crate) fn create(
        &self,
        cwd_candidate: Option<&str>,
        shell_path: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<TerminalSessionMetadata, String> {
        let shell = self.select_shell(shell_path)?;
        let cwd = resolve_initial_cwd(cwd_candidate)?;
        let session_id = Uuid::new_v4().to_string();
        let size = validated_size(cols, rows)?;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|error| format!("terminal_pty_open_failed:{error}"))?;

        let mut command = build_shell_command(&shell);
        command.cwd(&cwd);
        for (key, value) in get_terminal_environment() {
            command.env(key, value);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        #[cfg(unix)]
        {
            command.env("PS1", "\\w > ");
            command.env("PROMPT", "%~ > ");
        }
        #[cfg(windows)]
        command.env("PROMPT", "$P$G");

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("terminal_spawn_failed:{error}"))?;
        #[cfg(unix)]
        let process_group = pair.master.process_group_leader();
        let mut writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                #[cfg(unix)]
                terminate_child_tree(&mut *child, process_group);
                #[cfg(windows)]
                terminate_child_tree(&mut *child);
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(format!("terminal_writer_failed:{error}"));
            }
        };
        install_session_prompt(&mut writer, &shell, &cwd);
        // Let the login shell apply its profile and prompt hook, then start the visible session
        // with a clean screen without writing anything to user configuration files.
        let clear_command = if cfg!(windows) { "cls\r\n" } else { "clear\n" };
        let _ = writer
            .write_all(clear_command.as_bytes())
            .and_then(|_| writer.flush());
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                #[cfg(unix)]
                terminate_child_tree(&mut *child, process_group);
                #[cfg(windows)]
                terminate_child_tree(&mut *child);
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(format!("terminal_reader_failed:{error}"));
            }
        };

        let metadata = TerminalSessionMetadata {
            session_id: session_id.clone(),
            shell_name: shell.name,
            shell_path: shell.path,
            cwd: {
                #[cfg(windows)]
                {
                    windows_display_path(&cwd)
                }
                #[cfg(not(windows))]
                {
                    cwd.to_string_lossy().into_owned()
                }
            },
            alive: true,
        };
        let reader_metadata = metadata.clone();
        self.sessions.lock().sessions.insert(
            session_id.clone(),
            TerminalSession {
                metadata: reader_metadata.clone(),
                resources: Some(TerminalResources {
                    writer,
                    master: pair.master,
                    child,
                    #[cfg(unix)]
                    process_group,
                }),
            },
        );
        self.spawn_reader(session_id, reader, Arc::clone(&self.sessions));

        Ok(reader_metadata)
    }

    pub(crate) fn write(&self, session_id: &str, input: &str) -> Result<(), String> {
        let mut registry = self.sessions.lock();
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| "terminal_session_not_found".to_string())?;
        if !session.metadata.alive {
            return Err("terminal_session_exited".to_string());
        }

        let resources = session
            .resources
            .as_mut()
            .ok_or_else(|| "terminal_session_exited".to_string())?;
        resources
            .writer
            .write_all(input.as_bytes())
            .and_then(|_| resources.writer.flush())
            .map_err(|error| format!("terminal_write_failed:{error}"))
    }

    pub(crate) fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let size = validated_size(Some(cols), Some(rows))?;
        let registry = self.sessions.lock();
        let session = registry
            .sessions
            .get(session_id)
            .ok_or_else(|| "terminal_session_not_found".to_string())?;
        let resources = session
            .resources
            .as_ref()
            .ok_or_else(|| "terminal_session_exited".to_string())?;
        resources
            .master
            .resize(size)
            .map_err(|error| format!("terminal_resize_failed:{error}"))
    }

    /// Closes a session. Repeating a close is harmless so UI races cannot leak a PTY.
    pub(crate) fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self.sessions.lock().sessions.remove(session_id);
        if let Some(mut session) = session {
            session.metadata.alive = false;
            terminate_terminal_resources(session.resources.take());
        }
        Ok(())
    }

    pub(crate) fn cleanup_all(&self) {
        let session_ids = self
            .sessions
            .lock()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for session_id in session_ids {
            let _ = self.close(&session_id);
        }
    }

    fn select_shell(&self, shell_path: Option<&str>) -> Result<ShellDescriptor, String> {
        let shells = get_available_shells();
        let shell = match shell_path {
            Some(path) => shells.into_iter().find(|shell| shell.path == path),
            None => get_default_shell(),
        };
        shell.ok_or_else(|| "terminal_shell_unavailable".to_string())
    }

    fn spawn_reader(
        &self,
        session_id: String,
        mut reader: Box<dyn Read + Send>,
        sessions: Arc<Mutex<TerminalRegistry>>,
    ) {
        let app_handle = self.app_handle.clone();
        std::thread::spawn(move || {
            let mut buffer = vec![0_u8; 8192];
            let reached_eof = loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break true,
                    Ok(read) => {
                        let event = TerminalOutputEvent {
                            session_id: session_id.clone(),
                            data_base64: BASE64.encode(&buffer[..read]),
                        };
                        if app_handle
                            .emit_to(WORKFLOW_WINDOW_LABEL, "terminal://output", event)
                            .is_err()
                        {
                            break false;
                        }
                    }
                    Err(_) => break false,
                }
            };
            if reached_eof {
                reap_exited_session(&sessions, &session_id);
            } else {
                // A reader I/O error or output-event delivery failure leaves the UI unable to
                // safely control this session. Remove and terminate it before reporting exit.
                abort_terminal_session(&sessions, &session_id);
            }
            let _ = app_handle.emit_to(
                WORKFLOW_WINDOW_LABEL,
                "terminal://exit",
                TerminalExitEvent {
                    session_id,
                    exit_code: None,
                },
            );
        });
    }
}

fn abort_terminal_session(sessions: &Arc<Mutex<TerminalRegistry>>, session_id: &str) {
    let resources = sessions
        .lock()
        .sessions
        .remove(session_id)
        .and_then(|mut session| {
            session.metadata.alive = false;
            session.resources.take()
        });

    terminate_terminal_resources(resources);
}

fn reap_exited_session(sessions: &Arc<Mutex<TerminalRegistry>>, session_id: &str) {
    let resources = {
        let mut registry = sessions.lock();
        let Some(session) = registry.sessions.get_mut(session_id) else {
            return;
        };
        session.metadata.alive = false;
        session.resources.take()
    };

    reap_terminal_resources(resources);
}

#[cfg(test)]
fn reap_terminal_session(session: &mut TerminalSession) {
    session.metadata.alive = false;
    reap_terminal_resources(session.resources.take());
}

fn terminate_terminal_resources(resources: Option<TerminalResources>) {
    if let Some(mut resources) = resources {
        #[cfg(unix)]
        terminate_child_tree(&mut *resources.child, resources.process_group);
        #[cfg(windows)]
        terminate_child_tree(&mut *resources.child);
        #[cfg(not(any(unix, windows)))]
        {
            let _ = resources.child.kill();
            let _ = resources.child.wait();
        }
    }
}

fn reap_terminal_resources(resources: Option<TerminalResources>) {
    if let Some(mut resources) = resources {
        // EOF from the PTY means the child closed its terminal. Reap it now so an exited tab
        // keeps only UI metadata rather than a child handle, master PTY, or zombie process.
        let _ = resources.child.wait();
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.cleanup_all();
    }
}

fn build_shell_command(shell: &ShellDescriptor) -> CommandBuilder {
    let mut command = CommandBuilder::new(&shell.path);

    #[cfg(unix)]
    {
        match shell.name.as_str() {
            // fish uses a different startup contract and does not accept POSIX login flags.
            "fish" => command.arg("-i"),
            // POSIX-family shells support the same interactive login launch used by bash/zsh.
            _ => {
                command.arg("-l");
                command.arg("-i");
            }
        };
    }

    #[cfg(windows)]
    {
        if shell.name.eq_ignore_ascii_case("cmd.exe") {
            command.arg("/K");
        } else {
            command.arg("-NoLogo");
        }
    }

    command
}

#[cfg(unix)]
fn terminate_child_tree(child: &mut dyn portable_pty::Child, process_group: Option<libc::pid_t>) {
    let root_process = child
        .process_id()
        .map(|process_id| process_id as libc::pid_t);
    let mut tracked_processes = root_process
        .map(terminal_process_tree_members)
        .unwrap_or_default();
    if let Some(root_process) = root_process {
        tracked_processes.push(root_process);
    }

    if let Some(process_group) = process_group.filter(|group| *group > 0) {
        let session_id = terminal_session_id(process_group);
        tracked_processes.extend(terminal_session_members(session_id));
        // portable-pty usually creates an isolated session for the controlling PTY. Terminate all
        // members, while also retaining the shell descendant tree for PTY backends that do not.
        signal_processes(&tracked_processes, libc::SIGHUP);
        signal_processes(&tracked_processes, libc::SIGTERM);
        unsafe {
            libc::kill(-process_group, libc::SIGHUP);
            libc::kill(-process_group, libc::SIGTERM);
        }
    } else {
        signal_processes(&tracked_processes, libc::SIGHUP);
        signal_processes(&tracked_processes, libc::SIGTERM);
    }
    let _ = child.kill();
    let _ = child.wait();

    // A direct user shell may launch a background process that deliberately ignores HUP/TERM.
    // It can be reparented when the shell exits, so retain the pre-close process tree and force
    // kill every surviving member rather than relying on a second descendant-tree lookup.
    for _ in 0..5 {
        let surviving = tracked_processes
            .iter()
            .copied()
            .filter(|process_id| process_is_alive(*process_id))
            .collect::<Vec<_>>();
        if surviving.is_empty() {
            break;
        }
        signal_processes(&surviving, libc::SIGKILL);
        if let Some(process_group) = process_group.filter(|group| *group > 0) {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn signal_processes(process_ids: &[libc::pid_t], signal: libc::c_int) {
    for process_id in process_ids {
        if *process_id > 0 {
            unsafe {
                libc::kill(*process_id, signal);
            }
        }
    }
}

#[cfg(unix)]
fn process_is_alive(process_id: libc::pid_t) -> bool {
    process_id > 0 && unsafe { libc::kill(process_id, 0) == 0 }
}

#[cfg(unix)]
fn terminal_process_tree_members(root_process: libc::pid_t) -> Vec<libc::pid_t> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };

    let processes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse::<libc::pid_t>().ok()?,
                fields.next()?.parse::<libc::pid_t>().ok()?,
            ))
        })
        .collect::<Vec<_>>();
    let mut members = Vec::new();
    let mut parents = vec![root_process];
    while let Some(parent) = parents.pop() {
        for (process_id, process_parent) in &processes {
            if *process_parent == parent && !members.contains(process_id) {
                members.push(*process_id);
                parents.push(*process_id);
            }
        }
    }
    members
}

#[cfg(unix)]
fn terminal_session_id(process_group: libc::pid_t) -> libc::pid_t {
    let session_id = unsafe { libc::getsid(process_group) };
    (session_id > 0)
        .then_some(session_id)
        .unwrap_or(process_group)
}

#[cfg(target_os = "macos")]
fn terminal_session_members(session_id: libc::pid_t) -> Vec<libc::pid_t> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,sess="])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let process_id = fields.next()?.parse::<libc::pid_t>().ok()?;
            let process_session = fields.next()?.parse::<libc::pid_t>().ok()?;
            (process_session == session_id && process_id != std::process::id() as libc::pid_t)
                .then_some(process_id)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn terminal_session_members(session_id: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let process_id = entry
                .file_name()
                .to_string_lossy()
                .parse::<libc::pid_t>()
                .ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let fields = stat
                .rsplit_once(") ")?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            let process_session = fields.get(3)?.parse::<libc::pid_t>().ok()?;
            (process_session == session_id && process_id != std::process::id() as libc::pid_t)
                .then_some(process_id)
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn terminal_session_members(_: i32) -> Vec<i32> {
    Vec::new()
}

#[cfg(windows)]
fn terminate_child_tree(child: &mut dyn portable_pty::Child) {
    if let Some(process_id) = child.process_id() {
        let mut command = std::process::Command::new("taskkill");
        command
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
        let _ = command.output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn windows_display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}

#[cfg(windows)]
fn windows_prompt_bootstrap(is_cmd: bool, cwd: &Path) -> String {
    let cwd = windows_display_path(cwd);
    if is_cmd {
        format!(
            "cd /d \"{}\"\r\nPROMPT \u{1b}]7;file://%COMPUTERNAME%/$P\u{7}$P$G\r\n",
            cwd.replace('"', "\"\"")
        )
    } else {
        format!(
            "Set-Location -LiteralPath '{}'; function prompt {{ $path = $ExecutionContext.SessionState.Path.CurrentFileSystemLocation.ProviderPath -replace '^Microsoft\\.PowerShell\\.Core\\\\FileSystem::','' -replace '^\\\\\\\\\\?\\\\',''; $uriPath = $path.Replace('\\','/'); Write-Host -NoNewline ([char]27 + \"]7;file://$env:COMPUTERNAME/$uriPath\" + [char]7); \"$path > \" }}\r\n",
            cwd.replace('\'', "''")
        )
    }
}

fn install_session_prompt(writer: &mut (dyn Write + Send), shell: &ShellDescriptor, cwd: &Path) {
    #[cfg(unix)]
    {
        let _ = cwd;
        let command = match shell.name.as_str() {
            "zsh" => "autoload -Uz add-zsh-hook; _cs_terminal_osc7(){ print -n $'\\e]7;file://'${HOST:-localhost}${PWD}$'\\a'; }; add-zsh-hook precmd _cs_terminal_osc7; PROMPT='%~ > '\n",
            "bash" => "__cs_terminal_osc7(){ printf '\\033]7;file://%s%s\\007' \"${HOSTNAME:-localhost}\" \"$PWD\"; }; PROMPT_COMMAND=\"__cs_terminal_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; PS1='\\w > '\n",
            "fish" => "function __cs_terminal_osc7 --on-event fish_prompt; printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; end; function fish_prompt; set -l display_path \"$PWD\"; if test \"$PWD\" = \"$HOME\"; set display_path '~'; else; set display_path (string replace -- \"$HOME/\" '~/' \"$PWD\"); end; printf '%s > ' \"$display_path\"; end\n",
            _ => "PS1='\\w > '\n",
        };
        let _ = writer
            .write_all(command.as_bytes())
            .and_then(|_| writer.flush());
    }

    #[cfg(windows)]
    {
        let command = windows_prompt_bootstrap(shell.name.eq_ignore_ascii_case("cmd.exe"), cwd);
        let _ = writer
            .write_all(command.as_bytes())
            .and_then(|_| writer.flush());
    }
}

fn resolve_initial_cwd(candidate: Option<&str>) -> Result<PathBuf, String> {
    if let Some(candidate) = candidate {
        let path = PathBuf::from(candidate);
        if path.is_dir() {
            return path
                .canonicalize()
                .map_err(|error| format!("terminal_cwd_invalid:{error}"));
        }
    }

    if let Some(home) = dirs::home_dir().filter(|path| path.is_dir()) {
        return home
            .canonicalize()
            .map_err(|error| format!("terminal_home_invalid:{error}"));
    }

    std::env::current_dir().map_err(|error| format!("terminal_cwd_unavailable:{error}"))
}

fn validated_size(cols: Option<u16>, rows: Option<u16>) -> Result<PtySize, String> {
    let cols = cols.unwrap_or(DEFAULT_COLS);
    let rows = rows.unwrap_or(DEFAULT_ROWS);
    if cols == 0 || rows == 0 || cols > MAX_COLS || rows > MAX_ROWS {
        return Err("terminal_size_invalid".to_string());
    }

    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        build_shell_command, install_session_prompt, resolve_initial_cwd, validated_size,
        ShellDescriptor, DEFAULT_COLS, DEFAULT_ROWS,
    };

    #[test]
    fn validates_terminal_dimensions() {
        let size = validated_size(None, None).expect("default terminal size should be valid");
        assert_eq!(size.cols, DEFAULT_COLS);
        assert_eq!(size.rows, DEFAULT_ROWS);
        assert!(validated_size(Some(0), Some(10)).is_err());
        assert!(validated_size(Some(501), Some(10)).is_err());
    }

    #[test]
    fn invalid_cwd_falls_back_to_a_real_directory() {
        let cwd = resolve_initial_cwd(Some("/definitely/not/a/workflow/directory"))
            .expect("a fallback working directory should exist");
        assert!(cwd.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_launch_contracts_cover_posix_and_fish() {
        let bash = ShellDescriptor {
            name: "bash".to_string(),
            path: "/bin/bash".to_string(),
        };
        let bash_command = build_shell_command(&bash);
        let bash_arguments = bash_command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(bash_arguments, vec!["/bin/bash", "-l", "-i"]);

        let fish = ShellDescriptor {
            name: "fish".to_string(),
            path: "/usr/bin/fish".to_string(),
        };
        let fish_command = build_shell_command(&fish);
        let fish_arguments = fish_command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(fish_arguments, vec!["/usr/bin/fish", "-i"]);

        let mut bootstrap = Vec::new();
        install_session_prompt(&mut bootstrap, &bash, Path::new("/workspace"));
        let bootstrap = String::from_utf8(bootstrap).expect("bootstrap must be UTF-8");
        assert!(bootstrap.contains("PROMPT_COMMAND"));
        assert!(bootstrap.contains("PS1='\\w > '"));
        assert!(bootstrap.contains("]7;file://"));

        let mut fish_bootstrap = Vec::new();
        install_session_prompt(&mut fish_bootstrap, &fish, Path::new("/workspace"));
        let fish_bootstrap =
            String::from_utf8(fish_bootstrap).expect("fish bootstrap must be UTF-8");
        assert!(fish_bootstrap.contains("fish_prompt"));
        assert!(fish_bootstrap.contains("]7;file://"));
        assert!(fish_bootstrap.contains("if test \"$PWD\" = \"$HOME\""));
        assert!(fish_bootstrap.contains("string replace -- \"$HOME/\" '~/' \"$PWD\""));
        assert!(bootstrap.contains("PS1='\\w > '"));

        let zsh = ShellDescriptor {
            name: "zsh".to_string(),
            path: "/bin/zsh".to_string(),
        };
        let mut zsh_bootstrap = Vec::new();
        install_session_prompt(&mut zsh_bootstrap, &zsh, Path::new("/workspace"));
        let zsh_bootstrap = String::from_utf8(zsh_bootstrap).expect("zsh bootstrap must be UTF-8");
        assert!(zsh_bootstrap.contains("PROMPT='%~ > '"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_prompt_bootstraps_emit_valid_file_uris() {
        let cmd = super::windows_prompt_bootstrap(true, Path::new(r"C:\repo"));
        let powershell = super::windows_prompt_bootstrap(false, Path::new(r"\\?\C:\repo"));
        assert!(cmd.contains("cd /d \"C:\\repo\""));
        assert!(cmd.contains("file://%COMPUTERNAME%/$P"));
        assert!(powershell.contains("Set-Location -LiteralPath 'C:\\repo'"));
        assert!(powershell.contains("file://$env:COMPUTERNAME/$uriPath"));
        assert!(powershell.contains("CurrentFileSystemLocation.ProviderPath"));
        assert!(powershell.contains("Microsoft\\\\.PowerShell"));
        assert!(powershell.contains("[char]27"));
        assert!(!powershell.contains("`e]7"));
    }

    #[cfg(unix)]
    #[test]
    fn natural_terminal_exit_releases_managed_pty_resources() {
        use portable_pty::native_pty_system;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for natural-exit test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for natural-exit test");
        let writer = pair
            .master
            .take_writer()
            .expect("PTY writer should be available");
        let process_group = pair.master.process_group_leader();
        let metadata = super::TerminalSessionMetadata {
            session_id: "natural-exit".to_string(),
            shell_name: "sh".to_string(),
            shell_path: "/bin/sh".to_string(),
            cwd: "/".to_string(),
            alive: true,
        };
        let mut session = super::TerminalSession {
            metadata,
            resources: Some(super::TerminalResources {
                writer,
                master: pair.master,
                child,
                process_group,
            }),
        };

        super::reap_terminal_session(&mut session);
        assert!(!session.metadata.alive);
        assert!(session.resources.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reader_exit_reaping_retains_only_exited_tab_metadata() {
        use portable_pty::native_pty_system;
        use std::collections::HashMap;
        use std::sync::Arc;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for reader-exit test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for reader-exit test");
        let writer = pair
            .master
            .take_writer()
            .expect("PTY writer should be available");
        let process_group = pair.master.process_group_leader();
        let session_id = "reader-exit".to_string();
        let metadata = super::TerminalSessionMetadata {
            session_id: session_id.clone(),
            shell_name: "sh".to_string(),
            shell_path: "/bin/sh".to_string(),
            cwd: "/".to_string(),
            alive: true,
        };
        let sessions = Arc::new(super::Mutex::new(super::TerminalRegistry {
            sessions: HashMap::from([(
                session_id.clone(),
                super::TerminalSession {
                    metadata,
                    resources: Some(super::TerminalResources {
                        writer,
                        master: pair.master,
                        child,
                        process_group,
                    }),
                },
            )]),
        }));

        super::reap_exited_session(&sessions, &session_id);
        let registry = sessions.lock();
        let session = registry
            .sessions
            .get(&session_id)
            .expect("exited tab metadata retained");
        assert!(!session.metadata.alive);
        assert!(session.resources.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reader_failure_aborts_and_removes_the_live_session() {
        use portable_pty::native_pty_system;
        use std::collections::HashMap;
        use std::sync::Arc;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for reader-failure test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.args(["-c", "sleep 30"]);
        let child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for reader-failure test");
        let writer = pair
            .master
            .take_writer()
            .expect("PTY writer should be available");
        let process_group = pair.master.process_group_leader();
        let session_id = "reader-failure".to_string();
        let metadata = super::TerminalSessionMetadata {
            session_id: session_id.clone(),
            shell_name: "sh".to_string(),
            shell_path: "/bin/sh".to_string(),
            cwd: "/".to_string(),
            alive: true,
        };
        let sessions = Arc::new(super::Mutex::new(super::TerminalRegistry {
            sessions: HashMap::from([(
                session_id.clone(),
                super::TerminalSession {
                    metadata,
                    resources: Some(super::TerminalResources {
                        writer,
                        master: pair.master,
                        child,
                        process_group,
                    }),
                },
            )]),
        }));

        super::abort_terminal_session(&sessions, &session_id);
        assert!(
            !sessions.lock().sessions.contains_key(&session_id),
            "reader failures must not leave an unreachable backend session"
        );
        if let Some(process_group) = process_group {
            assert!(
                super::terminal_session_members(process_group).is_empty(),
                "reader failure leaked terminal processes"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn terminal_pty_preserves_ansi_bytes_and_accepts_resize() {
        use portable_pty::native_pty_system;
        use std::io::Read;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for ANSI routing test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf 'first\\rsecond\\033[31mred\\033[0m'"]);
        let mut child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for ANSI routing test");
        pair.master
            .resize(super::PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY resize should succeed");
        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("PTY reader should be available");
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .expect("PTY output should be readable");
        let ansi = b"\x1b[31mred\x1b[0m";
        assert!(output.windows(ansi.len()).any(|window| window == ansi));
        assert!(output
            .windows(b"first\rsecond".len())
            .any(|window| window == b"first\rsecond"));
        assert!(child.wait().is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn pty_cleanup_terminates_the_dedicated_terminal_process_group() {
        use portable_pty::native_pty_system;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for terminal cleanup test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let mut child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for terminal cleanup test");
        let process_group = pair.master.process_group_leader();
        assert!(process_group.is_some());

        super::terminate_child_tree(&mut *child, process_group);
        assert!(child.wait().is_ok());
        let remaining =
            super::terminal_session_members(process_group.expect("PTY session leader id"));
        assert!(
            remaining.is_empty(),
            "terminal session leaked processes: {remaining:?}"
        );
    }
    #[cfg(unix)]
    #[test]
    fn pty_cleanup_force_kills_signal_ignoring_background_jobs() {
        use portable_pty::native_pty_system;
        use std::fs;

        let marker_dir = tempfile::tempdir().expect("temporary marker directory");
        let marker = marker_dir.path().join("background-job.pid");
        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY should open for force-cleanup test");
        let mut command = super::CommandBuilder::new("/bin/sh");
        command.arg("-c");
        command.arg(format!(
            "trap '' HUP TERM; (trap '' HUP TERM; exec sleep 30) & echo $! > {}; wait",
            marker.display()
        ));
        let mut child = pair
            .slave
            .spawn_command(command)
            .expect("PTY child should start for force-cleanup test");
        let process_group = pair.master.process_group_leader();
        let session_id = super::terminal_session_id(process_group.expect("PTY session leader id"));
        let shell_pid = child
            .process_id()
            .map(|process_id| process_id as libc::pid_t)
            .expect("PTY shell process id");

        let background_pid = (0..10)
            .find_map(|_| {
                fs::read_to_string(&marker)
                    .ok()
                    .and_then(|value| value.trim().parse::<libc::pid_t>().ok())
                    .filter(|process_id| unsafe { libc::kill(*process_id, 0) == 0 })
                    .or_else(|| {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        None
                    })
            })
            .expect("signal-ignoring background job should start");
        assert!(
            super::terminal_process_tree_members(shell_pid).contains(&background_pid),
            "background job must remain in the shell process tree before cleanup"
        );

        super::terminate_child_tree(&mut *child, process_group);
        assert!(child.wait().is_ok());
        assert!(
            unsafe { libc::kill(background_pid, 0) != 0 },
            "force cleanup left signal-ignoring background process {background_pid} alive"
        );
        let remaining = super::terminal_session_members(session_id);
        assert!(
            remaining.is_empty(),
            "force cleanup leaked signal-ignoring terminal processes: {remaining:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn zsh_bootstrap_reports_cwd_with_osc7() {
        let shell = ShellDescriptor {
            name: "zsh".to_string(),
            path: "/bin/zsh".to_string(),
        };
        let mut bootstrap = Vec::new();
        install_session_prompt(&mut bootstrap, &shell, Path::new("/workspace"));
        let bootstrap = String::from_utf8(bootstrap).expect("bootstrap must be UTF-8");
        assert!(bootstrap.contains("add-zsh-hook"));
        assert!(bootstrap.contains("PROMPT='%~ > '"));
        assert!(bootstrap.contains("]7;file://"));
    }
}
