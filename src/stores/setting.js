import { defineStore } from 'pinia';

import { ref } from 'vue';

import { invoke } from '@tauri-apps/api/core'
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
  chatspeedCrawler: '',
  historyMessages: 1,
  conversationTitleGenModel: { id: '', model: '' },
  websearchModel: { id: '', model: '' },
  searchEngine: [],
  searchResultCount: 20,
  // shortcut settings
  mainWindowVisibleShortcut: 'F2',
  noteWindowVisibleShortcut: 'ALT+N',
  assistantWindowVisibleShortcut: 'ALT+Z',
  assistantWindowVisibleAndPasteShortcut: 'ALT+S',
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
  // This allows mapping a client-facing model alias (key) to one or more
  // actual backend model configurations (value as an array).
  // This enables features like load balancing or model fallback for a given alias.
  //
  // Example:
  // "client-model-alias": [
  //   { "id": 1, "model": "provider-specific-model-name-A" }, // Target model A from provider 1
  //   { "id": 2, "model": "provider-specific-model-name-B" }  // Target model B from provider 2
  // ]
  //
  // - The `key` (e.g., "client-model-alias") is the identifier the application uses
  //   when requesting a chat completion through the proxy.
  // - The `value` is an array, where each object specifies a target backend model:
  //   - `id`: Refers to the ID of an AI model provider configuration
  //           (e.g., an entry in your `ai_model` database table or store).
  //           This provider configuration contains the API endpoint, key, etc.
  //   - `model`: The specific model string (e.g., "gpt-4-turbo", "claude-3-opus")
  //              to be used with the provider identified by `id`. This string should
  //              correspond to a model supported by that provider.
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
  chatCompletionProxyKeys: []
}

/**
 * useSettingStore defines a store for managing application settings.
 * It includes state for the list of settings and related operations.
 */
export const useSettingStore = defineStore('setting', () => {
  /**
   * settings is a key-value pair object
   */
  const settings = ref({ ...defaultSettings });

  /**
   * Submits configuration to the database and updates local configuration cache upon success.
   * @param {string} key - The setting key to update.
   * @param {any} value - The value to set for the specified key.
   * @returns {Promise<void>} A promise that resolves when the setting is successfully updated.
   */
  const setSetting = async (key, value) => {
    return new Promise(async (resolve, reject) => {
      // Convert camelCase to snake_case
      let dbKey = camelToSnake(key)

      // Update shortcut if the key is for a main or assistant window shortcut setting
      //
      // IMPORTANT:
      //  We must ensure the shortcut binding is successful before updating the database
      if (key === 'mainWindowVisibleShortcut' ||
        key === 'assistantWindowVisibleShortcut' ||
        key === 'noteWindowVisibleShortcut') {
        try {
          await invoke('update_shortcut', { key: dbKey, value })
        } catch (error) {
          console.error('Failed to update shortcut:', error)
          return reject(i18n.global.t('setting.general.updateShortcutFailed', { error }))
        }
      }

      invoke('set_config', { key: dbKey, value })
        .then(() => {
          settings.value = {
            ...settings.value,
            [key]: value
          }
          sendSyncState('setting_changed', windowLabel, { [key]: value })

          resolve()
        })
        .catch(reject)
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
      invoke('get_all_config')
        .then((result) => {
          // Update the entire object reactively
          settings.value = {
            ...defaultSettings,
          }
          if (!isEmpty(result)) {
            Object.keys(result).forEach(x => {
              settings.value[snakeToCamel(x)] = result[x]
            })
            console.debug('settings', settings.value)
          }
          resolve()
        })
        .catch(reject)
    })
  }

  const setTextMonitor = (start) => {
    return new Promise((resolve, reject) => {
      if (start) {
        invoke('start_text_monitor', { force: true })
          .then(() => {
            setSetting('wordSelectionToolbar', true).then(resolve)
          })
          .catch(err => {
            settings.value.wordSelectionToolbar = false
            invoke('open_text_selection_permission_settings')
            reject(i18n.global.t('settings.general.startWordSelectionToolbarFailed', { error: err }))
          })
      } else {
        invoke('stop_text_monitor')
          .then(() => {
            setSetting('wordSelectionToolbar', false).then(resolve)
          })
          .catch(err => {
            settings.value.wordSelectionToolbar = true
            reject(i18n.global.t('settings.general.stopWordSelectionToolbarFailed', { error: err }))
          })
      }
    })
  }

  const reloadConfig = () => {
    return new Promise((resolve, reject) => {
      invoke('reload_config')
        .then(() => {
          updateSettingStore().then(resolve)
        })
        .catch(reject)
    })
  }

  const updateTray = () => {
    return new Promise((resolve, reject) => {
      invoke('update_tray')
        .then(() => {
          resolve()
        })
        .catch(reject)
    })
  }

  return {
    windowLabel,
    settings,
    setSetting,
    updateSettingStore,
    setTextMonitor,
    reloadConfig,
    updateTray
  };
});
