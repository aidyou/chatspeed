/**
 * Canonical workflow protocol constants shared by frontend modules.
 * Values must match backend serde tags exactly.
 */
export const SIGNAL_TYPES = {
  USER_MESSAGE: 'user_message',
  APPROVAL: 'approval',
  CONTINUE: 'continue',
  STOP: 'stop',
  REBROADCAST_PENDING: 'rebroadcast_pending',
  UPDATE_FINAL_AUDIT: 'update_final_audit',
  UPDATE_AUTO_COMPRESS: 'update_auto_compress',
  UPDATE_APPROVAL_LEVEL: 'update_approval_level',
  UPDATE_PHASE: 'update_phase',
  UPDATE_ALLOWED_PATHS: 'update_allowed_paths',
  UPDATE_MODEL_CONFIG: 'update_model_config',
  UPDATE_SKILLS_CONFIG: 'update_skills_config',
  REMOVE_SHELL_POLICY_ITEM: 'remove_shell_policy_item',
  REMOVE_AUTO_APPROVED_TOOL: 'remove_auto_approved_tool',
  REMOVE_QUEUED_USER_MESSAGE: 'remove_queued_user_message',
  SUB_AGENT_COMPLETE: 'sub_agent_complete',
} as const

export const WORKFLOW_WAIT_REASONS = {
  USER_INPUT: 'user_input',
  APPROVAL: 'approval',
  CONFIRMATION: 'confirmation',
  SUB_AGENT: 'sub_agent',
} as const

export const WORKFLOW_STATUSES = {
  PENDING: 'pending',
  THINKING: 'thinking',
  EXECUTING: 'executing',
  AUDITING: 'auditing',
  RUNNING: 'running',
  STOPPING: 'stopping',
  PAUSED: 'paused',
  AWAITING_USER: 'awaiting_user',
  AWAITING_APPROVAL: 'awaiting_approval',
  AWAITING_AUTO_APPROVAL: 'awaiting_auto_approval',
  AWAITING_SUB_AGENT: 'awaiting_sub_agent',
  COMPLETED: 'completed',
  ERROR: 'error',
  FAILED: 'failed',
  CANCELLED: 'cancelled',
} as const

export const RUNNING_STATUSES = [
  WORKFLOW_STATUSES.THINKING,
  WORKFLOW_STATUSES.EXECUTING,
  WORKFLOW_STATUSES.AUDITING,
  WORKFLOW_STATUSES.RUNNING,
] as const

export const WAITING_STATUSES = [
  WORKFLOW_STATUSES.PAUSED,
  WORKFLOW_STATUSES.AWAITING_USER,
  WORKFLOW_STATUSES.AWAITING_APPROVAL,
  WORKFLOW_STATUSES.AWAITING_AUTO_APPROVAL,
  WORKFLOW_STATUSES.AWAITING_SUB_AGENT,
] as const

export const APPROVAL_WAITING_STATUSES = [
  WORKFLOW_STATUSES.AWAITING_APPROVAL,
  WORKFLOW_STATUSES.AWAITING_AUTO_APPROVAL,
] as const

export const TERMINAL_STATUSES = [
  WORKFLOW_STATUSES.COMPLETED,
  WORKFLOW_STATUSES.CANCELLED,
  WORKFLOW_STATUSES.ERROR,
  WORKFLOW_STATUSES.FAILED,
] as const

export const RESUMABLE_STATUSES = [
  WORKFLOW_STATUSES.PAUSED,
  WORKFLOW_STATUSES.ERROR,
  WORKFLOW_STATUSES.FAILED,
  WORKFLOW_STATUSES.CANCELLED,
] as const

export const BLOCKING_WAIT_REASONS = [
  WORKFLOW_WAIT_REASONS.USER_INPUT,
  WORKFLOW_WAIT_REASONS.APPROVAL,
  WORKFLOW_WAIT_REASONS.CONFIRMATION,
] as const

export const FIELD_NAMES = {
  finalAudit: 'finalAudit',
  approvalLevel: 'approvalLevel',
} as const
