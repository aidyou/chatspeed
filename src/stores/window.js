import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core'

import { csStorageKey } from '@/config/config'
import { csGetStorage, csSetStorage } from '@/libs/util'

/**
 * useWindowStore defines a store for managing window-related state and actions.
 * It includes state for the visibility of the chat sidebar and related operations.
 */
export const useWindowStore = defineStore('window', () => {
  // operating system information, like 'macOS', 'Windows', 'Linux'
  const os = ref('')

  /**
   * Get the operating system information
   * @returns {Promise<string>} The operating system information
   */
  const initOs = async () => {
    if (os.value) {
      return
    }
    try {
      const info = await invoke('get_os_info'); // Directly await the promise
      os.value = info.os.toLowerCase()
    } catch (error) {
      console.error('error on get_os_info:', error)
    }
  }

  /**
   * The close button for macOS is on the left side, so space must be left for the collapse button.
   * @returns {string} The margin left value in px
   */
  const headerMarginLeft = computed(() => {
    return os.value === 'macos' && chatSidebarShow.value ? '0' : '50px'
  })

  /**
   * For Windows and Linux operating systems, the close button is on the right side, so space must be left for the new chat button.
   * @returns {string} The margin right value in px
   */
  const headerMarginRight = computed(() => {
    return os.value === 'macos' ? '0' : (os.value === 'windows' ? '100px' : '70px')
  })

  // Reactive reference to control the visibility of the chat sidebar, initialized from local storage
  const chatSidebarShow = ref(csGetStorage(csStorageKey.chatSidebarShow) || false);

  /**
   * Set the visibility of the chat sidebar and update local storage
   * @param {boolean} value - The new visibility state of the chat sidebar
   */
  const setChatSidebarShow = (value) => {
    csSetStorage(csStorageKey.chatSidebarShow, value);
    chatSidebarShow.value = value || false;
  };

  const assistantAlwaysOnTop = ref(false)

  /**
   * Toggle assistant window always on top state
   */
  const toggleAssistantAlwaysOnTop = async () => {
    try {
      assistantAlwaysOnTop.value = await invoke('toggle_window_always_on_top', {
        windowLabel: 'assistant',
        newState: !assistantAlwaysOnTop.value
      })
      return assistantAlwaysOnTop.value
    } catch (error) {
      console.error('Failed to toggle window always on top:', error)
      return false
    }
  }

  /**
   * Get the always on top state of the assistant window
   * @returns {Promise<boolean>} The always on top state
   */
  const initAssistantAlwaysOnTop = () => {
    invoke('get_window_always_on_top', { windowLabel: 'assistant' }).then(alwaysOnTop => {
      assistantAlwaysOnTop.value = alwaysOnTop || false
    }).catch(error => {
      console.error('Failed to get window always on top:', error)
    })
  }

  initOs()

  return {
    os,
    headerMarginLeft,
    headerMarginRight,
    chatSidebarShow,
    setChatSidebarShow,
    assistantAlwaysOnTop,
    toggleAssistantAlwaysOnTop,
    initAssistantAlwaysOnTop,
  }
})
