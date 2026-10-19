//! Single-instance quit coordinator for the desktop tray entry point.
//!
//! The tray `quit` entry is the only route that starts a confirmation. The
//! coordinator owns the state machine
//! `Idle -> Confirming(nonce, host) -> ExitingDesktop | StoppingRuntime ->
//! CommittingExit`, the nonce/host binding that every dialog submit is checked
//! against, and the once-only terminal commit. Every transition is idempotent:
//! a repeated tray click, a duplicate submit, a mismatched nonce/host, a cancel
//! or a timeout can never start a second request or a second commit.
//!
//! The module is deliberately transport-neutral and holds no runtime handle: it
//! records *what* was decided, never *how* the runtime or the lease is touched.
//! The Tauri command layer asks this machine for permission first and only then
//! releases the lease, requests the coordinated runtime shutdown or performs the
//! terminal `app.exit`.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{Emitter, Manager};

/// Window label that hosts the confirmation dialog.
pub const QUIT_HOST_WINDOW_LABEL: &str = "workflow";

/// How long a pending confirmation stays open before it returns to `Idle`.
pub const QUIT_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound on releasing the desktop's runtime lease on the exit-desktop path.
pub const QUIT_RELEASE_BUDGET: Duration = Duration::from_secs(3);

/// Upper bound on observing the coordinated runtime shutdown reach `stopped`.
///
/// The runtime enforces its own graceful and forced budgets; this is only the
/// desktop's outer wait, so a stop the runtime never reports cannot hang the
/// dialog forever.
pub const QUIT_STOP_BUDGET: Duration = Duration::from_secs(30);

/// Event published when a quit prompt must reach an already-mounted dialog host.
pub const QUIT_REQUEST_EVENT: &str = "quit://request";

/// Event publishing the coordinator status after a submit, cancel or failure.
pub const QUIT_STATUS_EVENT: &str = "quit://status";

/// Stable phase names shared with the frontend.
pub const QUIT_PHASE_IDLE: &str = "idle";
/// A confirmation is open and waiting for the user.
pub const QUIT_PHASE_CONFIRMING: &str = "confirming";
/// A coordinated runtime shutdown was requested and is being observed.
pub const QUIT_PHASE_STOPPING_RUNTIME: &str = "stopping_runtime";
/// The desktop-only exit was chosen and is about to commit.
pub const QUIT_PHASE_EXITING_DESKTOP: &str = "exiting_desktop";
/// The terminal commit already ran; no further transition is accepted.
pub const QUIT_PHASE_COMMITTING_EXIT: &str = "committing_exit";

/// Stable error code for a runtime that is not connected to this desktop.
pub const QUIT_ERROR_RUNTIME_NOT_CONNECTED: &str = "runtime_not_connected";
/// Stable error code for a runtime instance that is not the saved target.
pub const QUIT_ERROR_RUNTIME_INSTANCE_CHANGED: &str = "runtime_instance_changed";

/// Phase of the single-instance quit machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitPhase {
    Idle,
    Confirming,
    StoppingRuntime,
    ExitingDesktop,
    CommittingExit,
}

impl QuitPhase {
    /// Wire name of the phase, shared with the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => QUIT_PHASE_IDLE,
            Self::Confirming => QUIT_PHASE_CONFIRMING,
            Self::StoppingRuntime => QUIT_PHASE_STOPPING_RUNTIME,
            Self::ExitingDesktop => QUIT_PHASE_EXITING_DESKTOP,
            Self::CommittingExit => QUIT_PHASE_COMMITTING_EXIT,
        }
    }
}

/// The nonce/host pair binding a dialog to exactly one quit request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitPrompt {
    /// Opaque, per-request identifier every submit must echo back.
    pub nonce: String,
    /// Window label the dialog is rendered in.
    pub host: String,
}

/// Redacted coordinator snapshot handed to the dialog host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitStatus {
    /// Current phase name.
    pub phase: String,
    /// The pending prompt while a confirmation is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<QuitPrompt>,
    /// Stable error code; the frontend maps it to a localized message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of a tray-initiated quit request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitRequestOutcome {
    /// A fresh confirmation was opened with a new nonce.
    Started(QuitPrompt),
    /// A confirmation was already open; focus its host and reuse its nonce.
    FocusHost(QuitPrompt),
    /// A runtime stop or desktop exit is already in flight; never start another.
    InProgress(QuitPrompt),
    /// The terminal commit already ran.
    Committed,
}

/// Whether a submitted transition changed anything.
///
/// `Ignored` is the answer for a duplicated or stale submit (a wrong nonce, a
/// wrong host or a phase that is no longer accepting it), so a caller can treat
/// it as a no-op instead of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitTransition {
    Applied,
    Ignored,
}

/// Mutable state of the quit machine.
struct QuitState {
    phase: QuitPhase,
    nonce: Option<String>,
    host: Option<String>,
    /// Runtime instance id the accepted stop targets, recorded once so a retry
    /// never targets a different (successor) instance.
    stop_instance_id: Option<String>,
    /// Pid recorded alongside the target instance.
    stop_pid: Option<u32>,
    /// Stable error code of the last failed stop attempt.
    error: Option<String>,
    /// Whether the terminal commit already ran.
    committed: bool,
    /// Monotonic counter making each nonce unique within this process.
    nonce_counter: u64,
}

impl QuitState {
    fn idle() -> Self {
        Self {
            phase: QuitPhase::Idle,
            nonce: None,
            host: None,
            stop_instance_id: None,
            stop_pid: None,
            error: None,
            committed: false,
            nonce_counter: 0,
        }
    }

    fn prompt(&self) -> Option<QuitPrompt> {
        match (&self.nonce, &self.host) {
            (Some(nonce), Some(host)) => Some(QuitPrompt {
                nonce: nonce.clone(),
                host: host.clone(),
            }),
            _ => None,
        }
    }

    /// Clears every field that belongs to one prompt, leaving the counter and the
    /// committed flag untouched so nonces stay unique and the commit stays final.
    fn reset_prompt(&mut self) {
        self.phase = QuitPhase::Idle;
        self.nonce = None;
        self.host = None;
        self.stop_instance_id = None;
        self.stop_pid = None;
        self.error = None;
    }
}

/// The single per-process quit coordinator.
pub struct ExitCoordinator {
    inner: Mutex<QuitState>,
}

impl Default for ExitCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl ExitCoordinator {
    /// Creates an idle coordinator.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QuitState::idle()),
        }
    }

    /// Locks the machine, recovering a poisoned guard instead of panicking.
    fn lock(&self) -> MutexGuard<'_, QuitState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Current phase, used by the command layer before it touches any runtime.
    pub fn phase(&self) -> QuitPhase {
        self.lock().phase
    }

    /// Redacted snapshot for the dialog host.
    pub fn status(&self) -> QuitStatus {
        let state = self.lock();
        QuitStatus {
            phase: state.phase.as_str().to_string(),
            prompt: state.prompt(),
            error: state.error.clone(),
        }
    }

    /// The pending prompt while a confirmation (or an in-flight stop) exists.
    pub fn pending_prompt(&self) -> Option<QuitPrompt> {
        let state = self.lock();
        match state.phase {
            QuitPhase::Confirming | QuitPhase::StoppingRuntime | QuitPhase::ExitingDesktop => {
                state.prompt()
            }
            _ => None,
        }
    }

    /// Registers a tray quit request.
    ///
    /// Only an `Idle` machine opens a new confirmation. Any other phase reuses
    /// the existing nonce and host, so a second tray click focuses the same host
    /// without starting a second request.
    pub fn request_quit(&self, host: &str) -> QuitRequestOutcome {
        let mut state = self.lock();
        match state.phase {
            QuitPhase::Idle => {
                state.nonce_counter += 1;
                state.nonce = Some(next_nonce(state.nonce_counter));
                state.host = Some(host.to_string());
                state.stop_instance_id = None;
                state.stop_pid = None;
                state.error = None;
                state.phase = QuitPhase::Confirming;
                QuitRequestOutcome::Started(QuitPrompt {
                    nonce: state.nonce.clone().unwrap_or_default(),
                    host: host.to_string(),
                })
            }
            QuitPhase::Confirming => match state.prompt() {
                Some(prompt) => QuitRequestOutcome::FocusHost(prompt),
                None => QuitRequestOutcome::Committed,
            },
            QuitPhase::StoppingRuntime | QuitPhase::ExitingDesktop => match state.prompt() {
                Some(prompt) => QuitRequestOutcome::InProgress(prompt),
                None => QuitRequestOutcome::Committed,
            },
            QuitPhase::CommittingExit => QuitRequestOutcome::Committed,
        }
    }

    /// Moves a matching `Confirming` prompt to the desktop-only exit path.
    pub fn confirm_exit_desktop(&self, nonce: &str, host: &str) -> QuitTransition {
        let mut state = self.lock();
        if state.phase == QuitPhase::Confirming && prompt_matches(&state, nonce, host) {
            state.phase = QuitPhase::ExitingDesktop;
            return QuitTransition::Applied;
        }
        QuitTransition::Ignored
    }

    /// Moves a matching `Confirming` prompt to the runtime-stop path.
    pub fn confirm_stop_runtime(&self, nonce: &str, host: &str) -> QuitTransition {
        let mut state = self.lock();
        if state.phase == QuitPhase::Confirming && prompt_matches(&state, nonce, host) {
            state.phase = QuitPhase::StoppingRuntime;
            return QuitTransition::Applied;
        }
        QuitTransition::Ignored
    }

    /// Clears a recorded stop failure so the stop may be retried.
    pub fn begin_stop_retry(&self, nonce: &str, host: &str) -> QuitTransition {
        let mut state = self.lock();
        if state.phase == QuitPhase::StoppingRuntime && prompt_matches(&state, nonce, host) {
            state.error = None;
            return QuitTransition::Applied;
        }
        QuitTransition::Ignored
    }

    /// Abandons a failed runtime stop and takes the desktop-only exit instead.
    pub fn exit_desktop_after_stop(&self, nonce: &str, host: &str) -> QuitTransition {
        let mut state = self.lock();
        if state.phase == QuitPhase::StoppingRuntime && prompt_matches(&state, nonce, host) {
            state.error = None;
            state.phase = QuitPhase::ExitingDesktop;
            return QuitTransition::Applied;
        }
        QuitTransition::Ignored
    }

    /// Records the runtime instance a stop targets.
    ///
    /// It is recorded once and never overwritten by a later observation, so a
    /// retry keeps addressing the instance the user confirmed instead of a
    /// successor that may have restarted in between.
    pub fn set_stop_target(&self, instance_id: &str, pid: u32) {
        let mut state = self.lock();
        if state.stop_instance_id.is_none() {
            state.stop_instance_id = Some(instance_id.to_string());
            state.stop_pid = Some(pid);
        }
    }

    /// Returns the recorded stop target, if one exists.
    pub fn stop_target(&self) -> Option<(String, u32)> {
        let state = self.lock();
        match (&state.stop_instance_id, state.stop_pid) {
            (Some(instance_id), Some(pid)) => Some((instance_id.clone(), pid)),
            _ => None,
        }
    }

    /// Records a stable failure code while the stop stays retryable.
    pub fn set_stop_failure(&self, code: &str) {
        let mut state = self.lock();
        if state.phase == QuitPhase::StoppingRuntime {
            state.error = Some(code.to_string());
        }
    }

    /// Cancels a matching `Confirming` prompt and returns the machine to `Idle`.
    pub fn cancel(&self, nonce: &str, host: &str) -> QuitTransition {
        let mut state = self.lock();
        if state.phase == QuitPhase::Confirming && prompt_matches(&state, nonce, host) {
            state.reset_prompt();
            return QuitTransition::Applied;
        }
        QuitTransition::Ignored
    }

    /// Expires a matching `Confirming` prompt after the prompt timeout.
    pub fn expire_prompt(&self, nonce: &str, host: &str) -> QuitTransition {
        self.cancel(nonce, host)
    }

    /// Claims the terminal commit exactly once.
    ///
    /// Allowed from the desktop-only exit and from a coordinated stop that has
    /// not recorded a failure; a failed stop must be retried or abandoned first,
    /// so a failure can never silently become a successful exit.
    pub fn commit_once(&self) -> bool {
        let mut state = self.lock();
        if state.committed {
            return false;
        }
        let allowed = match state.phase {
            QuitPhase::ExitingDesktop => true,
            QuitPhase::StoppingRuntime => state.error.is_none(),
            QuitPhase::Idle | QuitPhase::Confirming | QuitPhase::CommittingExit => false,
        };
        if !allowed {
            return false;
        }
        state.committed = true;
        state.phase = QuitPhase::CommittingExit;
        true
    }
}

/// Whether a submit's nonce and host match the open prompt.
fn prompt_matches(state: &QuitState, nonce: &str, host: &str) -> bool {
    state.nonce.as_deref() == Some(nonce) && state.host.as_deref() == Some(host)
}

/// Builds a unique-enough prompt nonce without pulling in a dependency.
///
/// The wall clock separates prompts across restarts and the counter separates
/// prompts within one process, so a stale nonce from an earlier prompt cannot
/// match a later one.
fn next_nonce(counter: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("quit-{nanos:x}-{counter:x}")
}

/// Registers a quit request from the tray or a window menu and publishes it to the dialog host.
///
/// The workflow host window is created or focused first, then the prompt is
/// published. A host that mounts afterwards learns about the same prompt through
/// its `quit_status` query, so an event emitted before its listener exists is
/// never the only handshake.
pub fn request_quit(app: &tauri::AppHandle) {
    let coordinator = match app.try_state::<std::sync::Arc<ExitCoordinator>>() {
        Some(state) => state.inner().clone(),
        None => {
            log::warn!("[ExitCoordinator] quit requested before the coordinator was registered");
            return;
        }
    };

    let started = matches!(
        coordinator.request_quit(QUIT_HOST_WINDOW_LABEL),
        QuitRequestOutcome::Started(_)
    );

    // Focus (or create) the only dialog host. This reuses the existing workflow
    // window; no separate native window is created for the dialog.
    crate::window::show_and_focus_window(app, QUIT_HOST_WINDOW_LABEL);

    let Some(prompt) = coordinator.pending_prompt() else {
        return;
    };
    if let Err(error) = app.emit(QUIT_REQUEST_EVENT, prompt.clone()) {
        log::warn!("[ExitCoordinator] publishing the quit prompt failed: {error}");
    }
    if started {
        spawn_prompt_timeout(app.clone(), coordinator, prompt);
    }
}

/// Returns a matching pending prompt to `Idle` (and notifies the dialog) on timeout.
fn spawn_prompt_timeout(
    app: tauri::AppHandle,
    coordinator: std::sync::Arc<ExitCoordinator>,
    prompt: QuitPrompt,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(QUIT_PROMPT_TIMEOUT).await;
        if coordinator.expire_prompt(&prompt.nonce, &prompt.host) != QuitTransition::Applied {
            return;
        }
        if let Err(error) = app.emit(QUIT_STATUS_EVENT, coordinator.status()) {
            log::warn!("[ExitCoordinator] publishing the expired prompt failed: {error}");
        }
    });
}

/// Performs the terminal application exit exactly once.
///
/// The claim is the coordinator's, so a duplicate commit attempt (a second
/// dialog submit, a tray click after the commit or a direct quit) is a no-op
/// instead of a second `app.exit`.
pub fn commit_exit(app: &tauri::AppHandle, coordinator: &ExitCoordinator) {
    if coordinator.commit_once() {
        log::info!("[ExitCoordinator] committing the terminal application exit");
        app.exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "workflow";

    fn started_nonce(outcome: &QuitRequestOutcome) -> String {
        match outcome {
            QuitRequestOutcome::Started(prompt) => prompt.nonce.clone(),
            other => panic!("expected a started prompt, got {other:?}"),
        }
    }

    #[test]
    fn first_request_starts_confirming_with_a_fresh_nonce() {
        let coordinator = ExitCoordinator::new();
        assert_eq!(coordinator.phase(), QuitPhase::Idle);

        let outcome = coordinator.request_quit(HOST);
        let nonce = started_nonce(&outcome);
        assert!(!nonce.is_empty());
        assert_eq!(coordinator.phase(), QuitPhase::Confirming);
        assert_eq!(
            coordinator.pending_prompt(),
            Some(QuitPrompt {
                nonce: nonce.clone(),
                host: HOST.to_string()
            })
        );
    }

    #[test]
    fn a_repeated_tray_click_focuses_the_same_prompt_without_a_new_request() {
        let coordinator = ExitCoordinator::new();
        let first = started_nonce(&coordinator.request_quit(HOST));

        let second = coordinator.request_quit(HOST);
        assert_eq!(
            second,
            QuitRequestOutcome::FocusHost(QuitPrompt {
                nonce: first.clone(),
                host: HOST.to_string()
            })
        );
        // The nonce is unchanged, so a later submit still binds to the same prompt.
        assert_eq!(coordinator.pending_prompt().map(|p| p.nonce), Some(first));
    }

    #[test]
    fn a_stop_in_progress_never_starts_a_second_request() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));
        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Applied
        );

        let outcome = coordinator.request_quit(HOST);
        assert_eq!(
            outcome,
            QuitRequestOutcome::InProgress(QuitPrompt {
                nonce: nonce.clone(),
                host: HOST.to_string()
            })
        );
    }

    #[test]
    fn a_wrong_nonce_or_host_is_ignored() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));

        assert_eq!(
            coordinator.confirm_stop_runtime("not-the-nonce", HOST),
            QuitTransition::Ignored
        );
        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, "assistant"),
            QuitTransition::Ignored
        );
        assert_eq!(
            coordinator.confirm_exit_desktop(&nonce, "assistant"),
            QuitTransition::Ignored
        );
        assert_eq!(
            coordinator.cancel("not-the-nonce", HOST),
            QuitTransition::Ignored
        );
        // None of the mismatched submits changed the phase.
        assert_eq!(coordinator.phase(), QuitPhase::Confirming);
    }

    #[test]
    fn a_duplicate_submit_is_a_noop() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));

        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Applied
        );
        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Ignored
        );
        assert_eq!(
            coordinator.confirm_exit_desktop(&nonce, HOST),
            QuitTransition::Ignored
        );
    }

    #[test]
    fn cancel_returns_to_idle_and_a_reopen_starts_unchecked() {
        let coordinator = ExitCoordinator::new();
        let first = started_nonce(&coordinator.request_quit(HOST));
        assert_eq!(coordinator.cancel(&first, HOST), QuitTransition::Applied);
        assert_eq!(coordinator.phase(), QuitPhase::Idle);
        assert_eq!(coordinator.pending_prompt(), None);

        let second = started_nonce(&coordinator.request_quit(HOST));
        assert_ne!(first, second, "a reopened prompt gets a new nonce");
        assert_eq!(coordinator.phase(), QuitPhase::Confirming);
    }

    #[test]
    fn a_timed_out_prompt_returns_to_idle_and_a_stale_submit_is_ignored() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));

        assert_eq!(
            coordinator.expire_prompt(&nonce, HOST),
            QuitTransition::Applied
        );
        assert_eq!(coordinator.phase(), QuitPhase::Idle);
        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Ignored
        );
        assert_eq!(
            coordinator.expire_prompt(&nonce, HOST),
            QuitTransition::Ignored
        );
    }

    #[test]
    fn the_commit_runs_once_and_the_resulting_request_is_committed() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));
        assert_eq!(
            coordinator.confirm_exit_desktop(&nonce, HOST),
            QuitTransition::Applied
        );

        assert!(coordinator.commit_once(), "the first commit is claimed");
        assert!(!coordinator.commit_once(), "the second commit is a no-op");
        assert_eq!(coordinator.phase(), QuitPhase::CommittingExit);
        assert_eq!(
            coordinator.request_quit(HOST),
            QuitRequestOutcome::Committed
        );
    }

    #[test]
    fn a_failed_stop_cannot_commit_until_it_is_retried_or_abandoned() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));
        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Applied
        );
        coordinator.set_stop_target("instance-a", 42);
        coordinator.set_stop_failure("runtime_stop_timeout");

        assert!(
            !coordinator.commit_once(),
            "a failed stop is not a successful exit"
        );
        assert_eq!(
            coordinator.begin_stop_retry(&nonce, HOST),
            QuitTransition::Applied
        );
        assert_eq!(coordinator.status().error, None);

        // A second failure, then the explicit desktop-only exit.
        coordinator.set_stop_failure("runtime_stop_timeout");
        assert_eq!(
            coordinator.exit_desktop_after_stop(&nonce, HOST),
            QuitTransition::Applied
        );
        assert_eq!(coordinator.phase(), QuitPhase::ExitingDesktop);
        assert!(coordinator.commit_once());
    }

    #[test]
    fn the_stop_target_is_recorded_once_and_never_overwritten() {
        let coordinator = ExitCoordinator::new();
        coordinator.set_stop_target("instance-a", 42);
        coordinator.set_stop_target("instance-b", 99);
        assert_eq!(
            coordinator.stop_target(),
            Some(("instance-a".to_string(), 42))
        );
    }

    #[test]
    fn a_direct_quit_prepares_and_commits_once() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));
        assert_eq!(
            coordinator.confirm_exit_desktop(&nonce, HOST),
            QuitTransition::Applied
        );
        assert!(coordinator.commit_once());
        assert!(!coordinator.commit_once());
    }

    #[test]
    fn idle_never_commits_without_a_prompt() {
        let coordinator = ExitCoordinator::new();
        assert!(!coordinator.commit_once());
        assert_eq!(coordinator.phase(), QuitPhase::Idle);
    }

    #[test]
    fn status_serializes_the_phase_prompt_and_error_code() {
        let coordinator = ExitCoordinator::new();
        let nonce = started_nonce(&coordinator.request_quit(HOST));
        let confirming = serde_json::to_value(coordinator.status()).expect("serialize status");
        assert_eq!(confirming["phase"], "confirming");
        assert_eq!(confirming["prompt"]["nonce"], nonce);
        assert_eq!(confirming["prompt"]["host"], HOST);
        assert!(confirming.get("error").is_none());

        assert_eq!(
            coordinator.confirm_stop_runtime(&nonce, HOST),
            QuitTransition::Applied
        );
        coordinator.set_stop_failure(QUIT_ERROR_RUNTIME_NOT_CONNECTED);
        let stopping = serde_json::to_value(coordinator.status()).expect("serialize status");
        assert_eq!(stopping["phase"], "stopping_runtime");
        assert_eq!(stopping["error"], QUIT_ERROR_RUNTIME_NOT_CONNECTED);
    }
}
