import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'

import { invokeWrapper } from '@/libs/tauri'

// Events published by the desktop exit coordinator.
export const QUIT_REQUEST_EVENT = 'quit://request'
export const QUIT_STATUS_EVENT = 'quit://status'

// Phase names mirrored from the Rust coordinator (src-tauri/src/exit_coordinator.rs).
export const QUIT_PHASE_IDLE = 'idle'
export const QUIT_PHASE_CONFIRMING = 'confirming'
export const QUIT_PHASE_STOPPING_RUNTIME = 'stopping_runtime'
export const QUIT_PHASE_EXITING_DESKTOP = 'exiting_desktop'
export const QUIT_PHASE_COMMITTING_EXIT = 'committing_exit'

/**
 * Builds the quit-confirmation state machine.
 *
 * The Tauri command and event APIs are injected, so a test can drive the whole
 * flow without Tauri or a live runtime: `invoke` wraps one command call, and
 * `subscribe` registers one event listener and returns its unlisten function.
 */
export function createQuitConfirmation({ invoke, subscribe }) {
  const active = ref(false)
  const nonce = ref('')
  const host = ref('')
  // The checkbox never starts checked: only an explicit user action turns it on,
  // and every new prompt resets it.
  const stopRuntime = ref(false)
  const phase = ref(QUIT_PHASE_IDLE)
  const error = ref('')
  const submitting = ref(false)

  // A stop in progress owns the dialog: nothing may be submitted or dismissed
  // while the runtime is being observed.
  const busy = computed(
    () => submitting.value || phase.value === QUIT_PHASE_STOPPING_RUNTIME
  )
  const failed = computed(
    () => phase.value === QUIT_PHASE_STOPPING_RUNTIME && Boolean(error.value)
  )

  const reset = () => {
    active.value = false
    nonce.value = ''
    host.value = ''
    stopRuntime.value = false
    phase.value = QUIT_PHASE_IDLE
    error.value = ''
    submitting.value = false
  }

  const open = prompt => {
    // A repeated tray click re-publishes the same prompt: keep the user's current
    // checkbox and error and only start clean for a genuinely new prompt.
    const samePrompt =
      active.value &&
      Boolean(prompt) &&
      prompt.nonce === nonce.value &&
      prompt.host === host.value
    if (prompt) {
      nonce.value = prompt.nonce
      host.value = prompt.host
    }
    if (!samePrompt) {
      stopRuntime.value = false
      error.value = ''
    }
    submitting.value = false
    phase.value = QUIT_PHASE_CONFIRMING
    active.value = true
  }

  const applyStatus = status => {
    if (!status || typeof status.phase !== 'string') return
    phase.value = status.phase
    error.value = status.error || ''
    if (status.prompt) {
      nonce.value = status.prompt.nonce
      host.value = status.prompt.host
    }
    // The dialog stays visible while a prompt is pending; an idle or committed
    // coordinator ends it (a terminal commit owns the process from then on).
    active.value =
      status.phase === QUIT_PHASE_CONFIRMING || status.phase === QUIT_PHASE_STOPPING_RUNTIME
    if (!active.value) {
      submitting.value = false
    }
  }

  /** Submits the current dialog choice; a duplicate submit is ignored. */
  const submit = async () => {
    if (!active.value || submitting.value) return
    // A stop still being observed owns the dialog, but a recorded failure is
    // explicitly retryable and must be allowed through.
    if (phase.value === QUIT_PHASE_STOPPING_RUNTIME && !error.value) return
    submitting.value = true
    try {
      const status = await invoke('quit_confirm', {
        nonce: nonce.value,
        host: host.value,
        stopRuntime: stopRuntime.value
      })
      applyStatus(status)
    } catch (caught) {
      console.error('Failed to submit the quit confirmation:', caught)
      error.value = 'request_failed'
    } finally {
      submitting.value = false
    }
  }

  /** Cancels a pending confirmation and returns the dialog to idle. */
  const cancel = async () => {
    const pendingNonce = nonce.value
    const pendingHost = host.value
    if (phase.value === QUIT_PHASE_CONFIRMING && pendingNonce) {
      try {
        await invoke('quit_cancel', { nonce: pendingNonce, host: pendingHost })
      } catch (caught) {
        console.error('Failed to cancel the quit confirmation:', caught)
      }
    }
    reset()
  }

  /** Reads the pending prompt, so a host that mounted late still sees it. */
  const requestPending = async () => {
    try {
      const status = await invoke('quit_status')
      applyStatus(status)
    } catch (caught) {
      console.error('Failed to read the pending quit prompt:', caught)
    }
  }

  /** Registers the listeners and queries the pending prompt; returns a disposer. */
  const init = async () => {
    const unlistenRequest = await subscribe(QUIT_REQUEST_EVENT, event => open(event?.payload))
    const unlistenStatus = await subscribe(QUIT_STATUS_EVENT, event => applyStatus(event?.payload))
    await requestPending()
    return () => {
      unlistenRequest?.()
      unlistenStatus?.()
    }
  }

  return {
    active,
    nonce,
    host,
    stopRuntime,
    phase,
    error,
    submitting,
    busy,
    failed,
    open,
    applyStatus,
    submit,
    cancel,
    requestPending,
    init,
    reset
  }
}

/** Binds the quit-confirmation machine to the real Tauri event and command APIs. */
export function useQuitConfirmation() {
  const controller = createQuitConfirmation({
    invoke: invokeWrapper,
    subscribe: (event, handler) => listen(event, handler)
  })

  let dispose = null
  onMounted(async () => {
    dispose = await controller.init()
  })
  onBeforeUnmount(() => {
    dispose?.()
  })

  return controller
}