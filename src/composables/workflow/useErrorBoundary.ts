import * as vue from 'vue'

export interface ErrorBoundaryOptions {
  silent?: boolean
  onError?: (error: Error, info: string) => void
  resetOnError?: boolean
}

export function useErrorBoundary(options: ErrorBoundaryOptions = {}) {
  const hasError = vue.ref(false)
  const error = vue.ref<Error | null>(null)
  const errorInfo = vue.ref('')

  const captureError = (err: Error, info: string) => {
    hasError.value = true
    error.value = err
    errorInfo.value = info

    // Log warning instead of error for graceful degradation
    console.warn('[ErrorBoundary] UI render error captured:', err.message, info)

    if (options.onError) {
      try {
        options.onError(err, info)
      } catch (handlerError) {
        console.error('[ErrorBoundary] Error handler failed:', handlerError)
      }
    }

    // Prevent error propagation to stop cascade failures
    return false
  }

  const reset = () => {
    hasError.value = false
    error.value = null
    errorInfo.value = ''
  }

  vue.onErrorCaptured((err, instance, info) => {
    if (err instanceof Error) {
      return captureError(err, info)
    }
    return false
  })

  return {
    hasError,
    error,
    errorInfo,
    captureError,
    reset
  }
}

export function safeExecute<T>(fn: () => T, fallback?: T, context?: string): T | undefined {
  try {
    return fn()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    console.warn(`[safeExecute] ${context || 'Execution'} failed:`, message)
    return fallback
  }
}

export async function safeExecuteAsync<T>(
  fn: () => Promise<T>,
  fallback?: T,
  context?: string
): Promise<T | undefined> {
  try {
    return await fn()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    console.warn(`[safeExecuteAsync] ${context || 'Execution'} failed:`, message)
    return fallback
  }
}

export function withFallback<T>(primary: () => T, fallback: () => T, context?: string): T {
  try {
    return primary()
  } catch {
    console.warn(
      `[withFallback] Primary render failed for ${context || 'component'}, using fallback`
    )
    try {
      return fallback()
    } catch (fallbackErr) {
      console.error(`[withFallback] Fallback also failed:`, fallbackErr)
      throw fallbackErr
    }
  }
}
