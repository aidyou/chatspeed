// Importing the i18n instance for internationalization support.
import i18n from '@/i18n'

// This section configures the AI interface types supported by the system up to this point.
//
// The values in allowApiProvider correspond to entries in the translation file.
// This object maps API providers to their respective translation keys.
const allowApiProvider = {
  openai: 'settings.apiProvider.openai',
  gemini: 'settings.apiProvider.gemini',
  claude: 'settings.apiProvider.claude',
  ollama: 'settings.apiProvider.ollama',
  // huggingface: 'settings.apiProvider.huggingface',
}

/**
 * Retrieve the translated API types.
 * It returns an object where each key is an API type and its value is the corresponding translated string.
 */
export function apiProvider() {
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
}
Object.freeze(csStorageKey)

export { csStorageKey }
