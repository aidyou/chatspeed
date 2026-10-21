import { formatUserError } from './error.js'

// Resolve tool execution state from structured backend status only.
// Legacy history without a discriminator remains unknown; transcript wording is not authority.
export function resolveToolResultState(call) {
  if (!call || call.result === undefined || call.result === null) return 'calling'

  const explicit = call.is_error ?? call.isError ?? call.result_is_error ?? call.resultIsError
  if (typeof explicit === 'boolean') return explicit ? 'error' : 'success'

  const discriminator = call.status ?? call.execution_status ?? call.executionStatus
  if (typeof discriminator === 'string') {
    if (['error', 'failed', 'failure'].includes(discriminator.toLowerCase())) return 'error'
    if (['success', 'succeeded', 'completed', 'complete'].includes(discriminator.toLowerCase())) return 'success'
  }

  return 'unknown'
}

// Project only a failed tool's displayed copy; persisted/model results stay intact.
export function resolveToolResultDisplay(call) {
  if (resolveToolResultState(call) !== 'error') return call?.result
  return formatUserError({ presentation: call?.user_error, message: call?.result })
}

export default resolveToolResultState
