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
use std::path::PathBuf;
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

struct TerminalSession {
    metadata: TerminalSessionMetadata,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
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
        let process_group = pair.master.process_group_leader();
        let mut writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                terminate_child_tree(&mut *child, process_group);
                return Err(format!("terminal_writer_failed:{error}"));
            }
        };
        install_session_prompt(&mut writer, &shell);
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                terminate_child_tree(&mut *child, process_group);
                return Err(format!("terminal_reader_failed:{error}"));
            }
        };

        let metadata = TerminalSessionMetadata {
            session_id: session_id.clone(),
            shell_name: shell.name,
            shell_path: shell.path,
            cwd: cwd.to_string_lossy().into_owned(),
            alive: true,
        };
        let reader_metadata = metadata.clone();
        self.sessions.lock().sessions.insert(
            session_id.clone(),
            TerminalSession {
                metadata: reader_metadata.clone(),
                writer,
                master: pair.master,
                child,
                #[cfg(unix)]
                process_group,
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

        session
            .writer
            .write_all(input.as_bytes())
            .and_then(|_| session.writer.flush())
            .map_err(|error| format!("terminal_write_failed:{error}"))
    }

    pub(crate) fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let size = validated_size(Some(cols), Some(rows))?;
        let registry = self.sessions.lock();
        let session = registry
            .sessions
            .get(session_id)
            .ok_or_else(|| "terminal_session_not_found".to_string())?;
        session
            .master
            .resize(size)
            .map_err(|error| format!("terminal_resize_failed:{error}"))
    }

    /// Closes a session. Repeating a close is harmless so UI races cannot leak a PTY.
    pub(crate) fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self.sessions.lock().sessions.remove(session_id);
        if let Some(mut session) = session {
            session.metadata.alive = false;
            #[cfg(unix)]
            terminate_child_tree(&mut *session.child, session.process_group);
            #[cfg(windows)]
            terminate_child_tree(&mut *session.child);
            #[cfg(not(any(unix, windows)))]
            {
                let _ = session.child.kill();
                let _ = session.child.wait();
            }
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

    pub(crate) fn reset_workflow_window(&self) {
        self.cleanup_all();
        let _ = self
            .app_handle
            .emit_to(WORKFLOW_WINDOW_LABEL, "terminal://reset", ());
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
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let event = TerminalOutputEvent {
                            session_id: session_id.clone(),
                            data_base64: BASE64.encode(&buffer[..read]),
                        };
                        if app_handle
                            .emit_to(WORKFLOW_WINDOW_LABEL, "terminal://output", event)
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if let Some(session) = sessions.lock().sessions.get_mut(&session_id) {
                session.metadata.alive = false;
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

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.cleanup_all();
    }
}

fn build_shell_command(shell: &ShellDescriptor) -> CommandBuilder {
    let mut command = CommandBuilder::new(&shell.path);

    #[cfg(unix)]
    {
        command.arg("-l");
        command.arg("-i");
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
    if let Some(process_group) = process_group.filter(|group| *group > 0) {
        // portable-pty creates an isolated session for the controlling PTY. Terminate all
        // members so interactive-shell background jobs in separate process groups cannot leak.
        for process_id in terminal_session_members(process_group) {
            unsafe {
                libc::kill(process_id, libc::SIGHUP);
                libc::kill(process_id, libc::SIGTERM);
            }
        }
        // Preserve a process-group fallback if the platform cannot enumerate a session member.
        unsafe {
            libc::kill(-process_group, libc::SIGHUP);
            libc::kill(-process_group, libc::SIGTERM);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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
fn terminal_session_members(_: libc::pid_t) -> Vec<libc::pid_t> {
    Vec::new()
}

#[cfg(windows)]
fn terminate_child_tree(child: &mut dyn portable_pty::Child) {
    if let Some(process_id) = child.process_id() {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn install_session_prompt(writer: &mut (dyn Write + Send), shell: &ShellDescriptor) {
    #[cfg(unix)]
    {
        let command = match shell.name.as_str() {
            "zsh" => "autoload -Uz add-zsh-hook; _cs_terminal_osc7(){ print -n $'\\e]7;file://'${HOST:-localhost}${PWD}$'\\a'; }; add-zsh-hook precmd _cs_terminal_osc7; PROMPT='%~ > '\n",
            "bash" => "__cs_terminal_osc7(){ printf '\\033]7;file://%s%s\\007' \"${HOSTNAME:-localhost}\" \"$PWD\"; }; PROMPT_COMMAND=\"__cs_terminal_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; PS1='\\w > '\n",
            _ => "PS1='\\w > '\n",
        };
        let _ = writer
            .write_all(command.as_bytes())
            .and_then(|_| writer.flush());
    }

    #[cfg(windows)]
    {
        let command = if shell.name.eq_ignore_ascii_case("cmd.exe") {
            "PROMPT \u{1b}]7;file://%COMPUTERNAME%$P\u{7}$P$G\r\n"
        } else {
            "function prompt { $path = (Get-Location).Path; Write-Host -NoNewline \"`e]7;file://$env:COMPUTERNAME$path`a\"; \"$path > \" }\r\n"
        };
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
    fn supported_unix_shell_uses_login_interactive_startup_and_cwd_prompt_hook() {
        let shell = ShellDescriptor {
            name: "bash".to_string(),
            path: "/bin/bash".to_string(),
        };
        let command = build_shell_command(&shell);
        let arguments = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(arguments, vec!["/bin/bash", "-l", "-i"]);

        let mut bootstrap = Vec::new();
        install_session_prompt(&mut bootstrap, &shell);
        let bootstrap = String::from_utf8(bootstrap).expect("bootstrap must be UTF-8");
        assert!(bootstrap.contains("PROMPT_COMMAND"));
        assert!(bootstrap.contains("PS1='\\w > '"));
        assert!(bootstrap.contains("]7;file://"));
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
        command.args(["-c", "printf '\\033[31mred\\033[0m'"]);
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
    fn zsh_bootstrap_reports_cwd_with_osc7() {
        let shell = ShellDescriptor {
            name: "zsh".to_string(),
            path: "/bin/zsh".to_string(),
        };
        let mut bootstrap = Vec::new();
        install_session_prompt(&mut bootstrap, &shell);
        let bootstrap = String::from_utf8(bootstrap).expect("bootstrap must be UTF-8");
        assert!(bootstrap.contains("add-zsh-hook"));
        assert!(bootstrap.contains("PROMPT='%~ > '"));
        assert!(bootstrap.contains("]7;file://"));
    }
}
