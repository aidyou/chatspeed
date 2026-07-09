use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::workflow::react::engine::ReActExecutor;
use crate::workflow::react::types::WorkflowState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionStatus {
    Active,
    Waiting,
    Stopping,
    Completed,
    Failed,
    Cancelled,
}

pub struct ManagedSession {
    pub executor: Arc<Mutex<dyn ReActExecutor>>,
    pub status: ManagedSessionStatus,
    #[cfg(test)]
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ManagedSession {
    pub fn new(executor: Arc<Mutex<dyn ReActExecutor>>, status: ManagedSessionStatus) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            executor,
            status,
            #[cfg(test)]
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = chrono::Utc::now().timestamp_millis();
    }
}

#[derive(Debug, Error)]
pub enum WorkflowManagerError {
    #[error("Session already exists: {session_id}")]
    SessionAlreadyExists { session_id: String },

    #[error("Session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("Signal rejected for session {session_id}: {reason}")]
    SignalRejected { session_id: String, reason: String },
}

pub struct WorkflowManager {
    sessions: DashMap<String, ManagedSession>,
}

impl WorkflowManager {
    pub fn new() -> Self {
        log::info!("[WorkflowManager] Initialized");
        Self {
            sessions: DashMap::new(),
        }
    }

    pub fn register_session(
        &self,
        session_id: String,
        executor: Arc<Mutex<dyn ReActExecutor>>,
        status: ManagedSessionStatus,
    ) -> Result<(), WorkflowManagerError> {
        if self.sessions.contains_key(&session_id) {
            log::info!(
                "[WorkflowManager][session={}][event=session_registered] Session already exists",
                session_id
            );
            return Err(WorkflowManagerError::SessionAlreadyExists { session_id });
        }

        let session = ManagedSession::new(executor, status.clone());
        self.sessions.insert(session_id.clone(), session);

        log::info!(
            "[WorkflowManager][session={}][event=session_registered] Session registered with status {:?}",
            session_id,
            status
        );

        Ok(())
    }

    pub fn remove_session(&self, session_id: &str) -> Option<ManagedSession> {
        let removed = self.sessions.remove(session_id);

        if let Some((_, session)) = removed {
            log::info!(
                "[WorkflowManager][session={}][event=session_removed] Session removed, was in status {:?}",
                session_id,
                session.status
            );
            Some(session)
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_miss] Attempted to remove non-existent session",
                session_id
            );
            None
        }
    }

    #[cfg(test)]
    pub fn remove_session_if_matches(
        &self,
        session_id: &str,
        created_at_ms: i64,
    ) -> Option<ManagedSession> {
        if let Some(session) = self.sessions.get(session_id) {
            if session.created_at_ms != created_at_ms {
                return None;
            }
        } else {
            return None;
        }

        self.remove_session(session_id)
    }

    pub fn remove_session_if_updated_matches(
        &self,
        session_id: &str,
        updated_at_ms: i64,
    ) -> Option<ManagedSession> {
        if let Some(session) = self.sessions.get(session_id) {
            if session.updated_at_ms != updated_at_ms {
                return None;
            }
        } else {
            return None;
        }

        self.remove_session(session_id)
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        let exists = self.sessions.contains_key(session_id);

        if exists {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_hit] Session exists",
                session_id
            );
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_miss] Session not found",
                session_id
            );
        }

        exists
    }

    pub fn get_session_status(&self, session_id: &str) -> Option<ManagedSessionStatus> {
        self.sessions.get(session_id).map(|s| s.status.clone())
    }

    pub fn get_executor(&self, session_id: &str) -> Option<Arc<Mutex<dyn ReActExecutor>>> {
        self.sessions.get(session_id).map(|s| s.executor.clone())
    }

    #[cfg(test)]
    pub fn get_session_created_at_ms(&self, session_id: &str) -> Option<i64> {
        self.sessions.get(session_id).map(|s| s.created_at_ms)
    }

    pub fn get_session_updated_at_ms(&self, session_id: &str) -> Option<i64> {
        self.sessions.get(session_id).map(|s| s.updated_at_ms)
    }

    pub fn update_session_status(&self, session_id: &str, status: ManagedSessionStatus) -> bool {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.status = status.clone();
            session.touch();

            log::info!(
                "[WorkflowManager][session={}][event=status_updated] Status updated to {:?}",
                session_id,
                status
            );

            true
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_miss] Attempted to update non-existent session",
                session_id
            );
            false
        }
    }

    pub fn transition_session_status_if_current(
        &self,
        session_id: &str,
        expected: ManagedSessionStatus,
        next: ManagedSessionStatus,
    ) -> bool {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            if session.status != expected {
                log::debug!(
                    "[WorkflowManager][session={}][event=status_transition_skipped] Expected {:?} but found {:?}, next={:?}",
                    session_id,
                    expected,
                    session.status,
                    next
                );
                return false;
            }

            session.status = next.clone();
            session.touch();
            log::info!(
                "[WorkflowManager][session={}][event=status_updated] Status transitioned from {:?} to {:?}",
                session_id,
                expected,
                next
            );
            true
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_miss] Attempted conditional status transition on non-existent session",
                session_id
            );
            false
        }
    }

    pub fn managed_status_for_workflow_state(state: &WorkflowState) -> ManagedSessionStatus {
        match state {
            WorkflowState::Pending
            | WorkflowState::Thinking
            | WorkflowState::Executing
            | WorkflowState::Auditing => ManagedSessionStatus::Active,
            WorkflowState::Stopping => ManagedSessionStatus::Stopping,
            WorkflowState::Paused
            | WorkflowState::AwaitingUser
            | WorkflowState::AwaitingApproval
            | WorkflowState::AwaitingAutoApproval
            | WorkflowState::AwaitingSubAgent => ManagedSessionStatus::Waiting,
            WorkflowState::Completed => ManagedSessionStatus::Completed,
            WorkflowState::Error => ManagedSessionStatus::Failed,
            WorkflowState::Cancelled => ManagedSessionStatus::Cancelled,
        }
    }

    pub fn reconcile_session_status_from_workflow_state(
        &self,
        session_id: &str,
        state: &WorkflowState,
    ) -> Option<ManagedSessionStatus> {
        let status = Self::managed_status_for_workflow_state(state);
        self.update_session_status(session_id, status.clone())
            .then_some(status)
    }

    /// Validates whether a signal may be routed to a live session.
    /// This only checks session liveness/status; actual injection happens through the gateway.
    pub fn validate_signal_routing(
        &self,
        session_id: &str,
        signal_type: &str,
    ) -> Result<(), WorkflowManagerError> {
        let session = self.sessions.get(session_id);

        if let Some(session) = session {
            match session.status {
                ManagedSessionStatus::Active | ManagedSessionStatus::Waiting => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_routing_validated] Signal '{}' accepted for gateway injection",
                        session_id,
                        signal_type
                    );
                    Ok(())
                }
                ManagedSessionStatus::Stopping => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: session stopping",
                        session_id,
                        signal_type
                    );
                    Err(WorkflowManagerError::SignalRejected {
                        session_id: session_id.to_string(),
                        reason: "Session is stopping".to_string(),
                    })
                }
                ManagedSessionStatus::Completed => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: session completed",
                        session_id,
                        signal_type
                    );
                    Err(WorkflowManagerError::SignalRejected {
                        session_id: session_id.to_string(),
                        reason: "Session is completed".to_string(),
                    })
                }
                ManagedSessionStatus::Failed => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: session failed",
                        session_id,
                        signal_type
                    );
                    Err(WorkflowManagerError::SignalRejected {
                        session_id: session_id.to_string(),
                        reason: "Session has failed".to_string(),
                    })
                }
                ManagedSessionStatus::Cancelled => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: session cancelled",
                        session_id,
                        signal_type
                    );
                    Err(WorkflowManagerError::SignalRejected {
                        session_id: session_id.to_string(),
                        reason: "Session is cancelled".to_string(),
                    })
                }
            }
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=session_lookup_miss] Signal '{}' cannot be routed: session not found",
                session_id,
                signal_type
            );
            Err(WorkflowManagerError::SessionNotFound {
                session_id: session_id.to_string(),
            })
        }
    }

    #[cfg(test)]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for WorkflowManager {
    fn default() -> Self {
        Self::new()
    }
}

lazy_static::lazy_static! {
    static ref GLOBAL_SIGNAL_TX: std::sync::Mutex<std::collections::HashMap<String, tokio::sync::mpsc::Sender<String>>> =
        std::sync::Mutex::new(std::collections::HashMap::new());
}

impl WorkflowManager {
    fn lock_global_signal_tx() -> Option<
        std::sync::MutexGuard<
            'static,
            std::collections::HashMap<String, tokio::sync::mpsc::Sender<String>>,
        >,
    > {
        match GLOBAL_SIGNAL_TX.lock() {
            Ok(map) => Some(map),
            Err(error) => {
                log::error!(
                    "[WorkflowManager][event=global_signal_tx_poisoned] Failed to acquire GLOBAL_SIGNAL_TX: {}",
                    error
                );
                None
            }
        }
    }

    pub fn register_session_signal_tx(session_id: String, tx: tokio::sync::mpsc::Sender<String>) {
        Self::register_session_signal_tx_with_source(session_id, tx, "unspecified");
    }

    pub fn register_session_signal_tx_with_source(
        session_id: String,
        tx: tokio::sync::mpsc::Sender<String>,
        source: &str,
    ) {
        let Some(mut map) = Self::lock_global_signal_tx() else {
            return;
        };
        let replaced = map.insert(session_id.clone(), tx).is_some();
        if replaced {
            log::info!(
                "[WorkflowManager][session={}][event=signal_channel_replaced] Replaced existing GLOBAL_SIGNAL_TX entry, source={}",
                session_id,
                source
            );
        } else {
            log::info!(
                "[WorkflowManager][session={}][event=signal_channel_registered] Registered GLOBAL_SIGNAL_TX entry, source={}",
                session_id,
                source
            );
        }
    }

    pub fn unregister_session_signal_tx(session_id: &str) {
        Self::unregister_session_signal_tx_with_source(session_id, "unspecified");
    }

    pub fn unregister_session_signal_tx_with_source(session_id: &str, source: &str) {
        let Some(mut map) = Self::lock_global_signal_tx() else {
            return;
        };
        let existed = map.remove(session_id).is_some();
        if existed {
            log::info!(
                "[WorkflowManager][session={}][event=signal_channel_unregistered] Removed GLOBAL_SIGNAL_TX entry, source={}",
                session_id,
                source
            );
        } else {
            log::debug!(
                "[WorkflowManager][session={}][event=signal_channel_unregister_miss] GLOBAL_SIGNAL_TX entry already missing, source={}",
                session_id,
                source
            );
        }
    }

    pub fn send_signal_to_session(session_id: &str, signal: String) -> Result<(), String> {
        let Some(map) = Self::lock_global_signal_tx() else {
            return Err("GLOBAL_SIGNAL_TX poisoned".to_string());
        };
        if let Some(tx) = map.get(session_id) {
            tx.try_send(signal).map_err(|e| e.to_string())
        } else {
            Err(format!("Session {} signal channel not found", session_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::error::WorkflowEngineError;
    use crate::workflow::react::types::StepType;
    use crate::workflow::react::types::WorkflowState;

    struct MockExecutor {
        session_id: String,
        state: WorkflowState,
        messages: Vec<crate::db::WorkflowMessage>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                session_id: "mock".to_string(),
                state: WorkflowState::Pending,
                messages: vec![],
            }
        }
    }

    #[async_trait::async_trait]
    impl ReActExecutor for MockExecutor {
        async fn init(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn begin_new_context_segment(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn begin_manual_clear_context_segment(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn prepare_completed_resume(&mut self) -> Result<(), WorkflowEngineError> {
            self.state = WorkflowState::Thinking;
            Ok(())
        }

        async fn add_message_and_notify(
            &mut self,
            _role: String,
            _content: String,
            _attached_context: Option<String>,
            _reasoning: Option<String>,
            _step_type: Option<StepType>,
            _is_error: bool,
            _error_type: Option<String>,
            _metadata: Option<serde_json::Value>,
        ) -> Result<bool, WorkflowEngineError> {
            Ok(true)
        }

        fn session_id(&self) -> String {
            self.session_id.clone()
        }

        fn state(&self) -> WorkflowState {
            self.state.clone()
        }

        fn set_state(&mut self, state: WorkflowState) {
            self.state = state;
        }

        fn attach_signal_rx(&mut self, _signal_rx: tokio::sync::mpsc::Receiver<String>) {}

        fn messages(&self) -> Vec<crate::db::WorkflowMessage> {
            self.messages.clone()
        }
    }

    #[test]
    fn test_register_session() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        let result = manager.register_session(
            "test-session-1".to_string(),
            executor.clone(),
            ManagedSessionStatus::Active,
        );
        assert!(result.is_ok());

        let result = manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_session() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Active,
            )
            .unwrap();

        let removed = manager.remove_session("test-session-1");
        assert!(removed.is_some());

        assert!(!manager.has_session("test-session-1"));
    }

    #[test]
    fn test_remove_session_if_matches_created_at() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Completed,
            )
            .unwrap();

        let created_at = manager
            .get_session_created_at_ms("test-session-1")
            .expect("created_at should exist");

        assert!(manager
            .remove_session_if_matches("test-session-1", created_at + 1)
            .is_none());
        assert!(manager.has_session("test-session-1"));

        assert!(manager
            .remove_session_if_matches("test-session-1", created_at)
            .is_some());
        assert!(!manager.has_session("test-session-1"));
    }

    #[test]
    fn test_remove_session_if_updated_matches() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Completed,
            )
            .unwrap();

        let original_updated_at = manager
            .get_session_updated_at_ms("test-session-1")
            .expect("updated_at should exist");
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(manager.update_session_status("test-session-1", ManagedSessionStatus::Active));
        let refreshed_updated_at = manager
            .get_session_updated_at_ms("test-session-1")
            .expect("updated_at should refresh");

        assert!(refreshed_updated_at >= original_updated_at);
        assert!(manager
            .remove_session_if_updated_matches("test-session-1", original_updated_at)
            .is_none());
        assert!(manager.has_session("test-session-1"));

        assert!(manager
            .remove_session_if_updated_matches("test-session-1", refreshed_updated_at)
            .is_some());
        assert!(!manager.has_session("test-session-1"));
    }

    #[test]
    fn test_has_session() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        assert!(!manager.has_session("non-existent"));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Active,
            )
            .unwrap();

        assert!(manager.has_session("test-session-1"));
    }

    #[test]
    fn test_get_session_status() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        assert!(manager.get_session_status("non-existent").is_none());

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Active,
            )
            .unwrap();

        assert_eq!(
            manager.get_session_status("test-session-1"),
            Some(ManagedSessionStatus::Active)
        );
    }

    #[test]
    fn test_transition_session_status_if_current_only_allows_one_completed_resume_claim() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Completed,
            )
            .unwrap();

        assert!(manager.transition_session_status_if_current(
            "test-session-1",
            ManagedSessionStatus::Completed,
            ManagedSessionStatus::Active,
        ));
        assert!(!manager.transition_session_status_if_current(
            "test-session-1",
            ManagedSessionStatus::Completed,
            ManagedSessionStatus::Active,
        ));
        assert_eq!(
            manager.get_session_status("test-session-1"),
            Some(ManagedSessionStatus::Active)
        );
    }

    #[test]
    fn test_update_session_status() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Active,
            )
            .unwrap();

        let updated =
            manager.update_session_status("test-session-1", ManagedSessionStatus::Waiting);
        assert!(updated);

        assert_eq!(
            manager.get_session_status("test-session-1"),
            Some(ManagedSessionStatus::Waiting)
        );
    }

    #[test]
    fn test_validate_signal_routing() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));

        let result = manager.validate_signal_routing("non-existent", "user_input");
        assert!(result.is_err());

        manager
            .register_session(
                "test-session-1".to_string(),
                executor,
                ManagedSessionStatus::Active,
            )
            .unwrap();

        let result = manager.validate_signal_routing("test-session-1", "user_input");
        assert!(result.is_ok());

        manager.update_session_status("test-session-1", ManagedSessionStatus::Completed);

        let result = manager.validate_signal_routing("test-session-1", "user_input");
        assert!(result.is_err());
    }

    #[test]
    fn test_managed_status_for_workflow_state() {
        assert_eq!(
            WorkflowManager::managed_status_for_workflow_state(&WorkflowState::Thinking),
            ManagedSessionStatus::Active
        );
        assert_eq!(
            WorkflowManager::managed_status_for_workflow_state(&WorkflowState::AwaitingApproval),
            ManagedSessionStatus::Waiting
        );
        assert_eq!(
            WorkflowManager::managed_status_for_workflow_state(&WorkflowState::Completed),
            ManagedSessionStatus::Completed
        );
        assert_eq!(
            WorkflowManager::managed_status_for_workflow_state(&WorkflowState::Error),
            ManagedSessionStatus::Failed
        );
        assert_eq!(
            WorkflowManager::managed_status_for_workflow_state(&WorkflowState::Cancelled),
            ManagedSessionStatus::Cancelled
        );
    }
}
