/**
 * 错误边界 Composable
 * 
 * 阶段9：UI 渲染异常必须可降级，不能阻断 workflow 执行与恢复
 */

import { ref, onErrorCaptured } from 'vue'

export interface ErrorBoundaryOptions {
  /** 是否静默处理错误 */
  silent?: boolean
  /** 自定义错误处理函数 */
  onError?: (error: Error, info: string) => void
  /** 是否重置状态 */
  resetOnError?: boolean
}

/**
 * 创建错误边界
 */
export function useErrorBoundary(options: ErrorBoundaryOptions = {}) {
  const { silent = false, onError, resetOnError = false } = options

  const hasError = ref(false)
  const error = ref<Error | null>(null)
  const errorInfo = ref('')

  const captureError = (err: Error, info: string) => {
    hasError.value = true
    error.value = err
    errorInfo.value = info

    // 记录警告日志而非错误（降级处理）
    console.warn('[ErrorBoundary] UI render error captured:', err.message, info)

    // 调用自定义错误处理
    if (onError) {
      try {
        onError(err, info)
      } catch (handlerError) {
        console.error('[ErrorBoundary] Error handler failed:', handlerError)
      }
    }

    // 阻止错误继续传播（阻断级联失败）
    return false
  }

  const reset = () => {
    hasError.value = false
    error.value = null
    errorInfo.value = ''
  }

  // Vue 错误捕获
  onErrorCaptured((err, instance, info) => {
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

/**
 * 安全的函数执行包装器
 */
export function safeExecute<T>(
  fn: () => T,
  fallback?: T,
  context?: string
): T | undefined {
  try {
    return fn()
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    console.warn(`[safeExecute] ${context || 'Execution'} failed:`, message)
    return fallback
  }
}

/**
 * 安全的异步函数执行包装器
 */
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

/**
 * 降级渲染包装器
 */
export function withFallback<T>(
  primary: () => T,
  fallback: () => T,
  context?: string
): T {
  try {
    return primary()
  } catch (err) {
    console.warn(`[withFallback] Primary render failed for ${context || 'component'}, using fallback`)
    try {
      return fallback()
    } catch (fallbackErr) {
      console.error(`[withFallback] Fallback also failed:`, fallbackErr)
      throw fallbackErr
    }
  }
}
