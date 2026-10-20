import { computed, onScopeDispose, ref } from 'vue'

export const isRuntimeReady = status =>
  ['attached', 'spawned'].includes(status?.state) && status.heartbeat_ok !== false

// Polling and hydration have separate single flights so a slow load cannot hide a restart.
export function useRuntimeStatus(invoke, refresh, {
  interval = 1000, retryDelay = 2000, maxRetryDelay = 30000
} = {}) {
  const status = ref(null)
  const loading = ref(true)
  const error = ref(null)
  const statusError = ref(null)
  const ready = computed(() => isRuntimeReady(status.value))
  const usable = computed(() => ready.value && !loading.value && !error.value)
  let stopped = false
  let timer
  let started = false
  let revision = 0
  let hydratedRevision = -1
  let refreshing = false
  let failures = 0
  let retryAt = 0

  const hydrate = async () => {
    if (stopped || refreshing || !ready.value || hydratedRevision === revision || Date.now() < retryAt) return
    refreshing = true
    loading.value = true
    const currentRevision = revision
    const isCurrent = () => !stopped && ready.value && currentRevision === revision
    try {
      await refresh(isCurrent)
      if (isCurrent()) {
        hydratedRevision = currentRevision
        failures = 0
        retryAt = 0
        error.value = null
      }
    } catch (cause) {
      if (isCurrent()) {
        error.value = cause
        retryAt = Date.now() + Math.min(maxRetryDelay, retryDelay * 2 ** Math.min(failures, 30))
        failures += 1
      }
    } finally {
      refreshing = false
      if (isCurrent()) loading.value = false
    }
  }

  const poll = async () => {
    try {
      const next = await invoke('runtime_status')
      if (stopped) return
      if (isRuntimeReady(next) && (!ready.value || next.generation !== status.value?.generation)) {
        revision += 1
        failures = 0
        retryAt = 0
        error.value = null
        loading.value = true
      }
      status.value = next
      statusError.value = null
    } catch (cause) {
      if (stopped) return
      status.value = null
      statusError.value = cause
    }
    if (stopped) return
    void hydrate()
    timer = setTimeout(poll, interval)
  }

  const start = () => {
    if (started || stopped) return
    started = true
    void poll()
  }
  const stop = () => {
    stopped = true
    clearTimeout(timer)
  }
  onScopeDispose(stop)
  return { status, ready, loading, error, statusError, usable, start, stop }
}
