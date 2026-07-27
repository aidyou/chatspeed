use crate::workflow::react::types::WorkflowState;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::LazyLock;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::{Child, Command, Stdio};

/// Tracks the workflow sessions that currently need the host to stay awake.
///
/// Only states that can advance without a user action belong here. Waiting for approval,
/// `ask_user`, confirmation, or a sub-agent intentionally does not hold an assertion.
#[derive(Default)]
struct ActiveWorkflowSessions {
    enabled: bool,
    session_ids: HashSet<String>,
}

impl ActiveWorkflowSessions {
    fn should_inhibit(&self) -> bool {
        self.enabled && !self.session_ids.is_empty()
    }

    fn set_enabled(&mut self, enabled: bool) -> bool {
        self.enabled = enabled;
        self.should_inhibit()
    }

    fn set_session_active(&mut self, session_id: &str, active: bool) -> bool {
        if active {
            self.session_ids.insert(session_id.to_string());
        } else {
            self.session_ids.remove(session_id);
        }
        self.should_inhibit()
    }

    fn remove_session(&mut self, session_id: &str) -> bool {
        self.session_ids.remove(session_id);
        self.should_inhibit()
    }
}

impl WorkflowState {
    /// Returns whether this workflow state may make autonomous progress.
    ///
    /// This deliberately excludes all waiting and terminal states. A workflow waiting for a
    /// user answer, approval, confirmation, or sub-agent result cannot continue by itself, so it
    /// must not keep the computer awake after the user leaves.
    pub fn prevents_idle_sleep(&self) -> bool {
        matches!(self, Self::Thinking | Self::Executing | Self::Auditing)
    }
}

pub static WORKFLOW_IDLE_SLEEP_INHIBITOR: LazyLock<IdleSleepInhibitor> =
    LazyLock::new(|| IdleSleepInhibitor::new(false));

/// Serialized desired state for the shared native assertion.
///
/// `assertion_active` is updated in the same critical section as `sessions`; this prevents an
/// older workflow transition from applying a stale native state after a newer transition.
struct AssertionState {
    sessions: ActiveWorkflowSessions,
    assertion_active: bool,
}

impl AssertionState {
    fn new(enabled: bool) -> Self {
        Self {
            sessions: ActiveWorkflowSessions {
                enabled,
                session_ids: HashSet::new(),
            },
            assertion_active: false,
        }
    }

    fn reconcile(&mut self) -> Option<bool> {
        let should_inhibit = self.sessions.should_inhibit();
        (should_inhibit != self.assertion_active).then(|| {
            self.assertion_active = should_inhibit;
            should_inhibit
        })
    }

    fn set_enabled(&mut self, enabled: bool) -> Option<bool> {
        self.sessions.set_enabled(enabled);
        self.reconcile()
    }

    fn set_session_active(&mut self, session_id: &str, active: bool) -> Option<bool> {
        self.sessions.set_session_active(session_id, active);
        self.reconcile()
    }

    fn remove_session(&mut self, session_id: &str) -> Option<bool> {
        self.sessions.remove_session(session_id);
        self.reconcile()
    }
}

/// All native assertion changes are linearized under this mutex with their corresponding session
/// set mutations. Holding a single synchronization boundary is essential because workflows may
/// transition state concurrently on different runtime threads.
struct IdleSleepState {
    assertion: AssertionState,
    platform: PlatformIdleSleepAssertion,
}

/// A process-wide, reference-counted idle-sleep assertion for concurrently running workflows.
///
/// The assertion only prevents automatic idle sleep. It never requests display wakefulness and
/// does not override explicit user sleep, lid-close behavior, or administrator policy.
pub struct IdleSleepInhibitor {
    state: Mutex<IdleSleepState>,
}

impl IdleSleepInhibitor {
    pub fn new(enabled: bool) -> Self {
        Self {
            state: Mutex::new(IdleSleepState {
                assertion: AssertionState::new(enabled),
                platform: PlatformIdleSleepAssertion::new(),
            }),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut state = self.state.lock();
        if let Some(active) = state.assertion.set_enabled(enabled) {
            state.platform.set_active(active);
        }
    }

    /// Synchronizes one workflow's canonical state with the shared assertion.
    ///
    /// Calling this for every state transition keeps concurrent sessions independent: the native
    /// assertion is released only after the last active session leaves a progressing state.
    pub fn sync_workflow_state(&self, session_id: &str, state: &WorkflowState) {
        let mut shared_state = self.state.lock();
        if let Some(active) = shared_state
            .assertion
            .set_session_active(session_id, state.prevents_idle_sleep())
        {
            shared_state.platform.set_active(active);
        }
    }

    pub fn remove_workflow(&self, session_id: &str) {
        let mut state = self.state.lock();
        if let Some(active) = state.assertion.remove_session(session_id) {
            state.platform.set_active(active);
        }
    }
}

#[cfg(target_os = "macos")]
struct PlatformIdleSleepAssertion {
    process: Option<Child>,
}

#[cfg(target_os = "macos")]
impl PlatformIdleSleepAssertion {
    fn new() -> Self {
        Self { process: None }
    }

    fn set_active(&mut self, active: bool) {
        if active == self.process.is_some() {
            return;
        }

        if active {
            match Command::new("caffeinate")
                // `-i` blocks idle system sleep only; it deliberately does not use `-d`.
                .arg("-i")
                .arg("-w")
                .arg(std::process::id().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(process) => self.process = Some(process),
                Err(error) => log::warn!(
                    "[IdleSleepInhibitor][platform=macos][event=assertion_start_failed] {error}"
                ),
            }
        } else {
            self.stop_process();
        }
    }

    fn stop_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            if let Err(error) = process.kill() {
                log::debug!(
                    "[IdleSleepInhibitor][platform=macos][event=assertion_stop_failed] {error}"
                );
            }
            let _ = process.wait();
        }
    }
}

#[cfg(target_os = "linux")]
struct PlatformIdleSleepAssertion {
    process: Option<Child>,
}

#[cfg(target_os = "linux")]
impl PlatformIdleSleepAssertion {
    fn new() -> Self {
        Self { process: None }
    }

    fn set_active(&mut self, active: bool) {
        if active == self.process.is_some() {
            return;
        }

        if active {
            match Command::new("systemd-inhibit")
                .args([
                    // `idle` only inhibits automatic idle handling; it does not inhibit the
                    // separate `sleep` operation requested explicitly by the user.
                    "--what=idle",
                    "--mode=block",
                    "--why=ChatSpeed is executing a workflow",
                    "sleep",
                    "infinity",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(process) => self.process = Some(process),
                Err(error) => log::warn!(
                    "[IdleSleepInhibitor][platform=linux][event=assertion_start_failed] {error}"
                ),
            }
        } else {
            self.stop_process();
        }
    }

    fn stop_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            if let Err(error) = process.kill() {
                log::debug!(
                    "[IdleSleepInhibitor][platform=linux][event=assertion_stop_failed] {error}"
                );
            }
            let _ = process.wait();
        }
    }
}

#[cfg(target_os = "windows")]
struct PlatformIdleSleepAssertion {
    sender: std::sync::mpsc::Sender<bool>,
}

#[cfg(target_os = "windows")]
impl PlatformIdleSleepAssertion {
    fn new() -> Self {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };

        let (sender, receiver) = std::sync::mpsc::channel::<bool>();
        std::thread::spawn(move || {
            let mut active = false;
            while let Ok(next_active) = receiver.recv() {
                if next_active == active {
                    continue;
                }
                let flags = if next_active {
                    ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                } else {
                    ES_CONTINUOUS
                };
                // Keep all calls on this dedicated thread because the Windows execution state is
                // thread-associated. ES_DISPLAY_REQUIRED is intentionally never requested.
                if unsafe { SetThreadExecutionState(flags) } == 0 {
                    log::warn!(
                        "[IdleSleepInhibitor][platform=windows][event=assertion_update_failed] active={next_active}"
                    );
                }
                active = next_active;
            }
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        });
        Self { sender }
    }

    fn set_active(&mut self, active: bool) {
        if let Err(error) = self.sender.send(active) {
            log::warn!(
                "[IdleSleepInhibitor][platform=windows][event=assertion_channel_closed] {error}"
            );
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
struct PlatformIdleSleepAssertion;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl PlatformIdleSleepAssertion {
    fn new() -> Self {
        Self
    }

    fn set_active(&mut self, _active: bool) {}
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for PlatformIdleSleepAssertion {
    fn drop(&mut self) {
        self.stop_process();
    }
}

#[cfg(test)]
mod tests {
    use super::{AssertionState, WorkflowState};

    #[test]
    fn only_autonomously_progressing_states_prevent_idle_sleep() {
        for state in [
            WorkflowState::Thinking,
            WorkflowState::Executing,
            WorkflowState::Auditing,
        ] {
            assert!(state.prevents_idle_sleep());
        }

        for state in [
            WorkflowState::Pending,
            WorkflowState::Stopping,
            WorkflowState::Paused,
            WorkflowState::AwaitingUser,
            WorkflowState::AwaitingApproval,
            WorkflowState::AwaitingAutoApproval,
            WorkflowState::AwaitingSubAgent,
            WorkflowState::Completed,
            WorkflowState::Error,
            WorkflowState::Cancelled,
        ] {
            assert!(!state.prevents_idle_sleep());
        }
    }

    #[test]
    fn concurrent_sessions_keep_the_assertion_until_the_last_one_stops() {
        let mut state = AssertionState::new(true);

        assert_eq!(state.set_session_active("first", true), Some(true));
        assert_eq!(state.set_session_active("second", true), None);
        assert_eq!(state.set_session_active("first", false), None);
        assert_eq!(state.set_session_active("second", false), Some(false));
    }

    #[test]
    fn disabling_the_setting_releases_without_losing_active_sessions() {
        let mut state = AssertionState::new(true);
        assert_eq!(state.set_session_active("workflow", true), Some(true));

        assert_eq!(state.set_enabled(false), Some(false));
        assert_eq!(state.set_enabled(true), Some(true));
    }

    #[test]
    fn serialized_last_session_transition_cannot_apply_a_stale_assertion() {
        let mut state = AssertionState::new(true);

        // This is the interleaving that was unsafe with separate session/platform mutexes:
        // a first transition requested `true`, then the last session stopped before the native
        // assertion update happened. AssertionState serializes both transitions, so only the
        // current desired state can be emitted by each completed transition.
        assert_eq!(state.set_session_active("workflow", true), Some(true));
        assert_eq!(state.set_session_active("workflow", false), Some(false));
        assert_eq!(state.reconcile(), None);
        assert!(!state.sessions.should_inhibit());
        assert!(!state.assertion_active);
    }
}
