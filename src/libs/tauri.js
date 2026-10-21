import { invoke } from '@tauri-apps/api/core'
import i18n from '../i18n'
import { normalizeUserError, setUserErrorFallback } from './error.js'

setUserErrorFallback(() => i18n.global.t('common.errorFallback'))

/**
 * Custom error class for frontend to handle structured backend errors.
 */
export class FrontendAppError extends Error {
  constructor(module, kind, message, originalError) {
    const normalized = normalizeUserError(originalError === undefined ? message : originalError)
    super(normalizeUserError(message).message)
    this.name = 'FrontendAppError'
    this.module = module
    this.kind = kind
    this.originalError = originalError
    this.code = normalized.code
    this.reason = normalized.reason
    this.upstream_status = normalized.upstream_status
  }

  toFormattedString() {
    const kindStr = this.kind ? `:${this.kind}` : ''
    return `[${this.module}${kindStr}] ${this.message}`
  }
}

/**
 * A wrapper around Tauri's `invoke` function to provide standardized error handling.
 * It parses structured errors from the Rust backend and re-throws them as `FrontendAppError`.
 *
 * @param {string} command - The name of the Tauri command to invoke.
 * @param {object} [payload={}] - The payload to send with the command.
 * @returns {Promise<unknown>} A promise that resolves with the command result or rejects with a `FrontendAppError`.
 */
export async function invokeWrapper(command, payload = {}) {
  try {
    return await invoke(command, payload)
  } catch (error) {
    const parsed = normalizeUserError(error)
    throw new FrontendAppError(parsed.module || 'Unknown', parsed.kind || 'Unknown', parsed.message, error)
  }
}
