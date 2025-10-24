import { invoke } from '@tauri-apps/api/core'

/**
 * Custom error class for frontend to handle structured backend errors.
 */
export class FrontendAppError extends Error {
  constructor(module, kind, message, originalError) {
    super(message)
    this.name = 'FrontendAppError'
    this.module = module
    this.kind = kind
    this.originalError = originalError
  }

  toFormattedString() {
    return `[${this.module}:${this.kind}] ${this.message}`
  }
}

/**
 * A wrapper around Tauri's `invoke` function to provide standardized error handling.
 * It parses structured errors from the Rust backend and re-throws them as `FrontendAppError`.
 *
 * @param {string} command - The name of the Tauri command to invoke.
 * @param {object} [payload={}] - The payload to send with the command.
 * @returns {Promise<any>} A promise that resolves with the command result or rejects with a `FrontendAppError`.
 */
export async function invokeWrapper(command, payload = {}) {
  try {
    return await invoke(command, payload)
  } catch (error) {
    const parsedError = {
      module: 'Unknown',
      kind: 'Unknown',
      message: String(error),
      originalError: error
    }

    // Handle both string and object format errors from Rust backend
    let rustError = null

    if (typeof error === 'string') {
      try {
        rustError = JSON.parse(error)
      } catch {
        // If JSON parsing fails, use the original error string as message
        parsedError.message = String(error)
      }
    } else if (typeof error === 'object' && error !== null) {
      // Error is already an object
      rustError = error
    }

    if (rustError?.module) {
      parsedError.module = rustError.module
      parsedError.message = rustError.message || String(error) // Prioritize top-level message
      if (rustError.details) {
        if (typeof rustError.details === 'object' && rustError.details !== null) {
          // This is a structured error, e.g., { kind: '...', message: '...' }
          parsedError.kind = rustError.details.kind || 'Unknown'
          // The message from the inner error is often more specific
          parsedError.message = rustError.details.message || parsedError.message
        } else if (typeof rustError.details === 'string') {
          // This is a simple string error, e.g., from General(String)
          // The details string is the error message. The top-level message should be the same.
          // We don't have a 'kind' here. We can use the module name as a fallback.
          parsedError.kind = rustError.module
        }
      }
    }

    throw new FrontendAppError(
      parsedError.module,
      parsedError.kind,
      parsedError.message,
      parsedError.originalError
    )
  }
}
