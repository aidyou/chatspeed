import { computed, onBeforeUnmount, reactive } from 'vue'
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

const decode = (data: string) => Uint8Array.from(atob(data), value => value.charCodeAt(0))
const toTab = (session: TerminalSessionWire): TerminalTab => ({
  sessionId: session.session_id,
  shellName: session.shell_name,
  shellPath: session.shell_path,
  cwd: session.cwd,
  alive: session.alive
})

export function useTerminal(currentPaths: { value: string[] }) {
  const state = reactive({
    tabs: [] as TerminalTab[],
    activeSessionId: null as string | null,
    visible: false,
    fullscreen: false,
    height: 400,
    shells: [] as Array<{ name: string; path: string; is_default: boolean }>
  })
  const writers = new Map<string, (data: Uint8Array) => void>()
  const outputBuffers = new Map<string, Uint8Array[]>()
  let unlisteners: Array<() => void> = []

  const hasSessions = computed(() => state.tabs.length > 0)
  const activeTab = computed(() => state.tabs.find(tab => tab.sessionId === state.activeSessionId) || null)

  const syncShells = async () => {
    state.shells = await invokeWrapper('terminal_list_shells')
  }
  const syncSessions = async () => {
    state.tabs = (await invokeWrapper('terminal_list_sessions')).map(toTab)
    state.activeSessionId ||= state.tabs[0]?.sessionId || null
  }
  const create = async (shellPath?: string, cwd = currentPaths.value?.[0] || null) => {
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
  const close = async (sessionId: string) => {
    await invokeWrapper('terminal_close', { sessionId })
    writers.delete(sessionId)
    outputBuffers.delete(sessionId)
    state.tabs = state.tabs.filter(tab => tab.sessionId !== sessionId)
    state.activeSessionId = state.tabs[0]?.sessionId || null
    if (!hasSessions.value) state.visible = false
  }
  const restartWithShell = async (shellPath: string) => {
    const tab = activeTab.value
    if (!tab || tab.shellPath === shellPath) return
    const index = state.tabs.findIndex(item => item.sessionId === tab.sessionId)
    await invokeWrapper('terminal_close', { sessionId: tab.sessionId })
    writers.delete(tab.sessionId)
    outputBuffers.delete(tab.sessionId)
    const replacement = toTab(await invokeWrapper('terminal_create', { cwd: tab.cwd, shellPath }))
    state.tabs.splice(index, 1, replacement)
    state.activeSessionId = replacement.sessionId
    state.visible = true
  }
  const write = (sessionId: string, input: string) => invokeWrapper('terminal_write', { sessionId, input })
  const resize = (sessionId: string, cols: number, rows: number) => invokeWrapper('terminal_resize', { sessionId, cols, rows })
  const registerWriter = (sessionId: string, writer: (data: Uint8Array) => void) => {
    writers.set(sessionId, writer)
    for (const chunk of outputBuffers.get(sessionId) || []) writer(chunk)
    outputBuffers.delete(sessionId)
  }
  const unregisterWriter = (sessionId: string) => writers.delete(sessionId)
  const routeOutput = (sessionId: string, data: Uint8Array) => {
    const writer = writers.get(sessionId)
    if (writer) writer(data)
    else {
      const buffer = outputBuffers.get(sessionId) || []
      buffer.push(data)
      outputBuffers.set(sessionId, buffer.slice(-200))
    }
  }
  const updateCwd = (sessionId: string, cwd: string) => {
    const tab = state.tabs.find(item => item.sessionId === sessionId)
    if (tab && cwd) tab.cwd = cwd
  }
  const initialize = async () => {
    await Promise.all([syncShells(), syncSessions()])
    unlisteners = await Promise.all([
      listen<any>('terminal://output', ({ payload }) => routeOutput(payload.session_id, decode(payload.data_base64))),
      listen<any>('terminal://exit', ({ payload }) => {
        const tab = state.tabs.find(item => item.sessionId === payload.session_id)
        if (tab) tab.alive = false
      }),
      listen('terminal://reset', () => {
        writers.clear()
        outputBuffers.clear()
        state.tabs = []
        state.activeSessionId = null
        state.visible = false
        state.fullscreen = false
      })
    ])
  }

  onBeforeUnmount(() => unlisteners.splice(0).forEach(unlisten => unlisten()))
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
    updateCwd
  })
}
