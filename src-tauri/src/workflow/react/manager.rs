use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::workflow::react::engine::ReActExecutor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionStatus {
    Active,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

pub struct ManagedSession {
    pub session_id: String,
    pub executor: Arc<Mutex<dyn ReActExecutor>>,
    pub status: ManagedSessionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ManagedSession {
    pub fn new(
        session_id: String,
        executor: Arc<Mutex<dyn ReActExecutor>>,
        status: ManagedSessionStatus,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            session_id,
            executor,
            status,
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
    
    #[error("Invalid session state for {session_id}: expected {expected}, got {actual}")]
    InvalidState { session_id: String, expected: String, actual: String },
    
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
            log::warn!(
                "[WorkflowManager][session={}][event=session_registered] Session already exists",
                session_id
            );
            return Err(WorkflowManagerError::SessionAlreadyExists { session_id });
        }

        let session = ManagedSession::new(session_id.clone(), executor, status.clone());
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

    pub fn get_executor(
        &self,
        session_id: &str,
    ) -> Option<Arc<Mutex<dyn ReActExecutor>>> {
        self.sessions.get(session_id).map(|s| s.executor.clone())
    }

    pub fn update_session_status(
        &self,
        session_id: &str,
        status: ManagedSessionStatus,
    ) -> bool {
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

    pub fn route_signal(
        &self,
        session_id: &str,
        signal_type: &str,
    ) -> Result<(), WorkflowManagerError> {
        let session = self.sessions.get(session_id);
        
        if let Some(session) = session {
            match session.status {
                ManagedSessionStatus::Active | ManagedSessionStatus::Waiting => {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_routed] Signal '{}' routed successfully",
                        session_id,
                        signal_type
                    );
                    Ok(())
                }
                ManagedSessionStatus::Completed => {
                    log::warn!(
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
                    log::warn!(
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
                    log::warn!(
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

    pub fn get_all_session_ids(&self) -> Vec<String> {
        self.sessions.iter().map(|entry| entry.key().clone()).collect()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for WorkflowManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::types::WorkflowState;
    use crate::workflow::react::error::WorkflowEngineError;
    use crate::workflow::react::types::StepType;

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
        
        manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        ).unwrap();
        
        let removed = manager.remove_session("test-session-1");
        assert!(removed.is_some());
        
        assert!(!manager.has_session("test-session-1"));
    }

    #[test]
    fn test_has_session() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));
        
        assert!(!manager.has_session("non-existent"));
        
        manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        ).unwrap();
        
        assert!(manager.has_session("test-session-1"));
    }

    #[test]
    fn test_get_session_status() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));
        
        assert!(manager.get_session_status("non-existent").is_none());
        
        manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        ).unwrap();
        
        assert_eq!(
            manager.get_session_status("test-session-1"),
            Some(ManagedSessionStatus::Active)
        );
    }

    #[test]
    fn test_update_session_status() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));
        
        manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        ).unwrap();
        
        let updated = manager.update_session_status(
            "test-session-1",
            ManagedSessionStatus::Waiting,
        );
        assert!(updated);
        
        assert_eq!(
            manager.get_session_status("test-session-1"),
            Some(ManagedSessionStatus::Waiting)
        );
    }

    #[test]
    fn test_route_signal() {
        let manager = WorkflowManager::new();
        let executor = Arc::new(Mutex::new(MockExecutor::new()));
        
        let result = manager.route_signal("non-existent", "user_input");
        assert!(result.is_err());
        
        manager.register_session(
            "test-session-1".to_string(),
            executor,
            ManagedSessionStatus::Active,
        ).unwrap();
        
        let result = manager.route_signal("test-session-1", "user_input");
        assert!(result.is_ok());
        
        manager.update_session_status("test-session-1", ManagedSessionStatus::Completed);
        
        let result = manager.route_signal("test-session-1", "user_input");
        assert!(result.is_err());
    }
}
