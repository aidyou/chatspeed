import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

test('chat hub page reuses the settings list pattern with sortable rows', () => {
  const source = read('./ChatHub.vue')

  assert.match(source, /<span>\{\{ \$t\('settings\.chatHub\.title'\) \}\}<\/span>/)
  assert.match(source, /<span class="icon" @click="openChatHubDialog\(\)">/)
  assert.match(source, /<Sortable v-if="chatHubStore\.list\.length > 0"[\s\S]*?:list="chatHubStore\.list"/)
  assert.match(source, /@update="onChatHubSortUpdate" @end="onChatHubDragEnd"/)
  assert.match(source, /onChatHubDragEnd[\s\S]*?chatHubStore\.reorder\(chatHubStore\.list\.map\(hub => hub\.id\)\)/)
  assert.match(source, /<img v-if="chatHubLogo\(element\)"[\s\S]*?@error="onChatHubLogoError\(element\)"/)
  assert.match(source, /<avatar v-else :text="element\.name" :size="20" \/>/)
  assert.match(source, /const chatHubLogo = hub => \(hub\.logo && !chatHubBrokenLogos\.value\.has\(hub\.id\) \? hub\.logo : ''\)/)
  // Entries live in their own table through the dedicated store, not in settings.
  assert.match(source, /import \{ useChatHubStore \} from '@\/stores\/chatHub'/)
  assert.doesNotMatch(source, /settingStore|settings\.value/)
})

test('chat hub editor validates http(s) urls and only fills the favicon url', () => {
  const source = read('./ChatHub.vue')

  assert.match(source, /const isValidChatHubUrl = value => \{[\s\S]*?parsed\.protocol === 'http:' \|\| parsed\.protocol === 'https:'/)
  assert.match(source, /if \(!name\) \{[\s\S]*?chatHub\.nameRequired/)
  assert.match(source, /if \(!isValidChatHubUrl\(url\)\) \{[\s\S]*?chatHub\.urlInvalid/)
  assert.match(source, /if \(logo && !isValidChatHubUrl\(logo\)\) \{[\s\S]*?chatHub\.logoInvalid/)
  assert.match(
    source,
    /const fillChatHubFavicon = \(\) => \{[\s\S]*?if \(!isValidChatHubUrl\(chatHubForm\.url\)\) \{[\s\S]*?chatHub\.faviconInvalidUrl[\s\S]*?return[\s\S]*?chatHubForm\.logo = `https:\/\/www\.google\.com\/s2\/favicons\?sz=64&domain_url=\$\{encodeURIComponent\(/
  )
  // The favicon helper must only build a url, never download or parse an image.
  assert.doesNotMatch(source, /fetch\(|createImageBitmap|new Image\(/)
  assert.match(source, /await ElMessageBox\.confirm\([\s\S]*?chatHub\.deleteConfirm[\s\S]*?chatHub\.deleteConfirmTitle/)
  assert.match(source, /const deleteChatHub = async hub => \{/)
  assert.match(source, /chatHubBrokenLogos\.value\.delete\(chatHubForm\.id\)/)
})

test('the settings window exposes a dedicated chat hub tab right after the AI provider tab', () => {
  const settings = read('../../views/Settings.vue')

  assert.match(settings, /import chatHub from '@\/components\/setting\/ChatHub\.vue'/)
  assert.match(
    settings,
    /\{ label: t\('settings\.type\.model'\), icon: 'model', id: 'model' \},\s*\{ label: t\('settings\.type\.chatHub'\), icon: 'talk', id: 'chatHub' \},/
  )
  assert.match(
    settings,
    /<el-main v-show="settingType === 'chatHub'" class="main">\s*<chatHub \/>\s*<\/el-main>/
  )
  // The management UI is a page of its own, not a card inside the general tab.
  assert.doesNotMatch(read('./General.vue'), /settings\.general\.chatHub|chatHubStore/)
})

test('chat hub settings text exists in every locale with the same key set', () => {
  const locales = ['en', 'zh-Hans', 'zh-Hant'].map(locale =>
    JSON.parse(read(`../../i18n/locales/${locale}.json`))
  )

  const keySets = locales.map(locale => Object.keys(locale.settings.chatHub).sort())
  assert.ok(keySets[0].length > 0)
  for (const keySet of keySets) {
    assert.deepEqual(keySet, keySets[0])
  }
  for (const locale of locales) {
    assert.equal(typeof locale.settings.type.chatHub, 'string')
    assert.equal(typeof locale.settings.general.configCategories.chatHubs, 'string')
    assert.equal(locale.settings.general.chatHub, undefined)
  }
})