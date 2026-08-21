import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_EXECUTION_STYLE,
  EXECUTION_STYLE_PRESETS,
  getExecutionStyleOptions,
  resolveExecutionStylePreference
} from './executionStyle.js'

test('execution style options expose only the selected Agent custom style', () => {
  const agent = { personality: 'Be calm and decisive.' }
  const values = getExecutionStyleOptions(agent).map(option => option.value)

  assert.equal(values[0], DEFAULT_EXECUTION_STYLE)
  assert.deepEqual(
    values.slice(1, 1 + EXECUTION_STYLE_PRESETS.length),
    EXECUTION_STYLE_PRESETS.map(option => option.value)
  )
  assert.equal(values.at(-1), agent.personality)
})

test('stale custom execution styles fall back to the built-in default', () => {
  assert.equal(
    resolveExecutionStylePreference('No longer available', { personality: 'Current style' }),
    DEFAULT_EXECUTION_STYLE
  )
  assert.equal(
    resolveExecutionStylePreference('preset:researcher', { personality: 'Current style' }),
    'preset:researcher'
  )
})
