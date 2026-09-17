import { defineStore } from 'pinia'
import { reactive, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'

import { FrontendAppError, invokeWrapper } from '@/libs/tauri'

/**
 * @typedef {Object} ChatHub
 * @property {number} id - SQLite assigned row id.
 * @property {string} name - Display name shown in the entry list.
 * @property {string} logo - Remote logo url, empty means "use the letter avatar".
 * @property {string} url - Site url opened by the embedded webview.
 * @property {number} sortIndex - Persisted display order.
 * @property {boolean} isDefault - Whether the entry was seeded by the app.
 */

const reportError = (action, error) => {
  if (error instanceof FrontendAppError) {
    console.error(`Failed to ${action}: ${error.toFormattedString()}`, error.originalError)
  } else {
    console.error(`Failed to ${action}:`, error)
  }
}

/**
 * Width the docked page opens with, in logical pixels.
 *
 * A chat site opens at the width it is comfortable to read in, and the workflow window is
 * widened by that width so the workflow UI keeps the width it had. The splitter can still
 * drag the page down to the narrowest width the backend accepts. The backend owns the
 * authoritative limits and is asked for them on mount; this value only covers the moment
 * before they arrive.
 */
const DEFAULT_PAGE_WIDTH = 600

/**
 * useChatHubStore holds the independent list of ChatHub (web chat entry)
 * metadata. It is intentionally separate from `useSettingStore`: entries are
 * stored in their own database table, never in the generic config map.
 */
export const useChatHubStore = defineStore('chat_hub', () => {
  const list = reactive([])
  const loading = ref(false)
  const activeHubId = ref(0)
  /**
   * How the platform makes room for the docked page: `split` means the workflow UI is
   * already narrower, `reserve` means the page is stacked over it and this side has to
   * keep the space free.
   */
  const viewMode = ref('split')
  /** Width of the docked page, in logical pixels. */
  const pageWidth = ref(DEFAULT_PAGE_WIDTH)
  /** Width limits enforced by the backend, in logical pixels. */
  const pageMinWidth = ref(DEFAULT_PAGE_WIDTH)
  const pageMinHostWidth = ref(480)
  let unlistenSync = null

  const load = async () => {
    loading.value = true
    try {
      const items = await invokeWrapper('get_all_chat_hubs')
      list.splice(0, list.length, ...items)
      if (activeHubId.value && !list.some(hub => hub.id === activeHubId.value)) {
        activeHubId.value = 0
      }
      return list
    } catch (error) {
      reportError('load chat hubs', error)
      throw error
    } finally {
      loading.value = false
    }
  }

  const add = async ({ name, logo, url }) => {
    try {
      const hub = await invokeWrapper('add_chat_hub', { name, logo, url })
      list.push(hub)
      return hub
    } catch (error) {
      reportError('add chat hub', error)
      throw error
    }
  }

  const update = async ({ id, name, logo, url }) => {
    try {
      const hub = await invokeWrapper('update_chat_hub', { id, name, logo, url })
      const index = list.findIndex(item => item.id === hub.id)
      if (index !== -1) {
        list.splice(index, 1, hub)
      }
      return hub
    } catch (error) {
      reportError('update chat hub', error)
      throw error
    }
  }

  const remove = async id => {
    try {
      await invokeWrapper('delete_chat_hub', { id })
      const index = list.findIndex(item => item.id === id)
      if (index !== -1) {
        list.splice(index, 1)
      }
    } catch (error) {
      reportError('delete chat hub', error)
      throw error
    }
  }

  /**
   * Persists the exact id order produced by a drag and drop reorder. The backend
   * rejects partial or duplicated lists, so the local list is only reordered from
   * a confirmed result.
   */
  const reorder = async hubIds => {
    try {
      await invokeWrapper('update_chat_hub_order', { hubIds })
      const ordered = hubIds
        .map(id => list.find(hub => hub.id === id))
        .filter(Boolean)
      if (ordered.length === list.length) {
        ordered.forEach((hub, index) => {
          hub.sortIndex = index
        })
        list.splice(0, list.length, ...ordered)
      }
    } catch (error) {
      reportError('reorder chat hubs', error)
      throw error
    }
  }

  const setActiveHub = id => {
    activeHubId.value = id
  }

  /**
   * Reads how this platform makes room for the docked page, and the width limits it
   * accepts.
   *
   * Both are owned by the backend, so the splitter clamps a drag with exactly the
   * limits the carrier enforces and the reserved space always matches the page.
   */
  const loadViewLayout = async () => {
    try {
      const [mode, limits] = await Promise.all([
        invokeWrapper('get_chat_hub_view_mode'),
        invokeWrapper('get_chat_hub_page_limits')
      ])

      viewMode.value = mode === 'reserve' ? 'reserve' : 'split'

      const minWidth = Number(limits?.minWidth)
      const minHostWidth = Number(limits?.minHostWidth)
      if (Number.isFinite(minWidth) && minWidth > 0) {
        pageMinWidth.value = minWidth
      }
      if (Number.isFinite(minHostWidth) && minHostWidth > 0) {
        pageMinHostWidth.value = minHostWidth
      }
      if (pageWidth.value < pageMinWidth.value) {
        pageWidth.value = pageMinWidth.value
      }
    } catch (error) {
      reportError('read the chat page layout', error)
    }
  }

  /**
   * Records the width the splitter settled on. The backend clamps it again, so this is
   * only the view side of one shared value.
   */
  const setPageWidth = width => {
    if (Number.isFinite(width) && width > 0) {
      pageWidth.value = width
    }
  }

  /**
   * Refreshes the list whenever another window changes ChatHub entries.
   */
  const startSyncListener = async () => {
    if (unlistenSync) {
      return
    }
    unlistenSync = await listen('cs://sync-state', event => {
      if (event?.payload?.type === 'chat_hubs') {
        load().catch(() => {})
      }
    })
  }

  const stopSyncListener = () => {
    if (unlistenSync) {
      unlistenSync()
      unlistenSync = null
    }
  }

  return {
    list,
    loading,
    activeHubId,
    viewMode,
    pageWidth,
    pageMinWidth,
    pageMinHostWidth,
    load,
    add,
    update,
    remove,
    reorder,
    setActiveHub,
    loadViewLayout,
    setPageWidth,
    startSyncListener,
    stopSyncListener
  }
})