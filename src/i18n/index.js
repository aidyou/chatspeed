import { createI18n } from 'vue-i18n'
import common from './do_not_edit/copy_from_rust_src_i18n.json'
import de from './locales/de.json'
import en from './locales/en.json'
import fr from './locales/fr.json'
import ja from './locales/ja.json'
import zhHans from './locales/zh-Hans.json'
import zhHant from './locales/zh-Hant.json'
import ko from './locales/ko.json'
import es from './locales/es.json'
import ru from './locales/ru.json'
import pt from './locales/pt.json'

const savedLocale = localStorage.getItem('locale') || 'en'
const i18n = createI18n({
  legacy: false,
  locale: savedLocale,
  fallbackLocale: 'en',
  messages: {
    de: { ...common, ...de },
    en: { ...common, ...en },
    fr: { ...common, ...fr },
    ja: { ...common, ...ja },
    ko: { ...common, ...ko },
    es: { ...common, ...es },
    ru: { ...common, ...ru },
    pt: { ...common, ...pt },
    'zh-Hans': { ...common, ...zhHans },
    'zh-Hant': { ...common, ...zhHant }
  }
})

export function setI18nLanguage(locale) {
  i18n.global.locale.value = locale
  localStorage.setItem('locale', locale)
  document.querySelector('html').setAttribute('lang', locale)
}

export default i18n
export { common as languageConfig }
