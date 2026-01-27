import { defineStore } from 'pinia'

import { ref } from 'vue'

import { invokeWrapper, FrontendAppError } from '@/libs/tauri'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

import { sendSyncState } from '@/libs/sync'
import { isEmpty, snakeToCamel, camelToSnake } from '@/libs/util'
import { mapBrowserLangToStandard } from '@/i18n/langUtils'
import i18n from '@/i18n'

const windowLabel = getCurrentWebviewWindow().label

const defaultSettings = {
  httpServer: 'http://127.0.0.1:21912',
  interfaceLanguage: mapBrowserLangToStandard(navigator.language),
  primaryLanguage: mapBrowserLangToStandard(navigator.language),
  secondaryLanguage: 'en',
  theme: 'system',
  codeLightTheme: 'atom-one-light',
  codeDarkTheme: 'github-dark',
  showMenuButton: true,
  // chat settings
  historyMessages: 5,
  conversationTitleGenModel: { id: '', model: '' },
  sendMessageKey: 'Enter',
  // shortcut settings
  mainWindowVisibleShortcut: 'F2',
  noteWindowVisibleShortcut: 'ALT+N',
  assistantWindowVisibleShortcut: 'ALT+Z',
  assistantWindowVisibleAndPasteShortcut: 'ALT+S',
  proxySwitcherWindowVisibleShortcut: 'Alt+Shift+P',
  moveWindowLeftShortcut: 'Alt+Shift+ArrowLeft',
  moveWindowRightShortcut: 'Alt+Shift+ArrowRight',
  centerWindowShortcut: 'Alt+Shift+C',
  // network settings
  proxyType: 'none', // none, http, system
  proxyServer: '',
  proxyUsername: '',
  proxyPassword: '',
  // other settings
  wordSelectionToolbar: false,
  autoStart: false,
  autoUpdate: true,
  backupDir: '',
  // workflow settings
  workflowReasoningModel: {
    id: '',
    model: ''
  },
  workflowGeneralModel: {
    id: '',
    model: ''
  },
  // chat completion proxy settings
  // Allows defining grouped model aliases.
  // Each top-level key is a “group name”; within that group lives the usual
  // alias->targets mapping described below.
  //
  // Structure:
  //   "group-name": {
  //     "client-model-alias": [
  //       { "id": 1, "model": "provider-specific-model-name-A" },
  //       { "id": 2, "model": "provider-specific-model-name-B" }
  //     ],
  //     ...
  //   }
  //
  // - The outer key (group name) is just a namespace to keep aliases organized.
  // - The inner key (alias) is the public identifier the client uses.
  // - Each object in the array specifies:
  //     id    – an AI provider configuration id (endpoint, key, etc.)
  //     model – the exact model name that provider should call
  chatCompletionProxy: {},
  // Chat completion proxy authentication keys.
  // This array stores a list of keys that can be used by clients
  // to authenticate with the chat completion proxy service.
  // Each key object has a token and a descriptive name.
  //
  // Example:
  // [
  //   { "token": "cs-secure-token-for-client-A", "name": "Client A Access Key" },
  //   { "token": "cs-another-very-secret-token", "name": "Analytics Service Key" }
  // ]
  //
  // - Each object in the array represents an authentication key:
  //   - `token`: The actual secret string (API key or bearer token) that clients
  //              must provide to use the proxy service.
  //   - `name`: A human-readable name or description for the token,
  //             useful for managing multiple keys (e.g., "WebApp Client Key", "Mobile App Key").
  chatCompletionProxyKeys: [],
  chatCompletionProxyPort: 11434,
  chatCompletionProxyLogToFile: false,
  chatCompletionProxyLogProxyToFile: false,
  // Search
  googleApiKey: '',
  googleSearchId: '',
  serperApiKey: '',
  tavilyApiKey: '',
  websearchModel: { id: '', model: '' },
  // vision model settings
  visionModel: { id: '', model: '' },
  searchEngine: '',
  scraperConcurrencyCount: 5
}

/**
 * useSettingStore defines a store for managing application settings.
 * It includes state for the list of settings and related operations.
 */
export const useSettingStore = defineStore('setting', () => {
  /**
   * settings is a key-value pair object
   */
  const settings = ref({ ...defaultSettings })
  const env = ref({
    httpServer: '',
    logDir: '',
    schemaDir: '',
    chatCompletionProxy: ''
  })

  /**
   * Submits configuration to the database and updates local configuration cache upon success.
   * @param {string} key - The setting key to update.
   * @param {any} value - The value to set for the specified key.
   * @returns {Promise<void>} A promise that resolves when the setting is successfully updated.
   */
  const setSetting = async (key, value) => {
    // Convert camelCase to snake_case
    const dbKey = camelToSnake(key)

    // Update shortcut if the key is for a main or assistant window shortcut setting
    //
    // IMPORTANT:
    //  We must ensure the shortcut binding is successful before updating the database
    if (
      key === 'mainWindowVisibleShortcut' ||
      key === 'assistantWindowVisibleShortcut' ||
      key === 'assistantWindowVisibleAndPasteShortcut' ||
      key === 'moveWindowLeftShortcut' ||
      key === 'moveWindowRightShortcut' ||
      key === 'centerWindowShortcut' ||
      key === 'noteWindowVisibleShortcut' ||
      key === 'proxySwitcherWindowVisibleShortcut'
    ) {
      try {
        await invokeWrapper('update_shortcut', { key: dbKey, value })
      } catch (error) {
        if (error instanceof FrontendAppError) {
          console.error(`Failed to update shortcut: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Failed to update shortcut:', error);
        }
        throw new Error(i18n.global.t('settings.general.updateShortcutFailed', { error: error.message || String(error) }))
      }
    }

    return invokeWrapper('set_config', { key: dbKey, value }).then(() => {
      settings.value = {
        ...settings.value,
        [key]: value
      }
      sendSyncState('setting_changed', windowLabel, { [key]: value })
    }).catch(error => {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to set config: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to set config:', error);
      }
      throw error;
    })
  }

  /**
   * Fetches all application settings from the backend and updates the state.
   * Uses Tauri's invoke method to call the backend command `get_all_settings`.
   * If the result is empty, it sets the settings to an empty object.
   * @returns {Promise<void>} A promise that resolves when the settings are successfully updated.
   */
  const updateSettingStore = () => {
    return new Promise((resolve, reject) => {
      invokeWrapper('get_all_config')
        .then(result => {
          // Update the entire object reactively
          settings.value = {
            ...defaultSettings
          }
          if (!isEmpty(result)) {
            Object.keys(result).forEach(x => {
              settings.value[snakeToCamel(x)] = result[x]
            })
            console.debug('settings', settings.value)
          }
          resolve()
        })
        .catch(error => {
          if (error instanceof FrontendAppError) {
            console.error(`Failed to update setting store: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Failed to update setting store:', error);
          }
          reject(error);
        })
    })
  }

  const setTextMonitor = start => {
    return new Promise((resolve, reject) => {
      if (start) {
        invokeWrapper('start_text_monitor', { force: true })
          .then(() => {
            setSetting('wordSelectionToolbar', true).then(resolve)
          })
          .catch(err => {
            settings.value.wordSelectionToolbar = false
            invokeWrapper('open_text_selection_permission_settings')
            reject(
              i18n.global.t('settings.general.startWordSelectionToolbarFailed', { error: err.message || String(err) })
            )
          })
      } else {
        invokeWrapper('stop_text_monitor')
          .then(() => {
            setSetting('wordSelectionToolbar', false).then(resolve)
          })
          .catch(err => {
            settings.value.wordSelectionToolbar = true
            reject(i18n.global.t('settings.general.stopWordSelectionToolbarFailed', { error: err.message || String(err) }))
          })
      }
    })
  }

  const reloadConfig = () => {
    return new Promise((resolve, reject) => {
      invokeWrapper('reload_config')
        .then(() => {
          updateSettingStore().then(resolve)
        })
        .catch(error => {
          if (error instanceof FrontendAppError) {
            console.error(`Failed to reload config: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Failed to reload config:', error);
          }
          reject(error);
        })
    })
  }

  const updateTray = () => {
    return new Promise((resolve, reject) => {
      invokeWrapper('update_tray')
        .then(() => {
          resolve()
        })
        .catch(error => {
          if (error instanceof FrontendAppError) {
            console.error(`Failed to update tray: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Failed to update tray:', error);
          }
          reject(error);
        })
    })
  }

  const getEnv = (setLanguage = false) => {
    return new Promise((resolve, reject) => {
      invokeWrapper('get_env')
        .then(result => {
          if (setLanguage && result.language) {
            settings.value.interfaceLanguage = result.language
            settings.value.primaryLanguage = result.language
            settings.value.secondaryLanguage = result.language !== 'en' ? result.language : 'en'
            console.log('Language updated:', result.language)
          }
          env.value = { ...env.value, ...result }
          resolve()
        })
        .catch(error => {
          if (error instanceof FrontendAppError) {
            console.error(`Failed to get env: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Failed to get env:', error);
          }
          reject(error);
        })
    })
  }

  return {
    windowLabel,
    settings,
    setSetting,
    updateSettingStore,
    setTextMonitor,
    reloadConfig,
    updateTray,
    env,
    getEnv
  }
})
