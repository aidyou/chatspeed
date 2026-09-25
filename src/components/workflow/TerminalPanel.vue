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
import { FitAddon, init, Terminal, UrlRegexProvider } from 'ghostty-web'
import { writeClipboard } from '@/libs/clipboard'
import { openUrl } from '@/libs/util'
import { terminalSkinPalette } from '@/constants/terminalThemes'
import { terminalBlockTopRow, terminalClearSequence } from '@/composables/workflow/terminalClear'
import type { TerminalTab } from '@/composables/workflow/useTerminal'

const props = defineProps<{ terminal: any; preferences: any }>()
const { t } = useI18n()
const terminal = props.terminal
const ghosttyReady = ref(false)
const panel = ref<HTMLElement | null>(null)
const panelHeight = computed(() =>
  Math.min(Math.max(180, terminal.height), Math.max(180, window.innerHeight - 160))
)
const hosts = new Map<string, HTMLElement>()
const instances = new Map<
  string,
  { terminal: Terminal; fit: FitAddon; observer: ResizeObserver; disposeSelection: () => void; clearOutputQueue: () => void }
>()
const pageDark = ref(document.documentElement.classList.contains('dark'))
const getCssColor = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim()

const terminalTheme = computed(() => {
  const scheme = props.preferences.colorScheme || 'auto'
  const dark = scheme === 'dark' || (scheme === 'auto' && pageDark.value)
  // A selected skin replaces the application tokens entirely, including the ANSI palette that the
  // application tokens never define.
  const skinPalette = terminalSkinPalette(props.preferences.skin, dark)
  if (skinPalette) return skinPalette
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

// ghostty-web bakes the output limit and the colour scheme into a terminal when it is created, so a
// mounted instance keeps the old values until it is rebuilt.
const configuredScrollback = () =>
  Math.min(Math.max(100, Number(props.preferences.outputLineLimit || 2000)), 20000)
let mountedScrollback = configuredScrollback()
let mountedTheme = terminalTheme.value
const mountedInstancesAreStale = () =>
  mountedScrollback !== configuredScrollback() || mountedTheme !== terminalTheme.value

const tabTitle = (tab: TerminalTab) =>
  `${tab.cwd.split(/[\\/]/).filter(Boolean).pop() || tab.cwd} - ${tab.shellName}`
const selectTab = (sessionId: string) => {
  terminal.activeSessionId = sessionId
}
const focus = (sessionId: string) => instances.get(sessionId)?.terminal.focus()
const shortcutMainKey = (shortcut: string | undefined) => shortcut?.split('+').pop()?.toLowerCase()
// The workflow window already resolves the platform, so prefer that over sniffing the user agent.
const isCommandKeyPlatform = () => {
  const detected = props.preferences.usesCommandKey
  if (typeof detected === 'boolean') return detected
  return /Macintosh|Mac OS/.test(`${navigator.platform} ${navigator.userAgent}`)
}
const matchesTerminalShortcut = (
  event: KeyboardEvent,
  shortcut: string | undefined,
  commandModifierDown = false
) => {
  if (!shortcut) return false
  const parts = shortcut.split('+')
  const requiresCommandOrControl =
    parts.includes('CommandOrControl') || parts.includes('CommandOrCtrl')
  const isMacPlatform = isCommandKeyPlatform()
  const commandOrControlPressed = isMacPlatform
    ? (event.metaKey || commandModifierDown) && !event.ctrlKey
    : (event.ctrlKey || commandModifierDown) && !event.metaKey
  if (requiresCommandOrControl !== commandOrControlPressed) return false
  if (parts.includes('Alt') !== event.altKey || parts.includes('Shift') !== event.shiftKey)
    return false
  const mainKey = shortcutMainKey(shortcut)
  return event.code === `Key${mainKey?.toUpperCase()}` || event.key.toLowerCase() === mainKey
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
  instance.disposeSelection()
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

  let commandModifierDown = false
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Meta' || event.key === 'Control') {
      commandModifierDown = true
      return
    }
    // Both terminal shortcuts are consumed on the way down: the workflow window listener runs after
    // this capture phase and would otherwise repeat the same toggle, leaving the panel unchanged.
    const clears = matchesTerminalShortcut(event, props.preferences.clearShortcut, commandModifierDown)
    const toggles = matchesTerminalShortcut(event, props.preferences.toggleShortcut, commandModifierDown)
    if (!clears && !toggles) return
    event.preventDefault()
    event.stopImmediatePropagation()
    if (clears) terminal.clear(tab.sessionId)
    else terminal.visible = !terminal.visible
  }
  const onKeyUp = (event: KeyboardEvent) => {
    if (event.key === 'Meta' || event.key === 'Control') commandModifierDown = false
  }
  host.addEventListener('keydown', onKeyDown, true)
  host.addEventListener('keyup', onKeyUp, true)
  mountedScrollback = configuredScrollback()
  mountedTheme = terminalTheme.value
  const instance = new Terminal({
    cursorBlink: true,
    convertEol: false,
    fontSize: 13,
    scrollback: mountedScrollback,
    overviewRuler: { width: 10 },
    theme: terminalTheme.value
  })
  const fit = new FitAddon()
  instance.loadAddon(fit)
  instance.open(host)
  const urlProvider = new UrlRegexProvider(instance)
  instance.registerLinkProvider({
    provideLinks(y, callback) {
      urlProvider.provideLinks(y, links => {
        callback(
          links?.map(link => ({
            ...link,
            activate: event => {
              if (event.ctrlKey || event.metaKey) void openUrl(link.text)
            }
          }))
        )
      })
    },
    dispose: () => urlProvider.dispose()
  })
  instance.onData(data => {
    // Forward each xterm input chunk to the per-session FIFO bridge so rapid typing reaches the
    // PTY in order without debounce/coalescing dropping intermediate characters.
    void terminal.write(tab.sessionId, data)
  })
  const copySelection = () => {
    const selected = instance.getSelection()
    if (selected) void writeClipboard(selected)
  }
  host.addEventListener('mouseup', copySelection)
  const disposeSelection = () => {
    host.removeEventListener('mouseup', copySelection)
    host.removeEventListener('keydown', onKeyDown, true)
    host.removeEventListener('keyup', onKeyUp, true)
  }
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
  const joinOutputQueue = (first: Uint8Array) => {
    const totalLength = outputQueue.reduce((length, chunk) => length + chunk.length, first.length)
    const joined = new Uint8Array(totalLength)
    joined.set(first)
    let offset = first.length
    for (const chunk of outputQueue) {
      joined.set(chunk, offset)
      offset += chunk.length
    }
    return joined
  }
  const flushOutputQueue = () => {
    if (disposed || writeInFlight) return
    const first = outputQueue.shift()
    if (!first) return
    const output = joinOutputQueue(first)
    outputQueue = []
    writeInFlight = true
    // xterm's write callback fires only after parser/render consumption. Coalescing queued PTY chunks
    // avoids replaying restored startup output one chunk at a time while preserving byte order.
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
  const bufferRowIndex = (row: number) => instance.buffer.active.length - instance.rows + row
  const retainedInputText = (blockTopRow: number, cursorY: number) => {
    const buffer = instance.buffer.active
    let text = ''
    for (let row = blockTopRow; row <= cursorY; row += 1) {
      text += buffer.getLine(bufferRowIndex(row))?.translateToString(true) || ''
    }
    return text
  }
  instances.set(tab.sessionId, { terminal: instance, fit, observer, disposeSelection, clearOutputQueue })
  terminal.registerWriter(tab.sessionId, {
    write: enqueueOutput,
    clear: () => {
      clearPendingProgress()
      outputQueue = []
      writeInFlight = false
      const { cursorX, cursorY } = instance.buffer.active
      const blockTopRow = terminalBlockTopRow(
        cursorY,
        row => instance.buffer.active.getLine(bufferRowIndex(row))?.isWrapped === true
      )
      const retained = retainedInputText(blockTopRow, cursorY)
      instance.write(terminalClearSequence({ rows: instance.rows, cursorX, cursorY, blockTopRow }))
      // Replayed as the bounded history of a reloaded or remounted session, so the live input line
      // survives instead of a fabricated prompt.
      return new TextEncoder().encode(retained)
    }
  })
  syncSize(tab.sessionId)
}

const reconcile = async () => {
  if (!ghosttyReady.value) return
  const activeIds = new Set(terminal.tabs.map((tab: TerminalTab) => tab.sessionId))
  for (const sessionId of instances.keys()) {
    if (!activeIds.has(sessionId)) disposeTab(sessionId)
  }
  if (!terminal.visible || !terminal.activeTab) return
  await nextTick()
  // A preference changed while the panel was hidden has no live instance rebuilt yet, so apply it
  // here before the tab is mounted.
  if (mountedInstancesAreStale()) rebuildMountedInstances()
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

// The output limit and the colour scheme cannot be changed on a live ghostty terminal, so a mounted
// tab is rebuilt in place; remounting replays the retained history to restore its screen.
const rebuildMountedInstances = () => {
  const focusedSessionId = [...instances.keys()].find(sessionId =>
    hosts.get(sessionId)?.contains(document.activeElement)
  )
  mountedScrollback = configuredScrollback()
  mountedTheme = terminalTheme.value
  for (const sessionId of [...instances.keys()]) {
    const tab = terminal.tabs.find((item: TerminalTab) => item.sessionId === sessionId)
    disposeTab(sessionId)
    if (tab) mountTab(tab)
  }
  if (focusedSessionId) focus(focusedSessionId)
}

let themeObserver: MutationObserver | null = null

// A live ghostty terminal keeps the output limit and the colour palette it was created with, so a
// changed preference rebuilds the mounted instances; a hidden panel does it on the next reconcile.
watch([terminalTheme, () => props.preferences.outputLineLimit], () => {
  if (terminal.visible) rebuildMountedInstances()
})
watch(
  () => [
    terminal.visible,
    terminal.activeSessionId,
    terminal.tabs.map((tab: TerminalTab) => tab.sessionId).join(',')
  ],
  reconcile,
  { immediate: true, flush: 'post' }
)
onMounted(async () => {
  try {
    await init()
    ghosttyReady.value = true
    await reconcile()
  } catch (error) {
    console.error('Failed to initialize ghostty-web:', error)
    return
  }
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
  padding: var(--cs-space-sm);
  overflow: hidden;
  box-sizing: border-box;
  background: var(--workflow-terminal-background);
  font-size: 0;
  caret-color: transparent;

  :deep(br) {
    display: none;
  }
}

.workflow-terminal__content :deep(canvas) {
  display: block;
}
</style>
