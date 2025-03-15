// Importing the i18n instance for internationalization support.
import i18n from '@/i18n'

/**
 * Retrieve the translated API types.
 * It returns an object where each key is an API type and its value is the corresponding translated string.
 */
export function apiProtocol() {
  return Object.fromEntries(
    Object.entries(allowApiProvider).map(([key, value]) => [key, i18n.global.t(value)]) // Translate each API provider using i18n
  )
}

// Key config for local storage
const csStorageKey = {
  defaultModel: 'defaultModel', // Default model configuration stored in local storage
  chatSidebarShow: 'chatSidebarShow', // Chat sidebar show stored in local storage
  currentConversationId: 'currentConversationId', // Current conversation ID stored in local storage
  defaultModelIdAtDialog: 'defaultModelIdAtDialog', // Default model ID at dialog stored in local storage
  defaultModelAtDialog: 'defaultModelAtDialog', // Default model at dialog stored in local storage
  noteSidebarWidth: 'noteSidebarWidth',
  noteSidebarCollapsed: 'noteSidebarCollapsed',
  ignoreVersion: 'ignoreVersion',
  updateLater: 'updateLater',
  networkEnabled: 'networkEnabled',
  assistNetworkEnabled: 'assistNetworkEnabled',
}
Object.freeze(csStorageKey)

export { csStorageKey }
