<template>
  <!-- The dock column already provides the tab strip and the titlebar, so the trusted surface
       renders the management panel only; a window control here would close the Workflow window
       this view is docked into. -->
  <div class="agent-skills-surface">
    <AgentSkillsPanel v-if="pluginVerified" />
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { emit, listen } from '@tauri-apps/api/event'

import AgentSkillsPanel from '@/components/setting/AgentSkills.vue'
import { agentSkillsVerified } from '@/libs/agentSkillsView'
import { usePluginStore } from '@/stores/plugin'

/** Controlled event the surface raises so the Workflow window releases the tab that opened it. */
const AGENT_SKILLS_INVALIDATED_EVENT = 'cs://agent-skills-invalidated'

const pluginStore = usePluginStore()
const pluginVerified = ref(false)
let disposed = false
let unlistenPlugins = null
let refreshTimer = null

/**
 * Re-reads whether the runtime still confirms the built-in Agent Skills surface.
 *
 * The check fails closed (see `agentSkillsVerified`), so a snapshot the runtime did not confirm
 * unmounts the panel. Instead of closing a window, which would close the Workflow window this
 * surface is docked into, the surface raises one controlled event and lets the Workflow window
 * close the tab and the native view it owns.
 */
async function refreshPluginState() {
  try {
    await pluginStore.loadInventory()
  } catch {
    // The store keeps the unavailable state; the check below then fails closed.
  }
  if (disposed) return

  const verified = agentSkillsVerified(pluginStore.inventory, !!pluginStore.lastError)
  if (verified === pluginVerified.value) return
  pluginVerified.value = verified

  if (!verified) {
    try {
      await emit(AGENT_SKILLS_INVALIDATED_EVENT, {})
    } catch (error) {
      console.error('Failed to report the unavailable Agent Skills surface:', error)
    }
  }
}

onMounted(async () => {
  const unlisten = await listen('cs://plugins-changed', refreshPluginState)
  if (disposed) {
    unlisten()
    return
  }
  unlistenPlugins = unlisten
  // CLI mutations do not emit desktop events, so keep the surface runtime-confirmed.
  refreshTimer = setInterval(refreshPluginState, 5000)
  await refreshPluginState()
})

onUnmounted(() => {
  disposed = true
  clearInterval(refreshTimer)
  unlistenPlugins?.()
})
</script>

<style lang="scss" scoped>
.agent-skills-surface {
  height: 100%;
  overflow: auto;
  background: var(--cs-bg-color);
}
</style>