import { computed, onBeforeUnmount, reactive, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invokeWrapper } from '@/libs/tauri'

export interface TerminalTab {
  sessionId: string
  shellName: string
  shellPath: string
  cwd: string
  alive: boolean
}

interface TerminalSessionWire {
  session_id: string
  shell_name: string
  shell_path: string
  cwd: string
  alive: boolean
}

const TERMINAL_OUTPUT_STORAGE_KEY = 'chatspeed.workflow-terminal.output.v1'
const TERMINAL_PANEL_STORAGE_KEY = 'chatspeed.workflow-terminal.panel.v1'
const MAX_PERSISTED_TABS = 8
const MAX_PERSISTED_BYTES_PER_TAB = 128 * 1024

const decode = (data: string) => Uint8Array.from(atob(data), value => value.charCodeAt(0))
const encode = (data: Uint8Array) => {
  let binary = ''
  for (let offset = 0; offset < data.length; offset += 0x8000) {
    binary += String.fromCharCode(...data.subarray(offset, offset + 0x8000))
  }
  return btoa(binary)
}
const toTab = (session: TerminalSessionWire): TerminalTab => ({
  sessionId: session.session_id,
  shellName: session.shell_name,
  shellPath: session.shell_path,
  cwd: session.cwd,
  alive: session.alive
})

export function useTerminal(
  currentPaths: { value: string[] },
  preferences: {
    value?: {
      defaultShell?: string
      outputLineLimit?: number
      colorScheme?: 'auto' | 'light' | 'dark'
    }
  } = {}
) {
  const state = reactive({
    tabs: [] as TerminalTab[],
    activeSessionId: null as string | null,
    visible: false,
    fullscreen: false,
    height: 400,
    shells: [] as Array<{ name: string; path: string; is_default: boolean }>
  })
  let restoredPanelState: { activeSessionId?: string; visible?: boolean; fullscreen?: boolean; height?: number } | null = null
  try {
    const stored = JSON.parse(localStorage.getItem(TERMINAL_PANEL_STORAGE_KEY) || 'null')
    if (stored && typeof stored === 'object') restoredPanelState = stored
  } catch {
    localStorage.removeItem(TERMINAL_PANEL_STORAGE_KEY)
  }
  const writers = new Map<string, { write: (data: Uint8Array) => void; clear: () => Uint8Array }>()
  const outputBuffers = new Map<string, { chunks: Uint8Array[]; lines: number }>()
  const outputHistory = new Map<string, { chunks: Uint8Array[]; lines: number }>()
  const pendingInput = new Map<string, Promise<void>>()
  const inputBuffers = new Map<string, string>()
  const inputFlushTimers = new Map<string, number>()
  let persistOutputTimer: number | null = null
  let unlisteners: Array<() => void> = []

  const hasSessions = computed(() => state.tabs.length > 0)
  const activeTab = computed(() => state.tabs.find(tab => tab.sessionId === state.activeSessionId) || null)
  const outputLineLimit = computed(() => {
    const value = Number(preferences.value?.outputLineLimit ?? 2000)
    return Number.isFinite(value) ? Math.min(Math.max(Math.trunc(value), 100), 20000) : 2000
  })
  const requestedShell = computed(() => {
    const configuredShell = preferences.value?.defaultShell
    if (!configuredShell || configuredShell === 'system') return undefined
    return state.shells.find(shell => shell.path === configuredShell || shell.name.toLowerCase() === configuredShell.toLowerCase())?.path
  })
  const countLines = (data: Uint8Array) => data.reduce((count, byte) => count + Number(byte === 10), 0)
  const keepTrailingLines = (data: Uint8Array, maximumLines: number) => {
    let lines = 0
    for (let index = data.length - 1; index >= 0; index -= 1) {
      if (data[index] !== 10) continue
      lines += 1
      if (lines > maximumLines) return data.slice(index + 1)
    }
    return data
  }
  const trimOutput = (buffer: { chunks: Uint8Array[]; lines: number }) => {
    while (buffer.lines > outputLineLimit.value && buffer.chunks.length > 1) {
      const dropped = buffer.chunks.shift()
      if (dropped) buffer.lines -= countLines(dropped)
    }
    if (buffer.lines > outputLineLimit.value && buffer.chunks[0]) {
      buffer.chunks[0] = keepTrailingLines(buffer.chunks[0], outputLineLimit.value)
      buffer.lines = buffer.chunks.reduce((total, chunk) => total + countLines(chunk), 0)
    }
    let byteLength = buffer.chunks.reduce((total, chunk) => total + chunk.length, 0)
    while (byteLength > MAX_PERSISTED_BYTES_PER_TAB && buffer.chunks.length > 1) {
      const dropped = buffer.chunks.shift()
      if (dropped) {
        buffer.lines -= countLines(dropped)
        byteLength -= dropped.length
      }
    }
  }
  const persistOutput = () => {
    persistOutputTimer = null
    try {
      const entries = [...outputHistory.entries()]
        .slice(-MAX_PERSISTED_TABS)
        .map(([sessionId, buffer]) => [sessionId, buffer.chunks.map(encode)])
      localStorage.setItem(TERMINAL_OUTPUT_STORAGE_KEY, JSON.stringify(entries))
    } catch {
      // Output persistence is best-effort and must never prevent terminal input or rendering.
    }
  }
  const scheduleOutputPersistence = () => {
    if (persistOutputTimer !== null) return
    persistOutputTimer = window.setTimeout(persistOutput, 50)
  }
  const restorePersistedOutput = (sessions: TerminalTab[]) => {
    try {
      const stored = JSON.parse(localStorage.getItem(TERMINAL_OUTPUT_STORAGE_KEY) || '[]')
      const activeIds = new Set(sessions.filter(session => session.alive).map(session => session.sessionId))
      for (const entry of Array.isArray(stored) ? stored : []) {
        const [sessionId, chunks] = entry
        if (typeof sessionId !== 'string' || !activeIds.has(sessionId) || !Array.isArray(chunks)) continue
        const restored = { chunks: chunks.filter((chunk): chunk is string => typeof chunk === 'string').map(decode), lines: 0 }
        restored.lines = restored.chunks.reduce((total, chunk) => total + countLines(chunk), 0)
        const existing = outputHistory.get(sessionId)
        const buffer = existing
          ? { chunks: [...restored.chunks, ...existing.chunks], lines: restored.lines + existing.lines }
          : restored
        trimOutput(buffer)
        outputHistory.set(sessionId, buffer)
        const existingBuffer = outputBuffers.get(sessionId)
        const replay = { chunks: [...restored.chunks, ...(existingBuffer?.chunks || [])], lines: restored.lines + (existingBuffer?.lines || 0) }
        trimOutput(replay)
        outputBuffers.set(sessionId, replay)
      }
      for (const sessionId of [...outputHistory.keys()]) {
        if (!activeIds.has(sessionId)) outputHistory.delete(sessionId)
      }
      for (const sessionId of [...outputBuffers.keys()]) {
        if (!activeIds.has(sessionId)) outputBuffers.delete(sessionId)
      }
      persistOutput()
    } catch {
      localStorage.removeItem(TERMINAL_OUTPUT_STORAGE_KEY)
    }
  }

  const persistPanelState = () => {
    if (!hasSessions.value) {
      localStorage.removeItem(TERMINAL_PANEL_STORAGE_KEY)
      return
    }
    localStorage.setItem(
      TERMINAL_PANEL_STORAGE_KEY,
      JSON.stringify({
        activeSessionId: state.activeSessionId,
        visible: state.visible,
        fullscreen: state.fullscreen,
        height: state.height
      })
    )
  }
  watch(
    () => [state.activeSessionId, state.visible, state.fullscreen, state.height, state.tabs.map(tab => tab.sessionId).join(',')],
    persistPanelState,
    { flush: 'sync' }
  )

  const syncShells = async () => {
    state.shells = await invokeWrapper('terminal_list_shells')
  }
  const syncSessions = async () => {
    state.tabs = (await invokeWrapper('terminal_list_sessions')).map(toTab).filter(session => session.alive)
    restorePersistedOutput(state.tabs)
    if (!state.tabs.length) {
      state.activeSessionId = null
      state.visible = false
      state.fullscreen = false
      localStorage.removeItem(TERMINAL_PANEL_STORAGE_KEY)
      restoredPanelState = null
      return
    }

    const restoredActiveId = restoredPanelState?.activeSessionId
    state.activeSessionId = state.tabs.find(tab => tab.sessionId === restoredActiveId)?.sessionId || state.tabs[0]?.sessionId || null
    state.visible = restoredPanelState?.visible === true
    state.fullscreen = state.visible && restoredPanelState?.fullscreen === true
    const restoredHeight = Number(restoredPanelState?.height)
    if (Number.isFinite(restoredHeight)) state.height = Math.min(Math.max(Math.trunc(restoredHeight), 180), window.innerHeight)
    restoredPanelState = null
  }
  const create = async (shellPath = requestedShell.value, cwd = currentPaths.value?.[0] || null) => {
    const session = toTab(await invokeWrapper('terminal_create', { cwd, shellPath: shellPath || null }))
    state.tabs.push(session)
    state.activeSessionId = session.sessionId
    state.visible = true
    return session
  }
  const open = async () => {
    if (!hasSessions.value) await create()
    else state.visible = true
  }
  const removeTab = (sessionId: string) => {
    writers.delete(sessionId)
    outputBuffers.delete(sessionId)
    outputHistory.delete(sessionId)
    pendingInput.delete(sessionId)
    inputBuffers.delete(sessionId)
    const inputFlushTimer = inputFlushTimers.get(sessionId)
    if (inputFlushTimer !== undefined) window.clearTimeout(inputFlushTimer)
    inputFlushTimers.delete(sessionId)
    scheduleOutputPersistence()
    state.tabs = state.tabs.filter(tab => tab.sessionId !== sessionId)
    if (state.activeSessionId === sessionId) state.activeSessionId = state.tabs[0]?.sessionId || null
    if (!hasSessions.value) {
      state.visible = false
      state.fullscreen = false
    }
  }
  const close = async (sessionId: string) => {
    await invokeWrapper('terminal_close', { sessionId })
    removeTab(sessionId)
  }
  const restartWithShell = async (shellPath: string) => {
    const tab = activeTab.value
    if (!tab || tab.shellPath === shellPath) return
    const index = state.tabs.findIndex(item => item.sessionId === tab.sessionId)
    // Create first so a rejected shell/cwd leaves the working tab authoritative and usable.
    const replacement = toTab(await invokeWrapper('terminal_create', { cwd: tab.cwd, shellPath }))
    try {
      await invokeWrapper('terminal_close', { sessionId: tab.sessionId })
    } catch (error) {
      // Do not leak the replacement when the original tab cannot be retired.
      await invokeWrapper('terminal_close', { sessionId: replacement.sessionId }).catch(() => undefined)
      throw error
    }
    writers.delete(tab.sessionId)
    outputBuffers.delete(tab.sessionId)
    outputHistory.delete(tab.sessionId)
    pendingInput.delete(tab.sessionId)
    scheduleOutputPersistence()
    state.tabs.splice(index, 1, replacement)
    state.activeSessionId = replacement.sessionId
    state.visible = true
  }
  const queueWrite = (sessionId: string, input: string) => {
    const previous = pendingInput.get(sessionId) || Promise.resolve()
    const settled = previous
      .catch(() => undefined)
      .then(() => invokeWrapper('terminal_write', { sessionId, input }))
      .catch(() => undefined)
    pendingInput.set(sessionId, settled)
    return settled
  }
  const flushInput = (sessionId: string) => {
    const inputFlushTimer = inputFlushTimers.get(sessionId)
    if (inputFlushTimer !== undefined) window.clearTimeout(inputFlushTimer)
    inputFlushTimers.delete(sessionId)
    const input = inputBuffers.get(sessionId)
    inputBuffers.delete(sessionId)
    return input ? queueWrite(sessionId, input) : Promise.resolve()
  }
  const write = (sessionId: string, input: string) => {
    inputBuffers.set(sessionId, `${inputBuffers.get(sessionId) || ''}${input}`)
    // Submit, control sequences, and paste-sized chunks must reach interactive programs now.
    if (input.includes('\r') || input.includes('\n') || /[\u0000-\u001f]/.test(input) || input.length > 64) {
      return flushInput(sessionId)
    }
    if (!inputFlushTimers.has(sessionId)) {
      inputFlushTimers.set(sessionId, window.setTimeout(() => void flushInput(sessionId), 8))
    }
    return Promise.resolve()
  }
  const resize = (sessionId: string, cols: number, rows: number) => invokeWrapper('terminal_resize', { sessionId, cols, rows })
  const registerWriter = (
    sessionId: string,
    writer: { write: (data: Uint8Array) => void; clear: () => Uint8Array }
  ) => {
    writers.set(sessionId, writer)
    for (const chunk of outputBuffers.get(sessionId)?.chunks || []) writer.write(chunk)
    outputBuffers.delete(sessionId)
  }
  const unregisterWriter = (sessionId: string) => writers.delete(sessionId)
  const appendOutput = (sessionId: string, data: Uint8Array) => {
    const history = outputHistory.get(sessionId) || { chunks: [], lines: 0 }
    history.chunks.push(data)
    history.lines += countLines(data)
    trimOutput(history)
    outputHistory.set(sessionId, history)
    scheduleOutputPersistence()
  }
  const routeOutput = (sessionId: string, data: Uint8Array) => {
    appendOutput(sessionId, data)
    const writer = writers.get(sessionId)
    if (writer) {
      writer.write(data)
      return
    }

    const buffer = outputBuffers.get(sessionId) || { chunks: [], lines: 0 }
    buffer.chunks.push(data)
    buffer.lines += countLines(data)
    trimOutput(buffer)
    outputBuffers.set(sessionId, buffer)
  }
  const clear = (sessionId = state.activeSessionId) => {
    if (!sessionId) return
    const tab = state.tabs.find(item => item.sessionId === sessionId)
    // Clearing the screen should not make a live shell look dead after a page reload. Keep only
    // its current prompt as the new bounded history; subsequent PTY output replaces it naturally.
    const prompt = new TextEncoder().encode(`${tab?.cwd || ''} > `)
    outputBuffers.set(sessionId, { chunks: [prompt], lines: 0 })
    outputHistory.set(sessionId, { chunks: [prompt], lines: 0 })
    scheduleOutputPersistence()
    // xterm keeps the live shell cursor/prompt after clear. Do not write the cached prompt here,
    // otherwise repeated clear shortcuts visibly stack duplicate prompts in the running terminal.
    writers.get(sessionId)?.clear()
  }
  const updateCwd = (sessionId: string, cwd: string) => {
    const tab = state.tabs.find(item => item.sessionId === sessionId)
    if (tab && cwd) tab.cwd = cwd
  }
  const initialize = async () => {
    // Subscribe first so initial session reconciliation cannot miss PTY output during a page reload.
    unlisteners = await Promise.all([
      listen<any>('terminal://output', ({ payload }) => routeOutput(payload.session_id, decode(payload.data_base64))),
      listen<any>('terminal://exit', ({ payload }) => {
        removeTab(payload.session_id)
      }),
      listen('terminal://reset', () => {
        writers.clear()
        outputBuffers.clear()
        outputHistory.clear()
        pendingInput.clear()
        inputBuffers.clear()
        for (const inputFlushTimer of inputFlushTimers.values()) window.clearTimeout(inputFlushTimer)
        inputFlushTimers.clear()
        localStorage.removeItem(TERMINAL_OUTPUT_STORAGE_KEY)
        localStorage.removeItem(TERMINAL_PANEL_STORAGE_KEY)
        state.tabs = []
        state.activeSessionId = null
        state.visible = false
        state.fullscreen = false
      })
    ])
    await Promise.all([syncShells(), syncSessions()])
  }

  const persistBeforePageExit = () => {
    for (const sessionId of inputBuffers.keys()) void flushInput(sessionId)
    persistOutput()
    persistPanelState()
  }
  const persistWhenHidden = () => {
    if (document.visibilityState === 'hidden') persistBeforePageExit()
  }
  window.addEventListener('beforeunload', persistBeforePageExit)
  window.addEventListener('pagehide', persistBeforePageExit)
  document.addEventListener('visibilitychange', persistWhenHidden)

  onBeforeUnmount(() => {
    if (persistOutputTimer !== null) window.clearTimeout(persistOutputTimer)
    persistBeforePageExit()
    window.removeEventListener('beforeunload', persistBeforePageExit)
    window.removeEventListener('pagehide', persistBeforePageExit)
    document.removeEventListener('visibilitychange', persistWhenHidden)
    unlisteners.splice(0).forEach(unlisten => unlisten())
  })
  return Object.assign(state, {
    hasSessions,
    activeTab,
    initialize,
    open,
    create,
    close,
    restartWithShell,
    write,
    resize,
    registerWriter,
    unregisterWriter,
    clear,
    updateCwd
  })
}
