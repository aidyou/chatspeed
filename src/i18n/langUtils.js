import { langs, softwareLanguages } from './langs'

// 映射函数
export function mapBrowserLangToStandard(browserLang) {
  const lowerBrowserLang = browserLang.toLowerCase()

  // 特殊情况处理
  if (lowerBrowserLang.startsWith('zh')) {
    return lowerBrowserLang.includes('hant') ? 'zh-Hant' : 'zh-Hans'
  }

  // 常规匹配
  for (const langCode in langs) {
    if (lowerBrowserLang.startsWith(langCode)) {
      return langCode
    }
  }

  // 如果没有匹配，返回默认语言（例如英语）
  return 'en'
}

/**
 * Get all available languages
 * @returns {Object}
 */
export function getAvailableLanguages() {
  return Object.keys(langs).map(langCode => ({
    code: langCode,
    ...langs[langCode]
  }))
}

/**
 * Get language by language code: en -> english
 * @param {string} code
 * @returns {Object}
 */
export function getLanguageByCode(code) {
  return langs[code]?.name
}

/**
 * Get all available software languages
 * @returns {Object}
 */
export function getSoftwareLanguages() {
  console.log(softwareLanguages())
  return softwareLanguages()
}
