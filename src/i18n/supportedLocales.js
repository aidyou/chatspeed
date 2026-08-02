export const SUPPORTED_INTERFACE_LOCALES = ['en', 'ja', 'zh-Hans', 'zh-Hant']

export function normalizeInterfaceLocale(locale) {
  return SUPPORTED_INTERFACE_LOCALES.includes(locale) ? locale : 'en'
}
