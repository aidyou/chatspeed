import { computed, onScopeDispose, ref } from 'vue'

export const isRuntimeReady = status =>
  ['attached', 'spawned'].includes(status?.state) && status.heartbeat_ok !== false

// Polling and hydration have separate single flights so a slow load cannot hide a restart.
export function useRuntimeStatus(invoke, refresh, { interval = 1000 } = {}) {
  const status = ref(null)
  const loading = ref(true)
  const error = ref(null)
  const ready = computed(() => isRuntimeReady(status.value))
  const usable = computed(() => ready.value && !loading.value && !error.value)
  let stopped = false
  let timer
  let started = false
  let revision = 0
  let hydratedRevision = -1
  let refreshing = false

  const hydrate = async () => {
    if (stopped || refreshing || !ready.value || hydratedRevision === revision) return
    refreshing = true
    loading.value = true
    const currentRevision = revision
    const isCurrent = () => !stopped && ready.value && currentRevision === revision
    try {
      await refresh(isCurrent)
      if (isCurrent()) {
        hydratedRevision = currentRevision
        error.value = null
      }
    } catch (cause) {
      if (isCurrent()) error.value = cause
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
        loading.value = true
      }
      status.value = next
    } catch {
      if (stopped) return
      status.value = null
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
  return { status, ready, loading, error, usable, start, stop }
}
