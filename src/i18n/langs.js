import i18n from './index'

export const langs = {
  "ar": {
    "icon": "🇸🇦",
    "name": i18n.global.t('languages.ar')
  },
  "bg": {
    "icon": "🇧🇬",
    "name": i18n.global.t('languages.bg')
  },
  "bn": {
    "icon": "🇧🇩",
    "name": i18n.global.t('languages.bn')
  },
  "cs": {
    "icon": "🇨🇿",
    "name": i18n.global.t('languages.cs')
  },
  "da": {
    "icon": "🇩🇰",
    "name": i18n.global.t('languages.da')
  },
  "de": {
    "icon": "🇩🇪",
    "name": i18n.global.t('languages.de')
  },
  "el": {
    "icon": "🇬🇷",
    "name": i18n.global.t('languages.el')
  },
  "en": {
    "icon": "🇬🇧",
    "name": i18n.global.t('languages.en')
  },
  "es": {
    "icon": "🇪🇸",
    "name": i18n.global.t('languages.es')
  },
  "fi": {
    "icon": "🇫🇮",
    "name": i18n.global.t('languages.fi')
  },
  "fil": {
    "icon": "🇵🇭",
    "name": i18n.global.t('languages.fil')
  },
  "fr": {
    "icon": "🇫🇷",
    "name": i18n.global.t('languages.fr')
  },
  "hi": {
    "icon": "🇮🇳",
    "name": i18n.global.t('languages.hi')
  },
  "hr": {
    "icon": "🇭🇷",
    "name": i18n.global.t('languages.hr')
  },
  "hu": {
    "icon": "🇭🇺",
    "name": i18n.global.t('languages.hu')
  },
  "id": {
    "icon": "🇮🇩",
    "name": i18n.global.t('languages.id')
  },
  "is": {
    "icon": "🇮🇸",
    "name": i18n.global.t('languages.is')
  },
  "it": {
    "icon": "🇮🇹",
    "name": i18n.global.t('languages.it')
  },
  "ja": {
    "icon": "🇯🇵",
    "name": i18n.global.t('languages.ja')
  },
  "km": {
    "icon": "🇰🇭",
    "name": i18n.global.t('languages.km')
  },
  "ko": {
    "icon": "🇰🇷",
    "name": i18n.global.t('languages.ko')
  },
  "lo": {
    "icon": "🇱🇦",
    "name": i18n.global.t('languages.lo')
  },
  "mk": {
    "icon": "🇲🇰",
    "name": i18n.global.t('languages.mk')
  },
  "ms": {
    "icon": "🇲🇾",
    "name": i18n.global.t('languages.ms')
  },
  "my": {
    "icon": "🇲🇲",
    "name": i18n.global.t('languages.my')
  },
  "nl": {
    "icon": "🇳🇱",
    "name": i18n.global.t('languages.nl')
  },
  "no": {
    "icon": "🇳🇴",
    "name": i18n.global.t('languages.no')
  },
  "pt": {
    "icon": "🇵🇹",
    "name": i18n.global.t('languages.pt')
  },
  "pt-br": {
    "icon": "🇧🇷",
    "name": i18n.global.t('languages.pt-br')
  },
  "ro": {
    "icon": "🇷🇴",
    "name": i18n.global.t('languages.ro')
  },
  "ru": {
    "icon": "🇷🇺",
    "name": i18n.global.t('languages.ru')
  },
  "si": {
    "icon": "🇱🇰",
    "name": i18n.global.t('languages.si')
  },
  "sk": {
    "icon": "🇸🇰",
    "name": i18n.global.t('languages.sk')
  },
  "sr": {
    "icon": "🇷🇸",
    "name": i18n.global.t('languages.sr')
  },
  "sv": {
    "icon": "🇸🇪",
    "name": i18n.global.t('languages.sv')
  },
  "sw": {
    "icon": "🇰🇪",
    "name": i18n.global.t('languages.sw')
  },
  "th": {
    "icon": "🇹🇭",
    "name": i18n.global.t('languages.th')
  },
  "uk": {
    "icon": "🇺🇦",
    "name": i18n.global.t('languages.uk')
  },
  "vi": {
    "icon": "🇻🇳",
    "name": i18n.global.t('languages.vi')
  },
  "zh-Hans": {
    "icon": "🇨🇳",
    "name": i18n.global.t('languages.zh-Hans')
  },
  "zh-Hant": {
    "icon": "🇨🇳",
    "name": i18n.global.t('languages.zh-Hant')
  }
}

/**
 * Returns a list of available software languages
 * @returns {Object} Language configuration object
 */
export const softwareLanguages = () => {
  const available = ['de', 'en', 'es', 'fr', 'ja', 'ko', 'pt', 'ru', 'zh-Hans', 'zh-Hant']
  return available.reduce((acc, lang) => {
    acc[lang] = { code: lang, ...langs[lang] }
    return acc
  }, {})
}
