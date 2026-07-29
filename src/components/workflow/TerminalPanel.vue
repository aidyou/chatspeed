<template>
  <section
    v-show="terminal.visible"
    class="workflow-terminal"
    :class="{ fullscreen: terminal.fullscreen }"
    :style="terminal.fullscreen ? undefined : { height: `${panelHeight}px` }">
    <div class="workflow-terminal__resize" @mousedown="startResize" />
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
        <el-tooltip :content="$t('workflow.terminal.new')"><button type="button" @click="terminal.create()"><cs name="add" /></button></el-tooltip>
        <el-tooltip :content="$t('workflow.terminal.minimize')"><button type="button" @click="terminal.visible = false"><cs name="minimize" /></button></el-tooltip>
        <el-tooltip :content="$t('workflow.terminal.fullscreen')"><button type="button" @click="terminal.fullscreen = !terminal.fullscreen"><cs :name="terminal.fullscreen ? 'fullscreen-off' : 'fullscreen'" /></button></el-tooltip>
        <el-dropdown trigger="click" @command="terminal.restartWithShell">
          <button class="workflow-terminal__shell" type="button"><cs name="bash" />{{ terminal.activeTab?.shellName }}<cs name="caret-down" /></button>
          <template #dropdown><el-dropdown-menu><el-dropdown-item v-for="shell in terminal.shells" :key="shell.path" :command="shell.path">{{ shell.name }}</el-dropdown-item></el-dropdown-menu></template>
        </el-dropdown>
      </div>
    </header>
    <div
      v-for="tab in terminal.tabs"
      :key="tab.sessionId"
      v-show="tab.sessionId === terminal.activeSessionId"
      :ref="element => setHost(tab.sessionId, element)"
      class="workflow-terminal__content"
      @mousedown="focus(tab.sessionId)" />
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import type { TerminalTab } from '@/composables/workflow/useTerminal'

const props = defineProps<{ terminal: any }>()
const terminal = props.terminal
const panelHeight = computed(() => Math.min(Math.max(180, terminal.height), Math.max(180, window.innerHeight - 120)))
const hosts = new Map<string, HTMLElement>()
const instances = new Map<string, { terminal: Terminal; fit: FitAddon; observer: ResizeObserver }>()

const tabTitle = (tab: TerminalTab) => `${tab.cwd.split(/[\\/]/).filter(Boolean).pop() || tab.cwd} -- ${tab.shellName}`
const selectTab = (sessionId: string) => { terminal.activeSessionId = sessionId }
const focus = (sessionId: string) => instances.get(sessionId)?.terminal.focus()

const setHost = (sessionId: string, element: Element | null) => {
  if (element instanceof HTMLElement) hosts.set(sessionId, element)
  else hosts.delete(sessionId)
}

const disposeTab = (sessionId: string) => {
  const instance = instances.get(sessionId)
  if (!instance) return
  terminal.unregisterWriter(sessionId)
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

const mountTab = (tab: TerminalTab) => {
  if (instances.has(tab.sessionId)) return
  const host = hosts.get(tab.sessionId)
  if (!host) return

  const instance = new Terminal({ cursorBlink: true, convertEol: false, fontSize: 13 })
  const fit = new FitAddon()
  instance.loadAddon(fit)
  instance.parser.registerOscHandler(7, uri => {
    try {
      terminal.updateCwd(tab.sessionId, decodeURIComponent(new URL(uri).pathname))
    } catch {
      // Ignore malformed terminal title reports without affecting PTY rendering.
    }
    return true
  })
  instance.open(host)
  instance.onData(data => terminal.write(tab.sessionId, data))
  const observer = new ResizeObserver(() => syncSize(tab.sessionId))
  observer.observe(host)
  instances.set(tab.sessionId, { terminal: instance, fit, observer })
  terminal.registerWriter(tab.sessionId, data => instance.write(data))
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

const closeTab = async (sessionId: string) => {
  await terminal.close(sessionId)
  disposeTab(sessionId)
}

let resizing = false
const resizePanel = (event: MouseEvent) => {
  if (!resizing) return
  terminal.height = Math.min(Math.max(180, window.innerHeight - event.clientY), Math.max(180, window.innerHeight - 120))
}
const stopResize = () => {
  resizing = false
  window.removeEventListener('mousemove', resizePanel)
  window.removeEventListener('mouseup', stopResize)
}
const startResize = () => {
  resizing = true
  window.addEventListener('mousemove', resizePanel)
  window.addEventListener('mouseup', stopResize)
}

watch(() => [terminal.visible, terminal.activeSessionId, terminal.tabs.map((tab: TerminalTab) => tab.sessionId).join(',')], reconcile, { immediate: true, flush: 'post' })
onBeforeUnmount(() => {
  stopResize()
  for (const sessionId of instances.keys()) disposeTab(sessionId)
})
</script>

<style scoped lang="scss">
.workflow-terminal { position: relative; z-index: 4; min-height: 180px; display: flex; flex-direction: column; border-top: 1px solid var(--cs-border-color); background: var(--cs-bg-color); flex-shrink: 0; }
.workflow-terminal.fullscreen { position: absolute; inset: 0; height: auto; z-index: 20; }
.workflow-terminal__resize { position: absolute; top: -3px; left: 0; right: 0; height: 6px; cursor: ns-resize; z-index: 1; }
.workflow-terminal__bar { min-height: 38px; display: flex; align-items: center; border-bottom: 1px solid var(--cs-border-color); background: var(--cs-fill-color-light); }
.workflow-terminal__tabs { display: flex; min-width: 0; overflow-x: auto; flex: 1; }
.workflow-terminal__tab, .workflow-terminal__controls button { border: 0; background: transparent; color: var(--cs-text-color-secondary); cursor: pointer; }
.workflow-terminal__tab { display: inline-flex; align-items: center; gap: 8px; padding: 0 10px; height: 38px; white-space: nowrap; border-right: 1px solid var(--cs-border-color); }
.workflow-terminal__tab.active { color: var(--cs-text-color-primary); background: var(--cs-bg-color); }
.workflow-terminal__controls { display: flex; align-items: center; gap: 3px; padding: 0 8px; }
.workflow-terminal__controls button { display: inline-flex; align-items: center; gap: 5px; padding: 5px; }
.workflow-terminal__shell { max-width: 150px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.workflow-terminal__content { flex: 1; min-height: 0; padding: 8px; overflow: hidden; }
</style>
