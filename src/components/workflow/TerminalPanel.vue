<template>
  <section
    v-show="terminal.visible"
    ref="panel"
    class="workflow-terminal"
    :class="{ fullscreen: terminal.fullscreen }"
    :style="terminal.fullscreen ? undefined : { height: `${panelHeight}px` }">
    <div v-if="!terminal.fullscreen" class="workflow-terminal__resize" @mousedown="startResize" />
    <header class="workflow-terminal__bar">
      <div class="workflow-terminal__tabs">
        <button
          v-for="tab in terminal.tabs"
          :key="tab.sessionId"
          class="workflow-terminal__tab"
          :class="{ active: tab.sessionId === terminal.activeSessionId }"
          type="button"
          @click="selectTab(tab.sessionId)">
          <span>{{ tabTitle(tab) }}</span>
          <cs name="close" @click.stop="closeTab(tab.sessionId)" />
        </button>
      </div>
      <div class="workflow-terminal__controls">
        <div class="workflow-terminal__control-group">
          <el-tooltip :content="$t('workflow.terminal.new')">
            <button type="button" @click="terminal.create()">
              <cs name="add" />
            </button>
          </el-tooltip>
          <el-dropdown trigger="click" @command="confirmShellSwitch">
            <button class="workflow-terminal__shell" type="button">
              <cs name="bash" />{{ terminal.activeTab?.shellName }}
              <cs name="caret-down" />
            </button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item
                  v-for="shell in terminal.shells"
                  :key="shell.path"
                  :command="shell.path"
                  >{{ shell.name }}
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
        <span class="workflow-terminal__control-divider" aria-hidden="true" />
        <div class="workflow-terminal__control-group">
          <el-tooltip :content="$t('workflow.terminal.minimize')">
            <button type="button" @click="terminal.visible = false">
              <cs name="minimize" />
            </button>
          </el-tooltip>
          <el-tooltip :content="$t('workflow.terminal.fullscreen')">
            <button type="button" @click="terminal.fullscreen = !terminal.fullscreen">
              <cs :name="terminal.fullscreen ? 'fullscreen' : 'fullscreen-off'" />
            </button>
          </el-tooltip>
        </div>
      </div>
    </header>
    <div
      v-for="tab in terminal.tabs"
      v-show="tab.sessionId === terminal.activeSessionId"
      class="workflow-terminal__content"
      :key="tab.sessionId"
      :ref="element => setHost(tab.sessionId, element)"
      :style="{ '--workflow-terminal-background': terminalTheme.background }"
      @mousedown="focus(tab.sessionId)" />
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessageBox } from 'element-plus'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import type { TerminalTab } from '@/composables/workflow/useTerminal'

const props = defineProps<{ terminal: any; preferences: any }>()
const { t } = useI18n()
const terminal = props.terminal
const preferences = props.preferences
const panel = ref<HTMLElement | null>(null)
const panelHeight = computed(() =>
  Math.min(Math.max(180, terminal.height), Math.max(180, window.innerHeight - 160))
)
const hosts = new Map<string, HTMLElement>()
const instances = new Map<
  string,
  { terminal: Terminal; fit: FitAddon; observer: ResizeObserver; clearOutputQueue: () => void }
>()
const pageDark = ref(document.documentElement.classList.contains('dark'))
const getCssColor = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim()

const terminalTheme = computed(() => {
  const scheme = preferences.colorScheme || 'auto'
  const dark = scheme === 'dark' || (scheme === 'auto' && pageDark.value)
  return dark
    ? {
        background: getCssColor('--cs-terminal-dark-background'),
        foreground: getCssColor('--cs-terminal-dark-foreground'),
        cursor: getCssColor('--cs-terminal-dark-cursor'),
        selectionBackground: getCssColor('--cs-terminal-dark-selection')
      }
    : {
        background: getCssColor('--cs-terminal-light-background'),
        foreground: getCssColor('--cs-terminal-light-foreground'),
        cursor: getCssColor('--cs-terminal-light-cursor'),
        selectionBackground: getCssColor('--cs-terminal-light-selection')
      }
})

const tabTitle = (tab: TerminalTab) =>
  `${tab.cwd.split(/[\\/]/).filter(Boolean).pop() || tab.cwd} - ${tab.shellName}`
const selectTab = (sessionId: string) => {
  terminal.activeSessionId = sessionId
}
const focus = (sessionId: string) => instances.get(sessionId)?.terminal.focus()
const shortcutMainKey = (shortcut: string | undefined) => shortcut?.split('+').pop()?.toLowerCase()
const matchesTerminalShortcut = (event: KeyboardEvent, shortcut: string | undefined) => {
  if (!shortcut) return false
  const parts = shortcut.split('+')
  const requiresCommandOrControl = parts.includes('CommandOrControl')
  const commandOrControlPressed = preferences.usesCommandKey
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey
  if (requiresCommandOrControl !== commandOrControlPressed) return false
  if (parts.includes('Alt') !== event.altKey || parts.includes('Shift') !== event.shiftKey)
    return false
  return event.key.toLowerCase() === shortcutMainKey(shortcut)
}

const setHost = (sessionId: string, element: Element | null) => {
  if (element instanceof HTMLElement) hosts.set(sessionId, element)
  else hosts.delete(sessionId)
}

const disposeTab = (sessionId: string) => {
  const instance = instances.get(sessionId)
  if (!instance) return
  terminal.unregisterWriter(sessionId)
  instance.clearOutputQueue()
  instance.observer.disconnect()
  instance.terminal.dispose()
  instances.delete(sessionId)
}

const syncSize = (sessionId: string) => {
  if (!terminal.visible || terminal.activeSessionId !== sessionId) return
  const instance = instances.get(sessionId)
  if (!instance) return
  instance.fit.fit()
  if (instance.terminal.cols && instance.terminal.rows) {
    terminal.resize(sessionId, instance.terminal.cols, instance.terminal.rows)
  }
}

const cwdFromOsc7 = (uri: string) => {
  const parsed = new URL(uri)
  const pathname = decodeURIComponent(parsed.pathname)
  // OSC 7 represents Windows paths as file://host/C:/path. Convert only that URI form;
  // Unix paths remain untouched.
  return /^\/[A-Za-z]:\//.test(pathname) ? pathname.slice(1).replaceAll('/', '\\') : pathname
}

const mountTab = (tab: TerminalTab) => {
  if (instances.has(tab.sessionId)) return
  const host = hosts.get(tab.sessionId)
  if (!host) return

  const instance = new Terminal({
    cursorBlink: true,
    convertEol: false,
    fontSize: 13,
    scrollback: Math.min(Math.max(100, Number(preferences.outputLineLimit || 2000)), 20000),
    overviewRuler: { width: 10 },
    theme: terminalTheme.value
  })
  const fit = new FitAddon()
  instance.loadAddon(fit)
  instance.parser.registerOscHandler(7, uri => {
    try {
      terminal.updateCwd(tab.sessionId, cwdFromOsc7(uri))
    } catch {
      // Ignore malformed terminal title reports without affecting PTY rendering.
    }
    return true
  })
  instance.open(host)
  instance.attachCustomKeyEventHandler(event => {
    if (event.isComposing || event.key === 'Process' || event.keyCode === 229) {
      return true
    }
    if (matchesTerminalShortcut(event, preferences.clearShortcut)) {
      event.preventDefault()
      terminal.clear(tab.sessionId)
      return false
    }
    if (matchesTerminalShortcut(event, preferences.toggleShortcut)) {
      event.preventDefault()
      terminal.visible = !terminal.visible
      return false
    }
    return true
  })
  instance.onData(data => {
    // Forward each xterm input chunk to the per-session FIFO bridge so rapid typing reaches the
    // PTY in order without debounce/coalescing dropping intermediate characters.
    void terminal.write(tab.sessionId, data)
  })
  const observer = new ResizeObserver(() => syncSize(tab.sessionId))
  observer.observe(host)
  let outputQueue: Uint8Array[] = []
  let pendingProgressChunk: Uint8Array | null = null
  let pendingProgressTimer: number | null = null
  let writeInFlight = false
  let disposed = false
  const joinOutput = (first: Uint8Array, second: Uint8Array) => {
    const joined = new Uint8Array(first.length + second.length)
    joined.set(first)
    joined.set(second, first.length)
    return joined
  }
  const containsLineControl = (data: Uint8Array) => data.includes(10) || data.includes(13)
  const mayBeSplitProgressLine = (data: Uint8Array) => {
    if (data.length < Math.max(40, instance.cols)) return false
    if (containsLineControl(data)) return false
    return data.at(-1) === 32
  }
  const flushPendingProgressChunk = () => {
    if (pendingProgressTimer !== null) window.clearTimeout(pendingProgressTimer)
    pendingProgressTimer = null
    if (!pendingProgressChunk) return
    outputQueue.push(pendingProgressChunk)
    pendingProgressChunk = null
    flushOutputQueue()
  }
  const flushOutputQueue = () => {
    if (disposed || writeInFlight) return
    const output = outputQueue.shift()
    if (!output) return
    writeInFlight = true
    // xterm's write callback fires only after parser/render consumption. Serializing PTY chunks
    // through that callback preserves CR/CSI progress updates even when Tauri emits rapidly.
    instance.write(output, () => {
      writeInFlight = false
      flushOutputQueue()
    })
  }
  const enqueueOutput = (data: Uint8Array) => {
    if (!data.length) return
    const output = pendingProgressChunk ? joinOutput(pendingProgressChunk, data) : data
    if (pendingProgressTimer !== null) window.clearTimeout(pendingProgressTimer)
    pendingProgressChunk = null
    pendingProgressTimer = null

    // Cargo can split a padded CR progress update as "long line" then a standalone CR in the next
    // PTY event. Writing the padded line before its CR lets xterm enter pending-wrap state and the
    // later CR cannot fully undo the visual wrap, so briefly coalesce likely split progress lines.
    if (mayBeSplitProgressLine(output)) {
      pendingProgressChunk = output
      pendingProgressTimer = window.setTimeout(flushPendingProgressChunk, 8)
      return
    }

    outputQueue.push(output)
    flushOutputQueue()
  }
  const clearPendingProgress = () => {
    if (pendingProgressTimer !== null) window.clearTimeout(pendingProgressTimer)
    pendingProgressTimer = null
    pendingProgressChunk = null
  }
  const clearOutputQueue = () => {
    disposed = true
    clearPendingProgress()
    outputQueue = []
  }
  instances.set(tab.sessionId, { terminal: instance, fit, observer, clearOutputQueue })
  terminal.registerWriter(tab.sessionId, {
    write: enqueueOutput,
    clear: () => {
      clearPendingProgress()
      outputQueue = []
      writeInFlight = false
      instance.clear()
      return new Uint8Array()
    }
  })
  syncSize(tab.sessionId)
}

const reconcile = async () => {
  const activeIds = new Set(terminal.tabs.map((tab: TerminalTab) => tab.sessionId))
  for (const sessionId of instances.keys()) {
    if (!activeIds.has(sessionId)) disposeTab(sessionId)
  }
  if (!terminal.visible || !terminal.activeTab) return
  await nextTick()
  mountTab(terminal.activeTab)
  syncSize(terminal.activeTab.sessionId)
  focus(terminal.activeTab.sessionId)
}

const confirmShellSwitch = async (shellPath: string) => {
  if (!terminal.activeTab || terminal.activeTab.shellPath === shellPath) return
  try {
    await ElMessageBox.confirm(
      t('workflow.terminal.switchShellConfirmMessage'),
      t('workflow.terminal.switchShellConfirmTitle'),
      { type: 'warning' }
    )
  } catch {
    return
  }
  await terminal.restartWithShell(shellPath)
}

const closeTab = async (sessionId: string) => {
  try {
    await ElMessageBox.confirm(
      t('workflow.terminal.closeConfirmMessage'),
      t('workflow.terminal.closeConfirmTitle'),
      { type: 'warning' }
    )
  } catch {
    return
  }
  await terminal.close(sessionId)
  disposeTab(sessionId)
}

let resizing = false
const resizePanel = (event: MouseEvent) => {
  if (!resizing) return
  const containerBottom =
    panel.value?.parentElement?.getBoundingClientRect().bottom ?? window.innerHeight
  const maxHeight = Math.max(180, containerBottom - 160)
  terminal.height = Math.min(Math.max(180, containerBottom - event.clientY), maxHeight)
}
const stopResize = () => {
  resizing = false
  window.removeEventListener('mousemove', resizePanel)
  window.removeEventListener('mouseup', stopResize)
}
const startResize = (event: MouseEvent) => {
  event.preventDefault()
  resizing = true
  window.addEventListener('mousemove', resizePanel)
  window.addEventListener('mouseup', stopResize)
}

let themeObserver: MutationObserver | null = null

watch(terminalTheme, theme => {
  for (const instance of instances.values()) instance.terminal.options.theme = theme
})
watch(
  () => preferences.outputLineLimit,
  limit => {
    const scrollback = Math.min(Math.max(100, Number(limit || 2000)), 20000)
    for (const instance of instances.values()) instance.terminal.options.scrollback = scrollback
  }
)
watch(
  () => [
    terminal.visible,
    terminal.activeSessionId,
    terminal.tabs.map((tab: TerminalTab) => tab.sessionId).join(',')
  ],
  reconcile,
  { immediate: true, flush: 'post' }
)
onMounted(() => {
  themeObserver = new MutationObserver(() => {
    pageDark.value = document.documentElement.classList.contains('dark')
  })
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
})
onBeforeUnmount(() => {
  stopResize()
  themeObserver?.disconnect()
  for (const sessionId of instances.keys()) disposeTab(sessionId)
})
</script>

<style scoped lang="scss">
.workflow-terminal {
  position: relative;
  z-index: 4;
  min-height: 180px;
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--cs-border-color);
  background: var(--cs-bg-color);
  flex-shrink: 0;
}

.workflow-terminal.fullscreen {
  position: absolute;
  inset: 0;
  height: auto;
  z-index: 20;
}

.workflow-terminal__resize {
  position: absolute;
  top: -3px;
  left: 0;
  right: 0;
  height: 6px;
  cursor: ns-resize;
  z-index: 1;
}

.workflow-terminal__bar {
  min-height: 38px;
  display: flex;
  align-items: center;
  border-bottom: 1px solid var(--cs-border-color);
  background: var(--cs-fill-color-light);
}

.workflow-terminal__tabs {
  display: flex;
  min-width: 0;
  overflow-x: auto;
  flex: 1;
}

.workflow-terminal__tab,
.workflow-terminal__controls button {
  border: 0;
  background: transparent;
  color: var(--cs-text-color-secondary);
  cursor: pointer;
}

.workflow-terminal__tab {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  height: 38px;
  white-space: nowrap;
  border-right: 1px solid var(--cs-border-color);
}

.workflow-terminal__tab.active {
  color: var(--cs-text-color-primary);
  background: var(--cs-bg-color);
}

.workflow-terminal__controls {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  padding: 0 var(--cs-space-sm);
}

.workflow-terminal__control-group {
  display: flex;
  align-items: center;
  gap: 3px;
}

.workflow-terminal__control-divider {
  width: 1px;
  height: 16px;
  background: var(--cs-border-color);
}

.workflow-terminal__controls button {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px;
}

.workflow-terminal__shell {
  max-width: 150px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.workflow-terminal__content {
  flex: 1;
  min-height: 0;
  padding: 0;
  overflow: hidden;
  box-sizing: border-box;
}

.workflow-terminal__content :deep(.xterm) {
  width: 100%;
  height: 100%;
  padding: var(--cs-space-sm);
  box-sizing: border-box;
  background: inherit;
}

.workflow-terminal__content :deep(.xterm-screen) {
  max-width: 100%;
  padding-bottom: var(--cs-space-sm);
}

.workflow-terminal__content :deep(.xterm-viewport) {
  background-color: var(--workflow-terminal-background);
}
</style>
