import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import test from 'node:test'

const read = path => readFileSync(path, 'utf8')
const projectRoot = process.cwd()
const colors = {
  green: '65, 181, 85',
  purple: '175, 82, 222',
  yellow: '247, 131, 28',
  pink: '255, 45, 85',
  blue: '0, 122, 255'
}

test('primary presets, setting flow, and locale labels remain aligned', () => {
  const css = read(join(projectRoot, 'src/style/element/css-vars.css'))
  const store = read(join(projectRoot, 'src/stores/setting.js'))
  const app = read(join(projectRoot, 'src/App.vue'))
  const general = read(join(projectRoot, 'src/components/setting/General.vue'))

  assert.match(store, /primaryColor: 'green'/)
  assert.match(app, /const PRIMARY_COLORS = new Set\(\['green', 'purple', 'yellow', 'pink', 'blue'\]\)/)
  assert.match(app, /dataset\.primaryColor = PRIMARY_COLORS\.has\(value\) \? value : 'green'/)
  assert.match(general, /v-model="settings\.primaryColor"/)
  assert.match(general, /setSetting\('primaryColor', value \|\| 'green'\)/)

  for (const [color, rgb] of Object.entries(colors)) {
    assert.match(css, new RegExp(`:root\\[data-primary-color='${color}'\\]\\s*\\{\\s*--cs-color-primary-rgb: ${rgb};`))
  }

  for (const locale of ['en', 'ja', 'zh-Hans', 'zh-Hant']) {
    const json = JSON.parse(read(join(projectRoot, `src/i18n/locales/${locale}.json`)))
    assert.deepEqual(Object.keys(json.settings.general.primaryColors), Object.keys(colors).sort())
  }
})

test('authored frontend color literals are centralized in css-vars.css', () => {
  const allowed = new Set([
    'src/style/element/css-vars.css',
    'src/tool/ic.js',
    'src/libs/chat.js'
  ])
  const literal = /#[0-9a-fA-F]{3,8}\b|\brgba?\(/
  const walk = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? walk(path) : [path]
  })
  const offenders = walk(join(projectRoot, 'src'))
    .filter(path => /\.(vue|scss|css|js|ts)$/.test(path))
    .filter(path => !allowed.has(path.replace(`${projectRoot}/`, '')))
    .filter(path => literal.test(read(path)))
  assert.deepEqual(offenders, [])
})
