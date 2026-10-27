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

  for (const locale of ['en', 'zh-Hans', 'zh-Hant']) {
    const json = JSON.parse(read(join(projectRoot, `src/i18n/locales/${locale}.json`)))
    assert.deepEqual(Object.keys(json.settings.general.primaryColors), Object.keys(colors).sort())
  }
})

test('spacing and radii share a four-point scale with complete compatibility aliases', () => {
  const css = read(join(projectRoot, 'src/style/element/css-vars.css'))
  const tokens = new Map([...css.matchAll(/(--cs-(?:space|border-radius)[\w-]*)\s*:\s*([^;]+);/g)]
    .map(([, name, value]) => [name, value.trim()]))
  const resolve = (name, seen = new Set()) => {
    assert.ok(tokens.has(name), `Undefined token: ${name}`)
    assert.ok(!seen.has(name), `Cyclic token: ${name}`)
    seen.add(name)
    const value = tokens.get(name)
    const alias = /^var\((--cs-[\w-]+)\)$/.exec(value)
    return alias ? resolve(alias[1], seen) : value
  }
  for (const name of tokens.keys()) {
    const value = resolve(name)
    if (name === '--cs-border-radius-round') {
      assert.equal(value, '50%')
      continue
    }
    if (name === '--cs-border-radius-pill') {
      assert.equal(value, '9999px')
      continue
    }
    assert.match(value, /^\d+px$/)
    const pixels = Number.parseInt(value, 10)
    const micro = ['--cs-space-xxs', '--cs-space-2xs', '--cs-space-xss'].includes(name)
    assert.equal(micro ? pixels : pixels % 4, micro ? 2 : 0, name)
  }
  assert.equal(resolve('--cs-space-xl'), '40px')
  assert.equal(resolve('--cs-border-radius-xs'), '4px')
  assert.equal(resolve('--cs-border-radius-base'), '8px')

  const walk = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? walk(path) : [path]
  })
  for (const path of walk(join(projectRoot, 'src')).filter(path => /\.(vue|scss|css|js|ts)$/.test(path))) {
    for (const [, name] of read(path).matchAll(/var\((--cs-(?:space|border-radius)[\w-]*)/g)) {
      assert.ok(tokens.has(name), `${path}: undefined ${name}`)
    }
  }
})

test('layout literals stay on the grid except documented geometry and export details', () => {
  const exceptions = new Map([
    ['src/components/setting/Agent.vue|padding-left|26px', 1],
    ['src/components/workflow/TreeNode.vue|margin-left|21px', 1],
    ['src/components/workflow/StatusPanel.vue|border-radius|2px', 1],
    ['src/components/workflow/TaskLedger.vue|border-radius|2px', 1],
    ['src/views/ProxySwitcher.vue|border-radius|2px', 2],
    ['src/libs/directive.js|border-radius|2px', 1]
  ])
  const walk = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? walk(path) : [path]
  })
  const declarations = /\b(padding(?:-[\w-]+)?|margin(?:-[\w-]+)?|(?:row-|column-)?gap|border(?:-(?:top-left|top-right|bottom-left|bottom-right))?-radius|borderRadius)\s*:\s*([^;{}\n]+)/g
  const actualExceptions = new Map()
  for (const path of walk(join(projectRoot, 'src'))) {
    if (!/\.(vue|scss|css|js|ts)$/.test(path) || /\.test\./.test(path) || path.endsWith('css-vars.css')) continue
    const relative = path.slice(projectRoot.length + 1)
    const source = read(path).replace(/\/\*[\s\S]*?\*\//g, '')
    for (const [, property, declaration] of source.matchAll(declarations)) {
      const value = declaration.replace(/var\([^)]*\)/g, '')
      for (const [, literal] of value.matchAll(/(?<![\w.])(-?\d+(?:\.\d+)?)px/g)) {
        const pixels = Number(literal)
        if (pixels % 4 === 0 || (!/radius/i.test(property) && Math.abs(pixels) <= 2)) continue
        const key = [relative, property, literal + 'px'].join('|')
        assert.ok(exceptions.has(key), key)
        actualExceptions.set(key, (actualExceptions.get(key) || 0) + 1)
      }
    }
  }
  assert.deepEqual(actualExceptions, exceptions)
})

test('authored frontend color literals are centralized in css-vars.css', () => {
  const allowed = new Set([
    'src/style/element/css-vars.css',
    'src/tool/ic.js',
    'src/libs/chat.js',
    // Third-party terminal palettes copied verbatim from the upstream themes they name.
    'src/constants/terminalThemes.js'
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
