//! Tauri-free owner of the interactive user terminal PTYs.
//!
//! This is the runtime half of U-7: the standalone runtime is the single owner
//! of every user terminal, exactly as `src/terminal.rs` used to be for the
//! desktop. It is a real backend module rather than a `#[path]` include of the
//! desktop source, because the desktop source is bound to `tauri::AppHandle`
//! and emits window events. Here the same PTY core publishes typed events into
//! a bounded per-session broadcast broker instead, so the control plane can
//! relay them over SSE and the desktop relay can forward them to the window.
//!
//! Two invariants carry over unchanged:
//! - these are direct user terminals with the user's normal shell permissions,
//!   deliberately separate from the AI shell tool and `shell_policy`; and
//! - nothing is persisted: PTY output lives only in the bounded in-memory
//!   broadcast and is never written to the database or a transcript file.

use crate::environment::{
    get_available_shells, get_default_shell, get_terminal_environment, ShellDescriptor,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chatspeed_contracts::{
    TerminalExitEvent, TerminalOutputEvent, TerminalSessionMetadataDto, TerminalShellDto,
    TerminalStreamEnvelope, TerminalStreamEvent,
};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 200;

/// Bounded capacity of one session's SSE broadcast.
///
/// A slow observer that falls further behind than this is told to reset rather
/// than silently losing output, so a terminal can never show a corrupted mix of
/// bytes.
const TERMINAL_STREAM_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A terminal operation failure, before it is mapped onto the wire.
///
/// The variants mirror the stable `terminal_*` codes the desktop command layer
/// already returns, so the runtime relay reports the same reason to the UI.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// No live session exists for the id.
    #[error("terminal_session_not_found")]
    SessionNotFound,
    /// The session's process already ended.
    #[error("terminal_session_exited")]
    SessionExited,
    /// No supported shell is available.
    #[error("terminal_shell_unavailable")]
    ShellUnavailable,
    /// The requested PTY dimensions are outside the supported bounds.
    #[error("terminal_size_invalid")]
    InvalidSize,
    /// The requested working directory could not be resolved.
    #[error("terminal_cwd_invalid: {0}")]
    InvalidCwd(String),
    /// The caller's lease does not own the session.
    #[error("terminal_forbidden: {0}")]
    Forbidden(String),
    /// The PTY pair could not be opened.
    #[error("terminal_pty_open_failed: {0}")]
    PtyOpenFailed(String),
    /// The shell process could not be spawned.
    #[error("terminal_spawn_failed: {0}")]
    SpawnFailed(String),
    /// The PTY writer could not be taken.
    #[error("terminal_writer_failed: {0}")]
    WriterFailed(String),
    /// The PTY reader could not be taken.
    #[error("terminal_reader_failed: {0}")]
    ReaderFailed(String),
    /// Writing user input to the PTY failed.
    #[error("terminal_write_failed: {0}")]
    WriteFailed(String),
    /// Resizing the PTY failed.
    #[error("terminal_resize_failed: {0}")]
    ResizeFailed(String),
}

/// The registering client lease that owns a terminal session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOwner {
    pub client_id: String,
    pub lease_id: String,
}

// ---------------------------------------------------------------------------
// Per-session event broker
// ---------------------------------------------------------------------------

/// One publisher's view of a session's SSE broadcast.
pub struct TerminalSubscription {
    /// Live events published after this subscription attached.
    pub receiver: broadcast::Receiver<TerminalStreamEnvelope>,
    /// A terminal event already published before the subscriber attached.
    ///
    /// The stream keeps no transcript, so an observer that attaches after the
    /// process ended receives this single terminal envelope instead of waiting
    /// forever on a broadcast that will never produce another event.
    pub pending_terminal: Option<TerminalStreamEnvelope>,
}

/// The bounded broadcast of one session's typed terminal events.
pub struct TerminalStreamEntry {
    session_id: String,
    sender: broadcast::Sender<TerminalStreamEnvelope>,
    sequence: AtomicU64,
    terminal: Mutex<Option<TerminalStreamEnvelope>>,
}

impl TerminalStreamEntry {
    fn new(session_id: &str) -> Self {
        let (sender, _receiver) = broadcast::channel(TERMINAL_STREAM_CAPACITY);
        Self {
            session_id: session_id.to_string(),
            sender,
            sequence: AtomicU64::new(0),
            terminal: Mutex::new(None),
        }
    }

    /// Subscribes one SSE observer, reporting an already-published terminal
    /// event so a late observer is never left waiting.
    pub fn subscribe(&self) -> TerminalSubscription {
        let receiver = self.sender.subscribe();
        let pending_terminal = self.terminal.lock().clone();
        TerminalSubscription {
            receiver,
            pending_terminal,
        }
    }

    /// Publishes one non-terminal event with the next per-session sequence.
    pub fn publish(&self, event: TerminalStreamEvent) -> TerminalStreamEnvelope {
        let envelope = self.envelope(event);
        let _ = self.sender.send(envelope.clone());
        envelope
    }

    /// Publishes the single terminal event and remembers it for late observers.
    pub fn publish_terminal(&self, event: TerminalStreamEvent) -> TerminalStreamEnvelope {
        let envelope = self.publish(event);
        *self.terminal.lock() = Some(envelope.clone());
        envelope
    }

    fn envelope(&self, event: TerminalStreamEvent) -> TerminalStreamEnvelope {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        TerminalStreamEnvelope::new(self.session_id.clone(), sequence, event)
    }
}

/// The instance-local registry of per-session terminal broadcasts.
///
/// Entries are created with a session and removed when the session is removed,
/// so the map stays bounded by the tabs a client actually opened.
#[derive(Default)]
pub struct TerminalStreamBroker {
    inner: Mutex<HashMap<String, Arc<TerminalStreamEntry>>>,
}

impl TerminalStreamBroker {
    /// Creates an empty broker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the entry for `session_id`, creating it when absent.
    pub fn entry(&self, session_id: &str) -> Arc<TerminalStreamEntry> {
        self.inner
            .lock()
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(TerminalStreamEntry::new(session_id)))
            .clone()
    }

    /// Returns the entry for `session_id` if one exists.
    pub fn get(&self, session_id: &str) -> Option<Arc<TerminalStreamEntry>> {
        self.inner.lock().get(session_id).cloned()
    }

    /// Removes a session's entry once the session itself is gone.
    pub fn remove(&self, session_id: &str) {
        self.inner.lock().remove(session_id);
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

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
    metadata: TerminalSessionMetadataDto,
    owner: TerminalOwner,
    // Natural process exit releases PTY resources immediately while retaining metadata so an
    // exited tab can still be listed until the user explicitly closes it.
    resources: Option<TerminalResources>,
}

/// Owns every interactive PTY process the runtime serves.
///
/// The manager deliberately has no dependency on the AI shell tool or
/// `shell_policy`: these are direct user terminals with the user's normal shell
/// permissions, not AI shell-tool executions.
pub struct TerminalManager {
    sessions: Arc<Mutex<TerminalRegistry>>,
    broker: Arc<TerminalStreamBroker>,
}

fn owns_session(session: &TerminalSession, client_id: &str, lease_id: &str) -> bool {
    session.owner.client_id == client_id && session.owner.lease_id == lease_id
}

impl TerminalManager {
    /// Creates an empty manager with its own event broker.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(TerminalRegistry::default())),
            broker: Arc::new(TerminalStreamBroker::new()),
        }
    }

    /// The bounded per-session event broker this manager publishes into.
    pub fn broker(&self) -> &Arc<TerminalStreamBroker> {
        &self.broker
    }

    /// Lists the shells offered for an interactive terminal.
    pub fn list_shells(&self) -> Vec<TerminalShellDto> {
        let default_path = get_default_shell().map(|shell| shell.path);
        get_available_shells()
            .into_iter()
            .map(|shell| TerminalShellDto {
                is_default: default_path.as_deref() == Some(shell.path.as_str()),
                name: shell.name,
                path: shell.path,
            })
            .collect()
    }

    /// Lists the sessions owned by `(client_id, lease_id)`.
    ///
    /// A replacement lease for the same client id cannot attach to sessions
    /// created by the old lease.
    pub fn list_sessions(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Vec<TerminalSessionMetadataDto> {
        self.sessions
            .lock()
            .sessions
            .values()
            .filter(|session| owns_session(session, client_id, lease_id))
            .map(|session| session.metadata.clone())
            .collect()
    }

    /// Opens a new interactive session owned by `owner`.
    pub fn create(
        &self,
        owner: TerminalOwner,
        cwd_candidate: Option<&str>,
        shell_path: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
    ) -> Result<TerminalSessionMetadataDto, TerminalError> {
        let shell = select_shell(shell_path)?;
        let cwd = resolve_initial_cwd(cwd_candidate)?;
        let session_id = Uuid::new_v4().to_string();
        let size = validated_size(cols, rows)?;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|error| TerminalError::PtyOpenFailed(error.to_string()))?;

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
            .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
        #[cfg(unix)]
        let process_group = pair.master.process_group_leader();
        #[cfg(not(unix))]
        let process_group: Option<libc::pid_t> = None;
        let mut writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                terminate_child(&mut *child, process_group);
                return Err(TerminalError::WriterFailed(error.to_string()));
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
                terminate_child(&mut *child, process_group);
                return Err(TerminalError::ReaderFailed(error.to_string()));
            }
        };

        let metadata = TerminalSessionMetadataDto {
            session_id: session_id.clone(),
            shell_name: shell.name,
            shell_path: shell.path,
            cwd: display_cwd(&cwd),
            alive: true,
        };
        self.sessions.lock().sessions.insert(
            session_id.clone(),
            TerminalSession {
                metadata: metadata.clone(),
                owner,
                resources: Some(TerminalResources {
                    writer,
                    master: pair.master,
                    child,
                    #[cfg(unix)]
                    process_group,
                }),
            },
        );
        let entry = self.broker.entry(&session_id);
        spawn_reader(
            session_id,
            reader,
            Arc::clone(&self.sessions),
            Arc::clone(&self.broker),
            entry,
        );

        Ok(metadata)
    }

    /// Writes user input to a session owned by `client_id`.
    pub fn write(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        input: &str,
    ) -> Result<(), TerminalError> {
        let mut registry = self.sessions.lock();
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(TerminalError::SessionNotFound)?;
        if !owns_session(session, client_id, lease_id) {
            return Err(TerminalError::Forbidden(
                "the session belongs to another client lease".to_string(),
            ));
        }
        if !session.metadata.alive {
            return Err(TerminalError::SessionExited);
        }
        let resources = session
            .resources
            .as_mut()
            .ok_or(TerminalError::SessionExited)?;
        resources
            .writer
            .write_all(input.as_bytes())
            .and_then(|_| resources.writer.flush())
            .map_err(|error| TerminalError::WriteFailed(error.to_string()))
    }

    /// Resizes a session owned by `client_id`.
    pub fn resize(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalError> {
        let size = validated_size(Some(cols), Some(rows))?;
        let registry = self.sessions.lock();
        let session = registry
            .sessions
            .get(session_id)
            .ok_or(TerminalError::SessionNotFound)?;
        if !owns_session(session, client_id, lease_id) {
            return Err(TerminalError::Forbidden(
                "the session belongs to another client lease".to_string(),
            ));
        }
        let resources = session
            .resources
            .as_ref()
            .ok_or(TerminalError::SessionExited)?;
        resources
            .master
            .resize(size)
            .map_err(|error| TerminalError::ResizeFailed(error.to_string()))
    }

    /// Closes a session owned by `client_id`.
    ///
    /// Repeating a close is harmless so UI races cannot leak a PTY; closing a
    /// session that belongs to another lease is refused.
    pub fn close(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<(), TerminalError> {
        let owned = {
            let registry = self.sessions.lock();
            match registry.sessions.get(session_id) {
                None => return Ok(()),
                Some(session) if !owns_session(session, client_id, lease_id) => false,
                Some(_) => true,
            }
        };
        if !owned {
            return Err(TerminalError::Forbidden(
                "the session belongs to another client lease".to_string(),
            ));
        }
        self.drop_session(session_id, "session_closed");
        Ok(())
    }

    /// Subscribes to a session's typed event stream.
    ///
    /// `Ok(None)` means the session is owned but has no live broadcast in this
    /// instance (already torn down); `Err(Forbidden)` means another lease owns
    /// it and must not read its output.
    pub fn subscribe(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<Option<TerminalSubscription>, TerminalError> {
        {
            let registry = self.sessions.lock();
            let session = registry
                .sessions
                .get(session_id)
                .ok_or(TerminalError::SessionNotFound)?;
            if !owns_session(session, client_id, lease_id) {
                return Err(TerminalError::Forbidden(
                    "the session belongs to another client lease".to_string(),
                ));
            }
        }
        Ok(self.broker.get(session_id).map(|entry| entry.subscribe()))
    }

    /// Drops every session whose registering lease no longer validates.
    ///
    /// Called by the runtime lease sweeper, so a released or expired client
    /// lease can never leave an orphaned user PTY running.
    pub fn sweep_invalid_owners(&self, is_valid: &(dyn Fn(&str, &str) -> bool + Send + Sync)) {
        let stale = {
            let registry = self.sessions.lock();
            registry
                .sessions
                .iter()
                .filter(|(_, session)| !is_valid(&session.owner.client_id, &session.owner.lease_id))
                .map(|(session_id, _)| session_id.clone())
                .collect::<Vec<_>>()
        };
        for session_id in stale {
            self.drop_session(&session_id, "lease_released");
        }
    }

    /// Terminates and forgets every session. Runs on runtime shutdown.
    pub fn cleanup_all(&self) {
        let sessions = std::mem::take(&mut self.sessions.lock().sessions);
        for (session_id, mut session) in sessions {
            session.metadata.alive = false;
            terminate_terminal_resources(session.resources.take());
            if let Some(entry) = self.broker.get(&session_id) {
                entry.publish_terminal(terminal_reset("runtime_shutdown"));
            }
            self.broker.remove(&session_id);
        }
    }

    /// Removes one session regardless of ownership and tells its observers.
    fn drop_session(&self, session_id: &str, reason: &str) {
        let session = self.sessions.lock().sessions.remove(session_id);
        if let Some(mut session) = session {
            session.metadata.alive = false;
            terminate_terminal_resources(session.resources.take());
        }
        if let Some(entry) = self.broker.get(session_id) {
            entry.publish_terminal(terminal_reset(reason));
            self.broker.remove(session_id);
        }
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.cleanup_all();
    }
}

fn terminal_reset(reason: &str) -> TerminalStreamEvent {
    TerminalStreamEvent::Reset {
        reason: reason.to_string(),
    }
}

/// Spawns the PTY reader loop that publishes output and the terminal event.
///
/// A natural EOF reaps the exited child while retaining the tab's metadata; a
/// reader I/O error instead removes the session, because the UI can no longer
/// control a PTY whose output stopped flowing.
fn spawn_reader(
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    sessions: Arc<Mutex<TerminalRegistry>>,
    broker: Arc<TerminalStreamBroker>,
    entry: Arc<TerminalStreamEntry>,
) {
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 8192];
        let reached_eof = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break true,
                Ok(read) => {
                    entry.publish(TerminalStreamEvent::Output(TerminalOutputEvent {
                        session_id: session_id.clone(),
                        data_base64: BASE64.encode(&buffer[..read]),
                    }));
                }
                Err(_) => break false,
            }
        };
        if reached_eof {
            reap_exited_session(&sessions, &session_id);
        } else {
            abort_terminal_session(&sessions, &session_id);
        }
        entry.publish_terminal(TerminalStreamEvent::Exit(TerminalExitEvent {
            session_id: session_id.clone(),
            exit_code: None,
        }));
        if !reached_eof {
            // The reader failure removed the session, so its broadcast is no
            // longer reachable; subscribed observers already received the
            // buffered terminal event above.
            broker.remove(&session_id);
        }
    });
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
    if let Some(resources) = resources {
        let mut resources = resources;
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

#[cfg(unix)]
fn terminate_child(child: &mut dyn portable_pty::Child, process_group: Option<libc::pid_t>) {
    terminate_child_tree(child, process_group);
}

#[cfg(not(unix))]
fn terminate_child(child: &mut dyn portable_pty::Child, _process_group: Option<libc::pid_t>) {
    #[cfg(windows)]
    terminate_child_tree(child);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child.kill();
        let _ = child.wait();
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

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
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
fn display_cwd(path: &Path) -> String {
    windows_display_path(path)
}

#[cfg(not(windows))]
fn display_cwd(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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

fn unix_shell_quote(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

fn install_session_prompt(writer: &mut (dyn Write + Send), shell: &ShellDescriptor, cwd: &Path) {
    #[cfg(unix)]
    {
        let quoted_cwd = unix_shell_quote(cwd);
        let command = match shell.name.as_str() {
            "zsh" => format!("cd -- '{quoted_cwd}'; autoload -Uz add-zsh-hook; _cs_terminal_osc7(){{ print -n $'\\e]7;file://'${{HOST:-localhost}}${{PWD}}$'\\a'; }}; add-zsh-hook precmd _cs_terminal_osc7; PROMPT='%~ > '\n"),
            "bash" => format!("cd -- '{quoted_cwd}'; __cs_terminal_osc7(){{ printf '\\033]7;file://%s%s\\007' \"${{HOSTNAME:-localhost}}\" \"$PWD\"; }}; PROMPT_COMMAND=\"__cs_terminal_osc7${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\"; PS1='\\w > '\n"),
            "fish" => format!("cd -- '{quoted_cwd}'; function __cs_terminal_osc7 --on-event fish_prompt; printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; end; function fish_prompt; set -l display_path \"$PWD\"; if test \"$PWD\" = \"$HOME\"; set display_path '~'; else; set display_path (string replace -- \"$HOME/\" '~/' \"$PWD\"); end; printf '%s > ' \"$display_path\"; end\n"),
            _ => format!("cd -- '{quoted_cwd}'; PS1='\\w > '\n"),
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

fn resolve_initial_cwd(candidate: Option<&str>) -> Result<PathBuf, TerminalError> {
    if let Some(candidate) = candidate {
        let path = PathBuf::from(candidate);
        if path.is_dir() {
            return path
                .canonicalize()
                .map_err(|error| TerminalError::InvalidCwd(error.to_string()));
        }
    }

    if let Some(home) = dirs::home_dir().filter(|path| path.is_dir()) {
        return home
            .canonicalize()
            .map_err(|error| TerminalError::InvalidCwd(error.to_string()));
    }

    std::env::current_dir().map_err(|error| TerminalError::InvalidCwd(error.to_string()))
}

fn select_shell(shell_path: Option<&str>) -> Result<ShellDescriptor, TerminalError> {
    let shells = get_available_shells();
    let shell = match shell_path {
        Some(path) => shells.into_iter().find(|shell| shell.path == path),
        None => get_default_shell(),
    };
    shell.ok_or(TerminalError::ShellUnavailable)
}

fn validated_size(cols: Option<u16>, rows: Option<u16>) -> Result<PtySize, TerminalError> {
    let cols = cols.unwrap_or(DEFAULT_COLS);
    let rows = rows.unwrap_or(DEFAULT_ROWS);
    if cols == 0 || rows == 0 || cols > MAX_COLS || rows > MAX_ROWS {
        return Err(TerminalError::InvalidSize);
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
        build_shell_command, install_session_prompt, reap_exited_session, resolve_initial_cwd,
        validated_size, ShellDescriptor, TerminalError, TerminalManager, TerminalOwner,
        TerminalRegistry, TerminalResources, TerminalSession, DEFAULT_COLS, DEFAULT_ROWS,
    };

    fn metadata(session_id: &str) -> chatspeed_contracts::TerminalSessionMetadataDto {
        chatspeed_contracts::TerminalSessionMetadataDto {
            session_id: session_id.to_string(),
            shell_name: "sh".to_string(),
            shell_path: "/bin/sh".to_string(),
            cwd: "/".to_string(),
            alive: true,
        }
    }

    fn owner(client_id: &str) -> TerminalOwner {
        TerminalOwner {
            client_id: client_id.to_string(),
            lease_id: format!("{client_id}-lease"),
        }
    }

    #[test]
    fn validates_terminal_dimensions() {
        let size = validated_size(None, None).expect("default terminal size should be valid");
        assert_eq!(size.cols, DEFAULT_COLS);
        assert_eq!(size.rows, DEFAULT_ROWS);
        assert!(matches!(
            validated_size(Some(0), Some(10)),
            Err(TerminalError::InvalidSize)
        ));
        assert!(matches!(
            validated_size(Some(501), Some(10)),
            Err(TerminalError::InvalidSize)
        ));
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
        assert!(bootstrap.contains("cd -- '/workspace'"));
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

        let zsh = ShellDescriptor {
            name: "zsh".to_string(),
            path: "/bin/zsh".to_string(),
        };
        let mut zsh_bootstrap = Vec::new();
        install_session_prompt(&mut zsh_bootstrap, &zsh, Path::new("/workspace"));
        let zsh_bootstrap = String::from_utf8(zsh_bootstrap).expect("zsh bootstrap must be UTF-8");
        assert!(zsh_bootstrap.contains("PROMPT='%~ > '"));
        assert!(zsh_bootstrap.contains("add-zsh-hook"));
        assert!(zsh_bootstrap.contains("]7;file://"));
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
        let mut session = TerminalSession {
            metadata: metadata("natural-exit"),
            owner: owner("client-a"),
            resources: Some(TerminalResources {
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
        let sessions = Arc::new(super::Mutex::new(TerminalRegistry {
            sessions: HashMap::from([(
                session_id.clone(),
                TerminalSession {
                    metadata: metadata(&session_id),
                    owner: owner("client-a"),
                    resources: Some(TerminalResources {
                        writer,
                        master: pair.master,
                        child,
                        process_group,
                    }),
                },
            )]),
        }));

        reap_exited_session(&sessions, &session_id);
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
        let sessions = Arc::new(super::Mutex::new(TerminalRegistry {
            sessions: HashMap::from([(
                session_id.clone(),
                TerminalSession {
                    metadata: metadata(&session_id),
                    owner: owner("client-a"),
                    resources: Some(TerminalResources {
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
        // Release our own slave handle before reading: while it stays open the
        // master never observes EOF when the child exits, so `read_to_end` would
        // block forever. The spawned child keeps its own slave fd until it exits.
        drop(pair.slave);
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

    #[tokio::test]
    async fn the_broker_sequences_events_and_remembers_the_terminal_one() {
        use chatspeed_contracts::{TerminalOutputEvent, TerminalStreamEvent};

        let manager = TerminalManager::new();
        let entry = manager.broker().entry("session-1");
        let subscription = entry.subscribe();
        let mut receiver = subscription.receiver;

        entry.publish(TerminalStreamEvent::Output(TerminalOutputEvent {
            session_id: "session-1".to_string(),
            data_base64: "aGVsbG8=".to_string(),
        }));
        entry.publish_terminal(TerminalStreamEvent::Exit(
            chatspeed_contracts::TerminalExitEvent {
                session_id: "session-1".to_string(),
                exit_code: None,
            },
        ));

        let first = receiver.recv().await.expect("first envelope");
        assert_eq!(first.sequence, 0);
        assert!(!first.is_terminal());
        let second = receiver.recv().await.expect("terminal envelope");
        assert_eq!(second.sequence, 1);
        assert!(second.is_terminal());

        // A subscriber that attaches after the process ended still observes the
        // terminal event instead of waiting on a stream that will never resume.
        let late = entry.subscribe();
        assert!(late.pending_terminal.is_some());
        assert!(late
            .pending_terminal
            .expect("pending terminal event")
            .is_terminal());
    }

    #[test]
    fn sessions_are_only_visible_and_controllable_by_their_owning_lease() {
        let manager = TerminalManager::new();
        let session_id = "owned-session".to_string();
        let old_lease = owner("client-a");
        manager.sessions.lock().sessions.insert(
            session_id.clone(),
            TerminalSession {
                metadata: metadata(&session_id),
                owner: old_lease.clone(),
                resources: None,
            },
        );

        let new_lease = TerminalOwner {
            client_id: "client-a".to_string(),
            lease_id: "client-a-new-lease".to_string(),
        };
        assert_eq!(
            manager.list_sessions("client-a", &old_lease.lease_id).len(),
            1
        );
        assert!(manager
            .list_sessions("client-a", &new_lease.lease_id)
            .is_empty());
        assert!(matches!(
            manager.write("client-a", &new_lease.lease_id, &session_id, "ls\n"),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(matches!(
            manager.resize("client-a", &new_lease.lease_id, &session_id, 80, 24),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(matches!(
            manager.close("client-a", &new_lease.lease_id, &session_id),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(matches!(
            manager.subscribe("client-a", &new_lease.lease_id, &session_id),
            Err(TerminalError::Forbidden(_))
        ));

        assert!(manager.list_sessions("client-b", "lease-b").is_empty());
        assert!(matches!(
            manager.write("client-b", "lease-b", &session_id, "ls\n"),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(matches!(
            manager.resize("client-b", "lease-b", &session_id, 80, 24),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(matches!(
            manager.close("client-b", "lease-b", &session_id),
            Err(TerminalError::Forbidden(_))
        ));
        assert!(
            manager.sessions.lock().sessions.contains_key(&session_id),
            "a foreign lease must not be able to retire another client's session"
        );
        assert!(matches!(
            manager.subscribe("client-b", "lease-b", &session_id),
            Err(TerminalError::Forbidden(_))
        ));

        // The owner can observe the session, but a session whose resources are
        // gone reports a stable exited state rather than panicking.
        assert!(manager
            .subscribe("client-a", &old_lease.lease_id, &session_id)
            .is_ok());
        assert!(matches!(
            manager.write("client-a", &old_lease.lease_id, &session_id, "ls\n"),
            Err(TerminalError::SessionExited)
        ));

        assert!(manager
            .close("client-a", &old_lease.lease_id, &session_id)
            .is_ok());
        assert!(manager
            .list_sessions("client-a", &old_lease.lease_id)
            .is_empty());
        // Explicit close is idempotent.
        assert!(manager
            .close("client-a", &old_lease.lease_id, &session_id)
            .is_ok());
    }

    #[test]
    fn an_unknown_session_reports_not_found() {
        let manager = TerminalManager::new();
        assert!(matches!(
            manager.subscribe("client-a", "lease-a", "missing"),
            Err(TerminalError::SessionNotFound)
        ));
        assert!(matches!(
            manager.write("client-a", "lease-a", "missing", "x"),
            Err(TerminalError::SessionNotFound)
        ));
    }

    #[test]
    fn sweeping_drops_sessions_without_a_valid_lease() {
        let manager = TerminalManager::new();
        let session_id = "orphan".to_string();
        manager.sessions.lock().sessions.insert(
            session_id.clone(),
            TerminalSession {
                metadata: metadata(&session_id),
                owner: owner("client-a"),
                resources: None,
            },
        );

        manager.sweep_invalid_owners(&|_, _| false);
        assert!(manager
            .list_sessions("client-a", "client-a-lease")
            .is_empty());
        assert!(!manager.sessions.lock().sessions.contains_key(&session_id));
    }
}
