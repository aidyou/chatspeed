//! Owned child processes for runtime work that must not outlive the runtime.
//!
//! A pre-workflow automation shell is runtime-owned work. Once the runtime is
//! cancelled, the shell it started — including any descendant it left behind —
//! must be terminated and the root reaped before the process exits. Relying on
//! the `tokio::process::Child` being dropped is *not* enough: dropping a future
//! does not stop the child, and a process that ignores `SIGTERM` or holds the
//! inherited stdout/stderr pipe open would survive.
//!
//! This module owns that contract:
//!
//! - a shell is started in its own process *session* (and therefore its own
//!   process group), so its whole tree can be signalled with a negative-pid
//!   signal without ever touching the runtime's own group, the test harness
//!   group, or an unrelated service. The isolation is *verified after spawn*;
//!   when the child is not its own group leader the group signal is refused and
//!   only the root-anchored descendant tree is signalled, so a backend that
//!   failed to isolate can never escalate into a collateral group kill;
//! - on Windows the shell is bound into its own Job Object configured with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The tree is then owned by a kernel
//!   handle instead of a PID, so `TerminateJobObject` reaps exactly this tree and
//!   can never reach a reused global PID; the root's creation-time fingerprint is
//!   re-checked before the best-effort PID-based graceful signal;
//! - [`OwnedProcessCancellation`] is the one runtime-wide signal: `cancel_all`
//!   flips it and every owned shell tears its own tree down, so a coordinated
//!   shutdown observes a real teardown instead of only a closed admission gate;
//! - each owned shell holds an RAII ticket that keeps the in-flight count, so the
//!   shutdown can *await* confirmation ([`OwnedProcessCancellation::wait_idle`])
//!   instead of trusting `Drop`, while a dropped run future still hands its child
//!   to a detached cleanup task rather than leaking it;
//! - the process group's leader identity (a Linux start-time fingerprint) is
//!   recorded at spawn and re-checked before a group signal, so a reaped root
//!   whose PID was reused is never signalled as if it were still ours.

use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{watch, Notify};

/// Grace between the graceful `SIGTERM` and the forced `SIGKILL` sweep.
const SHELL_GRACE_BUDGET: Duration = Duration::from_millis(300);
/// Upper bound on the forced `SIGKILL` sweep. Kept well below the daemon's
/// coordinated-shutdown budget so an owned teardown never consumes it whole.
const SHELL_FORCE_KILL_BUDGET: Duration = Duration::from_millis(2_000);
/// Poll step of the forced-kill sweep.
const SHELL_FORCE_KILL_STEP: Duration = Duration::from_millis(25);

/// The outcome of one owned shell execution.
pub enum OwnedShellOutcome {
    /// The shell ran to completion; its captured output is returned.
    Completed(std::process::Output),
    /// The runtime cancelled the shell while it was in flight. The whole owned
    /// tree was terminated and the root reaped before this was returned.
    Cancelled,
}

/// The one runtime-wide cancellation signal for owned child processes.
///
/// Cheap to clone (it shares one inner `Arc`), so the runtime owner hands the
/// same signal to every owned shell it starts.
#[derive(Clone)]
pub struct OwnedProcessCancellation {
    inner: Arc<CancellationInner>,
}

struct CancellationInner {
    cancelled: watch::Sender<bool>,
    in_flight: AtomicUsize,
    idle: Notify,
}

impl OwnedProcessCancellation {
    pub fn new() -> Self {
        let (cancelled, _) = watch::channel(false);
        Self {
            inner: Arc::new(CancellationInner {
                cancelled,
                in_flight: AtomicUsize::new(0),
                idle: Notify::new(),
            }),
        }
    }

    /// Whether cancellation has already been requested. A cancelled runtime is
    /// never un-cancelled, so this is monotonic.
    pub fn is_cancelled(&self) -> bool {
        *self.inner.cancelled.borrow()
    }

    /// Owned shells currently in flight (started and not yet fully torn down).
    pub fn in_flight(&self) -> usize {
        self.inner.in_flight.load(Ordering::SeqCst)
    }

    /// Records one owned shell and returns the RAII ticket that keeps the
    /// in-flight count until the shell's tree has really been torn down.
    pub fn begin(&self) -> OwnedProcessTicket {
        self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        OwnedProcessTicket {
            receiver: self.inner.cancelled.subscribe(),
            cancel: Some(self.clone()),
        }
    }

    /// Flips the cancellation signal. Every owned shell observes it on its own
    /// and tears its tree down; this call itself does not wait.
    pub fn cancel_all(&self) {
        self.inner.cancelled.send_replace(true);
    }

    /// Resolves once cancellation is requested, without holding an in-flight
    /// ticket.
    ///
    /// This is for an observer that only needs to react to the signal (for
    /// example a tool child's teardown watcher) while the RAII ticket — which
    /// keeps the drain honest — is owned elsewhere.
    pub async fn cancelled(&self) {
        let mut receiver = self.inner.cancelled.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }

    /// Resolves once no owned shell is in flight. A caller with a deadline wraps
    /// this in its own timeout, so a shell that cannot finish never blocks the
    /// shutdown past the shared budget.
    pub async fn wait_idle(&self) {
        loop {
            if self.inner.in_flight.load(Ordering::SeqCst) == 0 {
                return;
            }
            let notified = self.inner.idle.notified();
            // Re-check after registering so a release that landed between the
            // first read and the registration is not lost.
            if self.inner.in_flight.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }

    /// Requests cancellation and awaits every owned shell's teardown. This is
    /// the explicit confirmation the coordinated shutdown uses instead of
    /// trusting a dropped handle.
    pub async fn cancel_all_and_wait(&self) {
        self.cancel_all();
        self.wait_idle().await;
    }
}

impl Default for OwnedProcessCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for OwnedProcessCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedProcessCancellation")
            .field("cancelled", &self.is_cancelled())
            .field("in_flight", &self.in_flight())
            .finish()
    }
}

/// RAII hold on one in-flight owned shell.
///
/// Dropping the ticket releases the hold and wakes any drain observer, so the
/// ticket must be dropped only once the shell's tree has been torn down (or the
/// shell completed). The cleanup task moves the ticket with itself, so the drain
/// still waits for the real teardown.
pub struct OwnedProcessTicket {
    receiver: watch::Receiver<bool>,
    cancel: Option<OwnedProcessCancellation>,
}

impl OwnedProcessTicket {
    /// Whether cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    /// Resolves once cancellation is requested. Returns immediately when it was
    /// already requested, so a late subscriber still observes the signal.
    pub async fn cancelled(&mut self) {
        if *self.receiver.borrow() {
            return;
        }
        let _ = self.receiver.changed().await;
    }

    fn release(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let previous = cancel.inner.in_flight.fetch_sub(1, Ordering::SeqCst);
            if previous <= 1 {
                cancel.inner.idle.notify_waiters();
            }
        }
    }
}

impl Drop for OwnedProcessTicket {
    fn drop(&mut self) {
        self.release();
    }
}

/// A Linux process start-time fingerprint, used to refuse a reused PID. Two
/// lookups of the *same* PID that return different fingerprints prove the PID
/// was reused by a different process.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessIdentity {
    /// `/proc/<pid>/stat` field 22 (`starttime`), in clock ticks.
    start_ticks: u64,
}

/// The short, owned-tree handle: the process group plus the identity needed to
/// signal exactly that group. It deliberately holds no `Child` and no second
/// registry — only what a cancellation needs to reach the tree.
#[cfg(unix)]
#[derive(Clone)]
struct OwnedTreeHandle {
    pid: libc::pid_t,
    /// Whether the child really became its own process group leader. Group
    /// signalling is refused otherwise.
    group_isolated: bool,
    /// Whether the child really became its own session leader. Session-member
    /// enumeration is refused otherwise (enumerating a shared session would
    /// reach the runtime's own group).
    session_isolated: bool,
    identity: Option<ProcessIdentity>,
}

#[cfg(windows)]
struct OwnedTreeHandle {
    pid: u32,
    /// The Job Object that holds the shell's whole tree, when it could be
    /// created and the freshly spawned child assigned to it. Ownership is by
    /// kernel handle, so teardown reaches exactly the assigned tree and a PID
    /// reused by an unrelated process can never be mistaken for it.
    job: Option<JobHandle>,
    /// The root's creation-time fingerprint. The job handle owns the tree, so
    /// this guards only the best-effort PID-based console signal: a root whose
    /// PID was reused must never receive it.
    identity: Option<u64>,
}

/// One owned child process: the tokio child plus the owned-tree handle.
struct OwnedChild {
    child: Child,
    handle: OwnedTreeHandle,
    reaped: bool,
}

impl OwnedChild {
    fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Terminates the whole owned tree and reaps the root. The root is only
    /// reaped after the tree signal, and the group signal is guarded by the
    /// leader identity (unix) / the job handle (windows), so a reaped-then-reused
    /// PID is never signalled.
    async fn terminate(&mut self) {
        self.signal_tree();
        // Terminate the direct root as well, by the child handle we own. On
        // Windows the job may have failed to be created or assigned, so the
        // tree signal can be a no-op; on unix this is a redundant root kill.
        // Either way the root is ours to kill by handle, never by name or a
        // global PID lookup.
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        self.reaped = true;
    }

    /// Best-effort teardown with no tokio runtime to await the root on. Used
    /// only from a `Drop` on a non-async path.
    fn signal_tree_blocking(&mut self) {
        self.signal_tree();
    }

    fn signal_tree(&mut self) {
        #[cfg(unix)]
        self.handle.signal_tree();
        #[cfg(windows)]
        self.handle.terminate_tree();
    }
}

/// A shell command in the runtime's own launch convention.
fn shell_command(command_text: &str) -> Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Start the child in a new *session*, which makes it both its own
        // session and process group leader (`sid == pgid == pid`). A session is
        // what keeps a negative-pid signal scoped to this tree; without it a
        // group signal could reach the runtime's own group. `setsid` fails only
        // for an already-group-leader child, which the post-fork child is not.
        let mut std_command = std::process::Command::new("sh");
        std_command.args(["-c", command_text]);
        unsafe {
            std_command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        std_command.into()
    }

    #[cfg(windows)]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", command_text]);
        // `tokio::process::Command` exposes `creation_flags` inherently.
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        command
    }
}

/// Spawns one owned shell in its own process tree.
fn spawn_owned_shell(command_text: &str) -> std::io::Result<OwnedChild> {
    let mut command = shell_command(command_text);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The owned shell is terminated by this module's explicit teardown, never
        // by a dropped handle; disabling the implicit kill keeps the teardown
        // path the single authority.
        .kill_on_drop(false);
    let child = command.spawn()?;

    #[cfg(unix)]
    let handle = {
        let pid = child.id().map(|id| id as libc::pid_t).unwrap_or(0);
        // Verify the isolation the spawn requested instead of trusting it: the
        // child is only group/session signalable when it really owns that group
        // and session.
        let group_isolated = pid > 0 && process_group_of(pid) == Some(pid);
        let session_isolated = pid > 0 && session_of(pid) == Some(pid);
        let identity = (pid > 0).then(|| process_identity(pid)).flatten();
        OwnedTreeHandle {
            pid,
            group_isolated,
            session_isolated,
            identity,
        }
    };

    #[cfg(windows)]
    let handle = {
        let pid = child.id().unwrap_or(0);
        // Bind the freshly spawned shell into its own Job Object. Creating and
        // assigning by handle (rather than by PID) is what keeps the teardown
        // scoped to this tree; a failed creation/assignment leaves the tree
        // unowned and the direct-root kill is the fallback, so the failure is
        // logged rather than silently downgraded.
        let identity = (pid > 0).then(|| process_creation_time(pid)).flatten();
        let job = match create_kill_on_close_job() {
            Some(job) if assign_to_job(&job, &child) => Some(job),
            Some(_) => {
                log::warn!(
                    "[owned_process][platform=windows] failed to assign pid {pid} into its job object; falling back to a direct-root kill"
                );
                None
            }
            None => {
                log::warn!(
                    "[owned_process][platform=windows] failed to create a kill-on-close job object for pid {pid}; falling back to a direct-root kill"
                );
                None
            }
        };
        OwnedTreeHandle { pid, job, identity }
    };

    Ok(OwnedChild {
        child,
        handle,
        reaped: false,
    })
}

/// Runs one owned shell to completion or until the runtime cancels it.
///
/// stdout and stderr are collected concurrently with the root's exit and with
/// the cancellation signal, so a shell that exits while a descendant still holds
/// the inherited pipe open does not block the cancel path: cancellation still
/// reaches the tree and the root is reaped before `Cancelled` is returned.
pub async fn run_owned_shell(
    command_text: &str,
    cancel: &OwnedProcessCancellation,
) -> Result<OwnedShellOutcome, String> {
    let ticket = cancel.begin();
    let child = spawn_owned_shell(command_text)
        .map_err(|error| format!("Failed to execute shell command: {error}"))?;
    let mut run = OwnedShellRun {
        child: Some(child),
        ticket: Some(ticket),
        finished: false,
    };
    run.execute().await
}

/// The RAII owner of one running shell; its `Drop` hands an unfinished child to
/// a detached cleanup task, so dropping the run future never leaks the tree.
struct OwnedShellRun {
    child: Option<OwnedChild>,
    ticket: Option<OwnedProcessTicket>,
    finished: bool,
}

impl OwnedShellRun {
    async fn execute(&mut self) -> Result<OwnedShellOutcome, String> {
        let (cancelled, status, stdout, stderr) = self.drive().await?;
        if cancelled {
            if let Some(child) = self.child.as_mut() {
                child.terminate().await;
            }
            self.finished = true;
            return Ok(OwnedShellOutcome::Cancelled);
        }
        self.finished = true;
        let status = status
            .ok_or_else(|| "owned shell produced no exit status".to_string())?
            .map_err(|error| format!("Failed to wait for shell: {error}"))?;
        Ok(OwnedShellOutcome::Completed(std::process::Output {
            status,
            stdout,
            stderr,
        }))
    }

    /// Drives the exit, both output streams and the cancellation signal to a
    /// joint completion. Returns `(cancelled, status, stdout, stderr)`.
    #[allow(clippy::type_complexity)]
    async fn drive(
        &mut self,
    ) -> Result<
        (
            bool,
            Option<std::io::Result<std::process::ExitStatus>>,
            Vec<u8>,
            Vec<u8>,
        ),
        String,
    > {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| "owned shell child is missing".to_string())?;
        let ticket = self
            .ticket
            .as_mut()
            .ok_or_else(|| "owned shell ticket is missing".to_string())?;

        let mut stdout = child
            .take_stdout()
            .ok_or_else(|| "Failed to capture shell stdout".to_string())?;
        let mut stderr = child
            .take_stderr()
            .ok_or_else(|| "Failed to capture shell stderr".to_string())?;

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
        let mut cancelled = false;

        loop {
            tokio::select! {
                biased;
                _ = ticket.cancelled() => {
                    cancelled = true;
                    break;
                }
                result = child.wait(), if status.is_none() => {
                    status = Some(result);
                }
                read = stdout.read_to_end(&mut stdout_buf), if !stdout_eof => {
                    stdout_eof = true;
                    read.map_err(|error| format!("Failed to read shell stdout: {error}"))?;
                }
                read = stderr.read_to_end(&mut stderr_buf), if !stderr_eof => {
                    stderr_eof = true;
                    read.map_err(|error| format!("Failed to read shell stderr: {error}"))?;
                }
            }
            if status.is_some() && stdout_eof && stderr_eof {
                break;
            }
        }

        // On cancellation the buffers are intentionally dropped: the tree
        // teardown in `execute` closes the pipe a descendant may still hold.
        Ok((cancelled, status, stdout_buf, stderr_buf))
    }
}

impl Drop for OwnedShellRun {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Dropping the future is not a teardown: hand the still-owned child to a
        // detached task that terminates its tree and reaps it, moving the ticket
        // with it so the runtime drain still waits for the real cleanup.
        let Some(mut child) = self.child.take() else {
            return;
        };
        let ticket = self.ticket.take();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    child.terminate().await;
                    drop(ticket);
                });
            }
            Err(_) => {
                child.signal_tree_blocking();
                drop(child);
                drop(ticket);
            }
        }
    }
}

// --- unix process-tree signalling -----------------------------------------

#[cfg(unix)]
impl OwnedTreeHandle {
    /// Whether a negative-pid signal may still target this group. The leader
    /// identity is re-checked so a root that was reaped and whose PID was reused
    /// by an unrelated process is never signalled as if it were ours.
    fn group_signal_is_safe(&self) -> bool {
        if !self.group_isolated || self.pid <= 0 {
            return false;
        }
        match (self.identity, process_identity(self.pid)) {
            // The recorded leader still holds its PID with the same start time.
            (Some(recorded), Some(current)) => recorded == current,
            // The leader's PID is gone. The process-group id stays reserved
            // while any member remains and an empty group yields `ESRCH`, so a
            // negative-pid signal cannot reach an unrelated group.
            _ => true,
        }
    }

    fn signal_group(&self, signal: libc::c_int) {
        if self.group_signal_is_safe() {
            unsafe {
                libc::kill(-self.pid, signal);
            }
        }
    }

    fn signal_tree(&self) {
        if self.pid <= 0 {
            return;
        }
        let mut tracked = descendant_pids(self.pid);
        if self.session_isolated {
            tracked.extend(session_member_pids(self.pid));
        }
        tracked.retain(|pid| *pid > 0 && *pid != std::process::id() as libc::pid_t);
        tracked.dedup();

        signal_pids(&tracked, libc::SIGTERM);
        self.signal_group(libc::SIGTERM);

        let grace_start = Instant::now();
        let mut survivors = surviving_pids(&tracked);
        while !survivors.is_empty() && grace_start.elapsed() < SHELL_GRACE_BUDGET {
            std::thread::sleep(SHELL_FORCE_KILL_STEP);
            survivors = surviving_pids(&tracked);
        }

        let force_start = Instant::now();
        while !survivors.is_empty() && force_start.elapsed() < SHELL_FORCE_KILL_BUDGET {
            self.signal_group(libc::SIGKILL);
            signal_pids(&survivors, libc::SIGKILL);
            std::thread::sleep(SHELL_FORCE_KILL_STEP);
            survivors = surviving_pids(&tracked);
        }
        // Final unconditional sweep of whatever was still tracked.
        signal_pids(&survivors, libc::SIGKILL);
        self.signal_group(libc::SIGKILL);
    }
}

#[cfg(unix)]
fn signal_pids(process_ids: &[libc::pid_t], signal: libc::c_int) {
    for process_id in process_ids {
        if *process_id > 0 {
            unsafe {
                libc::kill(*process_id, signal);
            }
        }
    }
}

#[cfg(unix)]
fn surviving_pids(process_ids: &[libc::pid_t]) -> Vec<libc::pid_t> {
    process_ids
        .iter()
        .copied()
        .filter(|process_id| process_is_alive(*process_id))
        .collect()
}

#[cfg(unix)]
fn process_is_alive(process_id: libc::pid_t) -> bool {
    process_id > 0 && unsafe { libc::kill(process_id, 0) == 0 }
}

#[cfg(unix)]
fn process_group_of(process_id: libc::pid_t) -> Option<libc::pid_t> {
    let group = unsafe { libc::getpgid(process_id) };
    (group > 0).then_some(group)
}

#[cfg(unix)]
fn session_of(process_id: libc::pid_t) -> Option<libc::pid_t> {
    let session = unsafe { libc::getsid(process_id) };
    (session > 0).then_some(session)
}

/// The `(pid, ppid)` pairs visible to this process. Linux reads `/proc`
/// directly (no `ps`/procps dependency); other Unix uses `ps`.
#[cfg(target_os = "linux")]
fn process_parent_pairs() -> Vec<(libc::pid_t, libc::pid_t)> {
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
            let rest = stat.rsplit_once(") ")?.1;
            let mut fields = rest.split_whitespace();
            let _state = fields.next()?;
            let parent = fields.next()?.parse::<libc::pid_t>().ok()?;
            Some((process_id, parent))
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_parent_pairs() -> Vec<(libc::pid_t, libc::pid_t)> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse::<libc::pid_t>().ok()?,
                fields.next()?.parse::<libc::pid_t>().ok()?,
            ))
        })
        .collect()
}

/// The whole descendant tree rooted at `root`, discovered from parent links.
///
/// It is always rooted at the owned child, so it can never enumerate an
/// unrelated process. It supplements the group signal: it reaches a descendant
/// that left the owned group but kept the parent chain.
#[cfg(unix)]
fn descendant_pids(root: libc::pid_t) -> Vec<libc::pid_t> {
    let processes = process_parent_pairs();
    let mut members = Vec::new();
    let mut parents = vec![root];
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

/// Members of the owned child's own session. Only called when the child is
/// verified to be its own session leader, so it can never enumerate the
/// runtime's session.
#[cfg(target_os = "linux")]
fn session_member_pids(session_id: libc::pid_t) -> Vec<libc::pid_t> {
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
            let rest = stat.rsplit_once(") ")?.1;
            let mut fields = rest.split_whitespace();
            let _state = fields.next()?;
            let _ppid = fields.next()?;
            let _pgrp = fields.next()?;
            let process_session = fields.next()?.parse::<libc::pid_t>().ok()?;
            (process_session == session_id && process_id != std::process::id() as libc::pid_t)
                .then_some(process_id)
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn session_member_pids(session_id: libc::pid_t) -> Vec<libc::pid_t> {
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

/// The Linux start-time fingerprint of `process_id`, or `None` when the process
/// does not exist or the platform has no cheap fingerprint.
#[cfg(target_os = "linux")]
fn process_identity(process_id: libc::pid_t) -> Option<ProcessIdentity> {
    let stat = std::fs::read_to_string(format!("/proc/{process_id}/stat")).ok()?;
    let rest = stat.rsplit_once(") ")?.1;
    // From the state field onward, `starttime` is token 19 (field 22).
    let start_ticks = rest.split_whitespace().nth(19)?.parse::<u64>().ok()?;
    Some(ProcessIdentity { start_ticks })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_identity(_process_id: libc::pid_t) -> Option<ProcessIdentity> {
    None
}

// --- windows tree teardown -------------------------------------------------

/// One Job Object owned by a single spawned shell.
///
/// The handle is closed on drop; because the job was configured with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, closing it also terminates every
/// process still assigned to it — the safety net for a teardown that never ran.
#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// A kernel handle is valid process-wide and its operations are thread-safe, so
// the owned tree may move with its shell across the runtime's tasks.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Creates a Job Object configured to kill its whole tree when the handle
/// closes. Returns `None` when the object cannot be created or configured, so
/// the caller falls back to the direct-root kill instead of pretending the tree
/// is owned.
#[cfg(windows)]
fn create_kill_on_close_job() -> Option<JobHandle> {
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return None;
    }
    let job = JobHandle(handle);
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    (configured != 0).then_some(job)
}

/// Assigns an already spawned child into the job. `CREATE_SUSPENDED` is not
/// used, so a child that spawns a grandchild in the tiny window before this call
/// is the one escaped case the module documents as best-effort on Windows.
#[cfg(windows)]
fn assign_to_job(job: &JobHandle, child: &Child) -> bool {
    let Some(process) = child.raw_handle() else {
        return false;
    };
    unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(job.0, process) != 0 }
}

/// The creation-time fingerprint of `process_id`, or `None` when the process
/// cannot be opened (already exited, or access denied).
#[cfg(windows)]
fn process_creation_time(process_id: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if process_id == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() {
        return None;
    }
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(handle) };
    (ok != 0).then(|| ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
}

/// Processes still active in the job. A query failure is reported as `0` so the
/// bounded force loop always terminates.
#[cfg(windows)]
fn job_active_processes(job: &JobHandle) -> u32 {
    use windows_sys::Win32::System::JobObjects::{
        JobObjectBasicAccountingInformation, QueryInformationJobObject,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    };

    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        QueryInformationJobObject(
            job.0,
            JobObjectBasicAccountingInformation,
            &mut accounting as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        0
    } else {
        accounting.ActiveProcesses
    }
}

#[cfg(windows)]
impl OwnedTreeHandle {
    /// Whether the root's PID still belongs to the process this handle spawned.
    /// The job handle owns the tree; this guard exists only so the best-effort
    /// PID-based console signal never reaches a reused PID.
    fn root_identity_matches(&self) -> bool {
        match (self.identity, process_creation_time(self.pid)) {
            (Some(recorded), Some(current)) => recorded == current,
            _ => false,
        }
    }

    /// Terminates the whole owned tree: a best-effort graceful phase, then a
    /// forced job termination bounded by the shared budgets.
    ///
    /// Windows has no `SIGTERM`; the graceful phase is a console break to the
    /// identity-verified root, which a `CREATE_NO_WINDOW` shell does not share a
    /// console with, so in practice the forced `TerminateJobObject` reaps the
    /// tree. Ownership is by kernel handle, so neither phase can reach an
    /// unrelated process by name or by a reused global PID.
    fn terminate_tree(&self) {
        use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};

        let Some(job) = self.job.as_ref() else {
            return;
        };
        if self.root_identity_matches() {
            unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid) };
        }
        let grace_start = Instant::now();
        while job_active_processes(job) > 0 && grace_start.elapsed() < SHELL_GRACE_BUDGET {
            std::thread::sleep(SHELL_FORCE_KILL_STEP);
        }
        let force_start = Instant::now();
        loop {
            unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0, 1) };
            if job_active_processes(job) == 0 || force_start.elapsed() >= SHELL_FORCE_KILL_BUDGET {
                break;
            }
            std::thread::sleep(SHELL_FORCE_KILL_STEP);
        }
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0, 1) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::path::Path;

    /// Polls a pid fixture written by a spawned shell. The fixture is the only
    /// way to observe a descendant the module deliberately does not expose.
    #[cfg(unix)]
    async fn wait_for_pid_fixture(path: &Path) -> libc::pid_t {
        for _ in 0..500 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<libc::pid_t>() {
                    return pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("pid fixture {} was never written", path.display());
    }

    /// A child launched by an owned shell must lead its own process group and
    /// its own session, and must never be the test process itself. This is the
    /// precondition that makes a negative-pid teardown scoped to the owned tree
    /// instead of to the shared test/dev group.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_spawns_a_private_group_and_session() {
        let mut child = spawn_owned_shell("sleep 30").expect("spawn owned shell");
        let pid = child.handle.pid;
        assert!(pid > 0, "owned shell must have a pid");
        assert_ne!(
            pid,
            std::process::id() as libc::pid_t,
            "must not be this process"
        );
        assert!(
            child.handle.group_isolated,
            "owned shell must lead its own process group"
        );
        assert!(
            child.handle.session_isolated,
            "owned shell must lead its own session"
        );
        assert_eq!(process_group_of(pid), Some(pid));
        assert_eq!(session_of(pid), Some(pid));
        // Tear the tree down without ever signalling anything but its group.
        child.terminate().await;
        assert!(!process_is_alive(pid));
    }

    /// Cancelling an in-flight owned shell must terminate the whole owned tree
    /// (root plus a grandchild that wrote a pid fixture) and reap the root
    /// inside a short budget.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_cancel_terminates_the_whole_tree_and_reaps_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        let grandchild_fixture = dir.path().join("grandchild.pid");
        let command = format!(
            "sleep 30 & echo $! > {}; sleep 30",
            grandchild_fixture.display()
        );

        let cancel = OwnedProcessCancellation::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_owned_shell(&command, &cancel).await }
        });

        let grandchild = wait_for_pid_fixture(&grandchild_fixture).await;
        assert!(grandchild > 0);
        assert_ne!(grandchild, std::process::id() as libc::pid_t);
        assert!(process_is_alive(grandchild), "grandchild must be running");
        assert!(cancel.in_flight() >= 1, "the run must be in flight");

        let started = Instant::now();
        cancel.cancel_all_and_wait().await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel teardown must finish inside a short budget"
        );

        let outcome = task.await.expect("run task").expect("run result");
        assert!(matches!(outcome, OwnedShellOutcome::Cancelled));
        assert_eq!(cancel.in_flight(), 0, "the ticket must be released");
        assert!(
            !process_is_alive(grandchild),
            "the grandchild must be terminated"
        );
    }

    /// A shell that ignores `SIGTERM` must still be force-killed by the
    /// `SIGKILL` sweep within the budget.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_ignoring_termination_is_force_killed() {
        let cancel = OwnedProcessCancellation::new();
        // The trap makes the shell itself ignore TERM; only SIGKILL ends it.
        let command = "trap '' TERM; sleep 30";
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_owned_shell(command, &cancel).await }
        });

        // Give the shell time to install the trap before cancelling.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = Instant::now();
        cancel.cancel_all_and_wait().await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "forced teardown must finish inside the budget"
        );

        let outcome = task.await.expect("run task").expect("run result");
        assert!(matches!(outcome, OwnedShellOutcome::Cancelled));
        assert_eq!(cancel.in_flight(), 0);
    }

    /// A root that exits while a descendant keeps the inherited stdout pipe open
    /// must not block cancellation: the cancel path still tears the tree down.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_root_exit_with_descendant_pipe_is_cancelled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let grandchild_fixture = dir.path().join("grandchild.pid");
        // The backgrounded sleep inherits stdout, so it holds the pipe open
        // after the shell itself exits.
        let command = format!(
            "sleep 30 & echo $! > {}; exit 0",
            grandchild_fixture.display()
        );

        let cancel = OwnedProcessCancellation::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_owned_shell(&command, &cancel).await }
        });

        let grandchild = wait_for_pid_fixture(&grandchild_fixture).await;
        // The root has exited; the descendant still holds the pipe.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(process_is_alive(grandchild));

        let started = Instant::now();
        cancel.cancel_all_and_wait().await;
        assert!(started.elapsed() < Duration::from_secs(5));

        let outcome = task.await.expect("run task").expect("run result");
        assert!(matches!(outcome, OwnedShellOutcome::Cancelled));
        assert!(
            !process_is_alive(grandchild),
            "the pipe-holding descendant must be terminated"
        );
    }

    /// Cancelling one owned shell must never reach an unrelated process that is
    /// not part of its tree — the safety property the whole design exists for.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_cancel_leaves_an_unrelated_sentinel_alive() {
        // The sentinel is in the test's own process group and is never a
        // descendant of the owned shell. A bug that signalled the shared group
        // would kill it (and the test process), so its survival is the proof.
        let mut sentinel = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sentinel");
        let sentinel_pid = sentinel.id() as libc::pid_t;

        let cancel = OwnedProcessCancellation::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_owned_shell("sleep 30", &cancel).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel_all_and_wait().await;
        let outcome = task.await.expect("run task").expect("run result");
        assert!(matches!(outcome, OwnedShellOutcome::Cancelled));

        assert!(
            process_is_alive(sentinel_pid),
            "an unrelated process must survive an owned cancel"
        );

        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }

    /// Dropping the run future must not leak the child: the RAII guard hands it
    /// to a detached cleanup task that terminates the tree and releases the
    /// in-flight ticket the shutdown drain waits on.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_dropped_future_cleans_up_and_releases() {
        let dir = tempfile::tempdir().expect("temp dir");
        let grandchild_fixture = dir.path().join("grandchild.pid");
        let command = format!(
            "sleep 30 & echo $! > {}; sleep 30",
            grandchild_fixture.display()
        );

        let cancel = OwnedProcessCancellation::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_owned_shell(&command, &cancel).await }
        });

        let grandchild = wait_for_pid_fixture(&grandchild_fixture).await;
        assert!(cancel.in_flight() >= 1);

        // Drop the future mid-flight; this is not cleanup by itself.
        task.abort();
        let _ = task.await;

        let deadline = Instant::now() + Duration::from_secs(5);
        while cancel.in_flight() > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            cancel.in_flight(),
            0,
            "the guard's detached cleanup must release the ticket"
        );
        assert!(
            !process_is_alive(grandchild),
            "the guard's detached cleanup must terminate the tree"
        );
    }

    #[tokio::test]
    async fn runtime_shutdown_owned_wait_idle_resolves_without_in_flight_work() {
        let cancel = OwnedProcessCancellation::new();
        // With no owned shell there is nothing to wait for.
        cancel.wait_idle().await;
        assert_eq!(cancel.in_flight(), 0);
        // A ticket that is dropped releases the hold, so the drain resolves.
        let ticket = cancel.begin();
        drop(ticket);
        cancel.wait_idle().await;
        assert_eq!(cancel.in_flight(), 0);
    }

    /// The descendant enumeration reaches a grandchild of the owned shell and
    /// scopes it to the owned session — a real process-tree test that does not
    /// rely on the group signal to find the descendant.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn runtime_shutdown_owned_descendant_enumeration_reaches_the_grandchild() {
        let mut child =
            spawn_owned_shell("sleep 30 & echo $! ; sleep 30").expect("spawn owned shell");
        let pid = child.handle.pid;

        let mut grandchild = None;
        for _ in 0..300 {
            if let Some(first) = descendant_pids(pid).first().copied() {
                grandchild = Some(first);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let grandchild = grandchild.expect("the owned shell's descendant must be enumerated");
        assert_ne!(grandchild, pid);
        assert_ne!(grandchild, std::process::id() as libc::pid_t);
        // The enumerated descendant belongs to the owned session, never ours.
        assert_eq!(session_of(grandchild), Some(pid));

        child.terminate().await;
        assert!(!process_is_alive(grandchild));
    }

    /// A group signal is refused unless the child was verified to lead its own
    /// process group. This is the guard that keeps a backend which failed to
    /// isolate from escalating a teardown into a collateral group kill.
    #[cfg(unix)]
    #[test]
    fn runtime_shutdown_owned_group_signal_requires_a_verified_isolated_group() {
        let unverified = OwnedTreeHandle {
            pid: std::process::id() as libc::pid_t,
            group_isolated: false,
            session_isolated: false,
            identity: None,
        };
        assert!(
            !unverified.group_signal_is_safe(),
            "an unverified group must never be signalled"
        );
    }

    /// The PID-reuse guard: a recorded leader identity that no longer matches the
    /// live pid refuses the group signal, while the recorded identity allows it.
    /// Neither branch signals anything.
    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_shutdown_owned_group_signal_is_refused_when_the_leader_identity_changed() {
        let pid = std::process::id() as libc::pid_t;
        let live = process_identity(pid).expect("this process has a fingerprint");

        let reused = OwnedTreeHandle {
            pid,
            group_isolated: true,
            session_isolated: false,
            identity: Some(ProcessIdentity {
                start_ticks: live.start_ticks.wrapping_add(1),
            }),
        };
        assert!(
            !reused.group_signal_is_safe(),
            "a reused pid must not be signalled as if it were the owned leader"
        );

        let owned = OwnedTreeHandle {
            pid,
            group_isolated: true,
            session_isolated: false,
            identity: Some(live),
        };
        assert!(
            owned.group_signal_is_safe(),
            "the recorded leader identity permits the group signal"
        );
    }

    /// A Job Object created for a shell must be configurable and start empty, so
    /// the tree it later holds is the one it owns. Windows-only: it compiles
    /// under the `x86_64-pc-windows-*` target check and runs only on a Windows
    /// runner.
    #[cfg(windows)]
    #[test]
    fn runtime_shutdown_owned_creates_a_kill_on_close_job() {
        let job = create_kill_on_close_job().expect("a job object must be creatable");
        assert_eq!(
            job_active_processes(&job),
            0,
            "a freshly created job owns no process yet"
        );
    }

    /// The recorded root creation time must accept the live root and refuse a
    /// fabricated/stale one, so the best-effort PID-based graceful signal cannot
    /// reach a reused PID. The job handle owns the tree; this is the secondary
    /// guard.
    #[cfg(windows)]
    #[test]
    fn runtime_shutdown_owned_root_identity_is_checked_against_a_reused_pid() {
        let pid = std::process::id();
        let identity = process_creation_time(pid).expect("this process has a creation time");

        let matching = OwnedTreeHandle {
            pid,
            job: None,
            identity: Some(identity),
        };
        assert!(matching.root_identity_matches());

        let reused = OwnedTreeHandle {
            pid,
            job: None,
            identity: Some(identity.wrapping_add(1)),
        };
        assert!(
            !reused.root_identity_matches(),
            "a reused pid must never receive the owned graceful signal"
        );

        let unknown = OwnedTreeHandle {
            pid,
            job: None,
            identity: None,
        };
        assert!(!unknown.root_identity_matches());
    }

    /// A spawned shell must be bound into its own Job Object and torn down (and
    /// reaped) by the job, not by a name or global PID. Windows-only: it type
    /// checks under the target check and runs only on a Windows runner.
    #[cfg(windows)]
    #[tokio::test]
    async fn runtime_shutdown_owned_job_terminates_the_tree_and_reaps_the_root() {
        let mut child = spawn_owned_shell("ping -n 30 127.0.0.1 > NUL").expect("spawn owned shell");
        assert!(
            child.handle.job.is_some(),
            "the shell must be bound into its own job object"
        );
        child.terminate().await;
        assert!(child.reaped);
    }
}
