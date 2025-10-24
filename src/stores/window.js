import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';

import { csStorageKey } from '@/config/config';
import { csGetStorage, csSetStorage, showMessage } from '@/libs/util';

const windowLabel = getCurrentWebviewWindow().label

/**
 * useWindowStore defines a store for managing window-related state and actions.
 * It includes state for the visibility of the chat sidebar and related operations.
 */
export const useWindowStore = defineStore('window', () => {
  const { t } = useI18n()

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
      const info = await invokeWrapper('get_os_info'); // Directly await the promise
      os.value = info.os.toLowerCase()
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Error on get_os_info: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Error on get_os_info:', error);
      }
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
  const chatSidebarShow = ref(csGetStorage(csStorageKey.chatSidebarShow, false));

  /**
   * Set the visibility of the chat sidebar and update local storage
   * @param {boolean} value - The new visibility state of the chat sidebar
   */
  const setChatSidebarShow = (value) => {
    csSetStorage(csStorageKey.chatSidebarShow, value);
    chatSidebarShow.value = value || false;
  };

  // Reactive reference to control the visibility of the workflow sidebar, initialized from local storage
  const workflowSidebarShow = ref(csGetStorage(csStorageKey.workflowSidebarShow, true));

  /**
   * Set the visibility of the workflow sidebar and update local storage
   * @param {boolean} value - The new visibility state of the workflow sidebar
   */
  const setWorkflowSidebarShow = (value) => {
    csSetStorage(csStorageKey.workflowSidebarShow, value);
    workflowSidebarShow.value = value || false;
  };

  const assistantAlwaysOnTop = ref(false)
  const mainWindowAlwaysOnTop = ref(false)
  const workflowWindowAlwaysOnTop = ref(false)

  /**
   * Toggle window always on top state
   * @param {string} windowLabel - The label of the window to toggle
   * @param {boolean} state - The current always on top state
   * @returns {Promise<boolean>} The new always on top state
   */
  const toggleWindowAlwaysOnTop = async (windowLabel, state) => {
    try {
      const newState = await invokeWrapper('toggle_window_always_on_top', {
        windowLabel,
        newState: !state
      })
      console.log('pin state change to:', newState, ' window label:', windowLabel)
      return newState
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to toggle window always on top: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to toggle window always on top:', error);
      }
      return false
    }
  }

  /**
   * Initialize the always on top state of the specified window
   * @param {string} windowLabel - The label of the window to initialize
   */
  const initWindowAlwaysOnTop = (windowLabel) => {
    invokeWrapper('get_window_always_on_top', { windowLabel })
      .then(state => {
        console.log('pin state:', state, ' window label:', windowLabel)
        switch (windowLabel) {
          case 'assistant':
            assistantAlwaysOnTop.value = state;
            break;
          case 'main':
            mainWindowAlwaysOnTop.value = state;
            break;
          case 'workflow':
            workflowWindowAlwaysOnTop.value = state;
            break;
        }
      })
      .catch(error => {
        if (error instanceof FrontendAppError) {
          console.error(`Failed to get window always on top: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Failed to get window always on top:', error);
        }
      });
  }

  /**
   * Toggle assistant window always on top state
   * @returns {Promise<boolean>} The always on top state
   */
  const toggleAssistantAlwaysOnTop = async () => {
    assistantAlwaysOnTop.value = await toggleWindowAlwaysOnTop('assistant', assistantAlwaysOnTop.value)
  }

  /**
   * Toggle main window always on top state
   * @returns {Promise<boolean>} The always on top state
   */
  const toggleMainWindowAlwaysOnTop = async () => {
    mainWindowAlwaysOnTop.value = await toggleWindowAlwaysOnTop('main', mainWindowAlwaysOnTop.value)
  }

  /**
   * Get the always on top state of the assistant window
   * @returns {Promise<boolean>} The always on top state
   */
  const initAssistantAlwaysOnTop = () => {
    initWindowAlwaysOnTop('assistant')
  }

  /**
   * Get the always on top state of the main window
   * @returns {Promise<boolean>} The always on top state
   */
  const initMainWindowAlwaysOnTop = () => {
    initWindowAlwaysOnTop('main')
  }

  const toggleWorkflowWindowAlwaysOnTop = async () => {
    workflowWindowAlwaysOnTop.value = await toggleWindowAlwaysOnTop('workflow', workflowWindowAlwaysOnTop.value)
  }

  const initWorkflowWindowAlwaysOnTop = () => {
    initWindowAlwaysOnTop('workflow')
  }

  const setMouseEventState = (state) => {
    invokeWrapper('set_mouse_event_state', { state, windowLabel })
      .catch(error => {
        if (error instanceof FrontendAppError) {
          console.error(`Failed to set mouse event state: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Failed to set mouse event state:', error);
        }
      });
  }

  /**
   * Move window to the left or right bottom corner of the current screen
   * @param {string} direction - 'left' or 'right'
   */
  const moveWindowToScreenEdge = async (direction) => {
    try {
      await invokeWrapper('move_window_to_screen_edge', {
        windowLabel: windowLabel,
        direction: direction
      })
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to move window: ${error.toFormattedString()}`, error.originalError);
        showMessage(t('chat.errorOnMoveWindow', { error: error.toFormattedString() }), 'error', 3000);
      } else {
        console.error('Failed to move window:', error)
        showMessage(t('chat.errorOnMoveWindow', { error: error.message || String(error) }), 'error', 3000)
      }
    }
  }

  initOs()

  return {
    os,
    headerMarginLeft,
    headerMarginRight,
    chatSidebarShow,
    setChatSidebarShow,
    workflowSidebarShow,
    setWorkflowSidebarShow,
    assistantAlwaysOnTop,
    toggleAssistantAlwaysOnTop,
    initAssistantAlwaysOnTop,
    mainWindowAlwaysOnTop,
    initMainWindowAlwaysOnTop,
    toggleMainWindowAlwaysOnTop,
    workflowWindowAlwaysOnTop,
    initWorkflowWindowAlwaysOnTop,
    toggleWorkflowWindowAlwaysOnTop,
    setMouseEventState,
    moveWindowToScreenEdge,
    windowLabel,
  }
})
