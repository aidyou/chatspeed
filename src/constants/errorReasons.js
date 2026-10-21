export const USER_ERROR_REASONS = Object.freeze({
  UNKNOWN: 'unknown',
  SESSION_ALREADY_EXISTS: 'session_already_exists',
  SESSION_STOPPING: 'session_stopping',
  NO_ACTIVE_SESSION: 'no_active_session',
  FILE_TOO_LARGE: 'file_too_large',
  FILE_CHANGED_ON_DISK: 'file_changed_on_disk',
  UNSUPPORTED_LIST_MODELS: 'unsupported_list_models',
  RUNTIME_STOPPING: 'runtime_stopping'
})

const reasons = new Set(Object.values(USER_ERROR_REASONS))

export function normalizeErrorReason(reason) {
  return reasons.has(reason) ? reason : USER_ERROR_REASONS.UNKNOWN
}
