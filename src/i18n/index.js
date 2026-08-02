import { createI18n } from 'vue-i18n'
import common from './do_not_edit/copy_from_rust_src_i18n.json'
import en from './locales/en.json'
import ja from './locales/ja.json'
import zhHans from './locales/zh-Hans.json'
import zhHant from './locales/zh-Hant.json'
import { normalizeInterfaceLocale } from './supportedLocales'

const savedLocale = normalizeInterfaceLocale(localStorage.getItem('locale'))
localStorage.setItem('locale', savedLocale)
const i18n = createI18n({
  legacy: false,
  locale: savedLocale,
  fallbackLocale: 'en',
  messages: {
    en: { ...common, ...en },
    ja: { ...common, ...ja },
    'zh-Hans': { ...common, ...zhHans },
    'zh-Hant': { ...common, ...zhHant }
  }
})

export function setI18nLanguage(locale) {
  const normalizedLocale = normalizeInterfaceLocale(locale)
  i18n.global.locale.value = normalizedLocale
  localStorage.setItem('locale', normalizedLocale)
  document.querySelector('html').setAttribute('lang', normalizedLocale)
}

export default i18n
export { common as languageConfig }
