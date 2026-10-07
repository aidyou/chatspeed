import { invoke } from '@tauri-apps/api/core'
import { defineStore } from 'pinia'
import { ref } from 'vue'

import { parseCapabilityError } from '@/libs/capability.js'

/**
 * Runtime-owned static plugin inventory and lifecycle state.
 *
 * The store deliberately exposes no filesystem or generic command API: every
 * mutation is forwarded by the Tauri adapter to the standalone runtime.
 */
export const usePluginStore = defineStore('plugin', () => {
  const inventory = ref(null)
  const loading = ref(false)
  const applying = ref(false)
  const lastError = ref(null)

  async function invokePlugin(command) {
    try {
      return await invoke(command)
    } catch (error) {
      throw parseCapabilityError(error)
    }
  }

  async function loadInventory() {
    loading.value = true
    lastError.value = null
    try {
      inventory.value = await invokePlugin('plugin_inventory')
      return inventory.value
    } catch (error) {
      lastError.value = error.message
      throw error
    } finally {
      loading.value = false
    }
  }

  async function mutate(command) {
    applying.value = true
    lastError.value = null
    try {
      inventory.value = await invokePlugin(command)
      return inventory.value
    } catch (error) {
      lastError.value = error.message
      throw error
    } finally {
      applying.value = false
    }
  }

  function install() {
    return mutate('plugin_load')
  }

  function disable() {
    return mutate('plugin_disable')
  }

  function uninstall() {
    return mutate('plugin_uninstall')
  }

  return {
    inventory,
    loading,
    applying,
    lastError,
    loadInventory,
    install,
    disable,
    uninstall
  }
})
