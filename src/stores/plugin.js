import { invoke } from '@tauri-apps/api/core'
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { parseCapabilityError } from '@/libs/capability.js'

/**
 * Runtime-owned plugin inventory and lifecycle state.
 *
 * The runtime answers every lifecycle call with one collection snapshot
 * (`{ schema_version, plugins: [...] }`), so the store mirrors that collection
 * and derives the views the UI needs instead of reading the filesystem. It
 * deliberately exposes no filesystem or generic command API: every mutation is
 * forwarded by the Tauri adapter to the standalone runtime.
 */
export const usePluginStore = defineStore('plugin', () => {
  const inventory = ref(null)
  const loading = ref(false)
  const applying = ref(false)
  const lastError = ref(null)

  // A monotonic token shared by every inventory read and mutation. A refresh
  // that started earlier must never commit after a newer mutation, so each
  // operation only adopts its snapshot while its token is still current.
  let revision = 0

  /** Every plugin the runtime reports, in inventory order. */
  const plugins = computed(() => inventory.value?.plugins ?? [])
  /** The runtime's built-in plugins (`kind === 'builtin'`). */
  const builtinPlugins = computed(() =>
    plugins.value.filter(plugin => plugin?.kind === 'builtin')
  )
  /**
   * Built-in plugins whose static UI the runtime verified and that are enabled.
   * Any failure hides the whole surface: a snapshot the runtime did not confirm
   * is never rendered.
   */
  const verifiedUiPlugins = computed(() => {
    if (!inventory.value || lastError.value) return []
    return builtinPlugins.value.filter(
      plugin => plugin?.state === 'enabled' && plugin?.ui?.verified === true
    )
  })

  async function invokePlugin(command) {
    try {
      return await invoke(command)
    } catch (error) {
      throw parseCapabilityError(error)
    }
  }

  async function loadInventory() {
    if (applying.value || loading.value) return inventory.value
    const mine = ++revision
    loading.value = true
    try {
      const snapshot = await invokePlugin('plugin_inventory')
      // A later mutation supersedes this read; keep its newer snapshot.
      if (mine !== revision) return inventory.value
      inventory.value = snapshot
      lastError.value = null
      return snapshot
    } catch (error) {
      // A read the runtime could not answer is not a valid snapshot, so the
      // surface fails closed instead of presenting stale plugin state.
      if (mine === revision) {
        inventory.value = null
        lastError.value = error.message
      }
      throw error
    } finally {
      loading.value = false
    }
  }

  /**
   * Runs one lifecycle command and adopts the snapshot it returns, so the UI
   * never derives state the runtime did not report.
   */
  async function mutate(command) {
    const mine = ++revision
    applying.value = true
    try {
      const snapshot = await invokePlugin(command)
      if (mine !== revision) return inventory.value
      inventory.value = snapshot
      lastError.value = null
      return snapshot
    } catch (error) {
      // A failed mutation keeps the last authoritative snapshot, and the error
      // hides the UI until a later read confirms the runtime again.
      if (mine === revision) lastError.value = error.message
      throw error
    } finally {
      applying.value = false
    }
  }

  /** Stages, verifies and publishes the built-in bundle (also re-enables it). */
  function install() {
    return mutate('plugin_load')
  }

  /** Re-enables a disabled bundle; the runtime publishes it enabled. */
  function enable() {
    return mutate('plugin_load')
  }

  /** Marks the installed bundle disabled without touching its assets. */
  function disable() {
    return mutate('plugin_disable')
  }

  /** Removes the plugin-owned bundle only, never installed Skills. */
  function uninstall() {
    return mutate('plugin_uninstall')
  }

  return {
    inventory,
    loading,
    applying,
    lastError,
    plugins,
    builtinPlugins,
    verifiedUiPlugins,
    loadInventory,
    install,
    enable,
    disable,
    uninstall
  }
})
