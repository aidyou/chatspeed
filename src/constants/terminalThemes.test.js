import assert from 'node:assert/strict'
import test from 'node:test'

import { DEFAULT_TERMINAL_SKIN, TERMINAL_SKINS, terminalSkinPalette } from './terminalThemes.js'

const CANVAS_KEYS = [
  'background',
  'foreground',
  'cursor',
  'selectionBackground',
  'selectionForeground'
]
const ANSI_KEYS = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite'
]

const colorSkins = TERMINAL_SKINS.filter(skin => skin.palettes)

const luminance = hex => {
  const channels = [1, 3, 5].map(index => parseInt(hex.slice(index, index + 2), 16) / 255)
  const [r, g, b] = channels.map(value =>
    value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  )
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

test('every terminal skin ships a complete dark and light palette', () => {
  assert.equal(colorSkins.length, 3)
  assert.equal(TERMINAL_SKINS.length, colorSkins.length + 1)

  for (const skin of colorSkins) {
    assert.deepEqual(Object.keys(skin.palettes).sort(), ['dark', 'light'])
    for (const variant of ['dark', 'light']) {
      const palette = skin.palettes[variant]
      for (const key of [...CANVAS_KEYS, ...ANSI_KEYS]) {
        assert.match(palette[key] ?? '', /^#[0-9a-f]{6}$/, `${skin.id} ${variant} ${key}`)
      }
    }
  }
})

test('a light palette stays lighter than its dark counterpart', () => {
  for (const skin of colorSkins) {
    assert.ok(
      luminance(skin.palettes.light.background) > luminance(skin.palettes.dark.background),
      `${skin.id} light background must be lighter than its dark background`
    )
    assert.ok(
      luminance(skin.palettes.light.foreground) < luminance(skin.palettes.dark.foreground),
      `${skin.id} light foreground must be darker than its dark foreground`
    )
  }
})

test('the default skin keeps the application terminal tokens', () => {
  assert.equal(TERMINAL_SKINS[0].id, DEFAULT_TERMINAL_SKIN)
  assert.equal(TERMINAL_SKINS[0].palettes, undefined)
  assert.equal(terminalSkinPalette(DEFAULT_TERMINAL_SKIN, true), undefined)
  assert.equal(terminalSkinPalette(undefined, true), undefined)
  assert.equal(terminalSkinPalette('not-a-skin', false), undefined)
})

test('a skin resolves to the palette matching the resolved scheme', () => {
  for (const skin of colorSkins) {
    assert.equal(terminalSkinPalette(skin.id, true), skin.palettes.dark)
    assert.equal(terminalSkinPalette(skin.id, false), skin.palettes.light)
  }
})

test('every skin names its upstream palette and its locale label', () => {
  for (const skin of colorSkins) {
    assert.match(skin.labelKey, /^settings\.general\.terminalSkin/)
    assert.ok(skin.source.length > 0)
  }
})