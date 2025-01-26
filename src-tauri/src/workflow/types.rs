use super::error::WorkflowError;
use std::fmt;

/// Workflow state enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowState {
    /// Initial state when workflow is first created
    Init,

    /// Ready to execute, all prerequisites are met
    Ready,

    /// Workflow is currently running
    Running,

    /// Workflow execution is temporarily paused
    /// Can be resumed later
    Paused,

    /// Workflow is manually cancelled by user or system
    /// Cannot be resumed
    Cancelled,

    /// Workflow is stopped (neutral state)
    /// Different from Cancelled (user action) or Completed (successful end)
    Stopped,

    /// Workflow completed successfully
    Completed,

    /// Content review failed
    /// Contains node ID that failed review and reason
    /// Can be restarted from the failed node
    ReviewFailed {
        /// ID of the node that failed review
        node_id: String,
        /// Reason for review failure
        reason: String,
        /// Original node output (for reference)
        original_output: Option<String>,
        /// Timestamp when the review failed
        timestamp: i64,
    },

    /// Workflow failed but can be retried
    /// Contains retry count and error message
    FailedWithRetry {
        /// Number of retry attempts made
        retry_count: u32,
        /// Maximum number of retries allowed
        max_retries: u32,
        /// Error message describing the failure
        error: String,
    },

    /// Workflow failed permanently
    /// Contains failure reason and timestamp
    Failed {
        /// Error message describing the failure
        error: String,
        /// Timestamp when the failure occurred (Unix timestamp in seconds)
        timestamp: i64,
    },

    /// Workflow encountered a recoverable error
    /// Different from Failed (permanent) or FailedWithRetry (automatic retry)
    Error(String),
}

impl WorkflowState {
    /// Check if the workflow is in a final state
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            WorkflowState::Completed | WorkflowState::Failed { .. } | WorkflowState::Cancelled
        )
    }

    /// Check if the workflow can be resumed
    pub fn can_resume(&self) -> bool {
        matches!(
            self,
            WorkflowState::Paused | WorkflowState::FailedWithRetry { .. } | WorkflowState::ReviewFailed { .. }
        )
    }

    /// Check if the workflow is in an error state
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            WorkflowState::Error(_)
                | WorkflowState::Failed { .. }
                | WorkflowState::FailedWithRetry { .. }
                | WorkflowState::ReviewFailed { .. }
        )
    }
}

impl fmt::Display for WorkflowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "init"),
            Self::Ready => write!(f, "ready"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Stopped => write!(f, "stopped"),
            Self::Completed => write!(f, "completed"),
            Self::ReviewFailed { node_id, reason, original_output, timestamp } => {
                write!(f, "review failed at {}: node {} (reason: {})", timestamp, node_id, reason)?;
                if let Some(output) = original_output {
                    write!(f, ", original output: {}", output)?;
                }
                Ok(())
            }
            Self::FailedWithRetry { retry_count, max_retries, error } => {
                write!(f, "failed_with_retry(attempt {}/{}): {}", retry_count, max_retries, error)
            }
            Self::Failed { error, timestamp } => {
                write!(f, "failed at {}: {}", timestamp, error)
            }
            Self::Error(error) => write!(f, "error: {}", error),
        }
    }
}

/// Type alias for workflow operation results
pub type WorkflowResult<T> = Result<T, WorkflowError>;
