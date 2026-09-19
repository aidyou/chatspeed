import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const component = readFileSync(new URL('./AgentSkills.vue', import.meta.url), 'utf8')
const settings = readFileSync(new URL('../../views/Settings.vue', import.meta.url), 'utf8')

const locales = ['en', 'zh-Hans', 'zh-Hant'].map(name => ({
  name,
  messages: JSON.parse(
    readFileSync(new URL(`../../i18n/locales/${name}.json`, import.meta.url), 'utf8')
  )
}))

test('the Agent Skills page is separate from the prompt Skill page', () => {
  // File-based Agent Skills never share the database prompt skill store.
  assert.doesNotMatch(component, /useSkillStore|stores\/skill/)
  assert.match(component, /import { useCapabilityStore } from '@\/stores\/capability'/)
  // The prompt page keeps its own menu entry, and the new page is additive.
  assert.match(settings, /id: 'skill'/)
  assert.match(settings, /id: 'agentSkills'/)
  assert.match(settings, /<agent-skills \/>/)
})

test('check, install and uninstall all go through the capability service', () => {
  assert.match(component, /store\.checkSource/)
  assert.match(component, /store\.install\(/)
  assert.match(component, /store\.uninstall\(/)
  assert.match(component, /store\.loadInventory\(\)/)
  assert.match(component, /store\.loadDoctor\(\)/)
  // Reconcile is the one mutation the page offers, and it too goes through the
  // single capability service so the desktop and CLI converge the same facts.
  assert.match(component, /store\.reconcile\(\)/)
  // The install button is gated on the checker verdict, never on a UI toggle.
  assert.match(component, /verdictAllowsInstall\(report\.value\)/)
  // A source change drops the previous verdict instead of reusing it.
  assert.match(component, /store\.resetCheck\(\)/)
})

test('the page surfaces every refusal state instead of hiding it', () => {
  for (const status of ['skipped_existing', 'unsupported', 'blocked', 'refused', 'failed']) {
    assert.match(component, new RegExp(`'${status}'`), `missing ${status} handling`)
  }
  // Findings and permissions from the check report are rendered.
  assert.match(component, /report\.findings/)
  assert.match(component, /report\.permissions/)
  assert.match(component, /report\.checker_version/)
  // An uninstall is only offered when the backend says it is permitted.
  assert.match(component, /:disabled="!row\.uninstallable"/)
})

test('user-visible copy comes from i18n and never from a literal', () => {
  assert.doesNotMatch(component, /[\u4e00-\u9fff]/, 'hardcoded CJK copy in the component')
  const keys = [...component.matchAll(/t\('(settings\.[A-Za-z0-9_.]+)'/g)].map(match => match[1])
  assert.ok(keys.length > 10, 'the page should be fully localized')
  for (const key of keys) {
    assert.match(key, /^settings\.(agentSkills|type)\./)
  }
})

/** Flattens a message subtree into `a.b` leaf paths. */
function flatten(messages, prefix = '') {
  const entries = []
  for (const [key, value] of Object.entries(messages)) {
    const path = prefix ? `${prefix}.${key}` : key
    if (value && typeof value === 'object') {
      entries.push(...flatten(value, path))
    } else {
      entries.push([path, value])
    }
  }
  return entries
}

test('every locale defines the same Agent Skills keys', () => {
  const reference = flatten(locales[0].messages.settings.agentSkills).map(entry => entry[0])
  assert.ok(reference.length > 30)
  assert.deepEqual(reference, [...reference].sort(), 'reference keys are not sorted')

  for (const locale of locales) {
    const entries = flatten(locale.messages.settings.agentSkills)
    assert.deepEqual(
      entries.map(entry => entry[0]),
      reference,
      `${locale.name} has a different Agent Skills surface`
    )
    for (const [key, value] of entries) {
      assert.equal(typeof value, 'string', `${locale.name}.${key} must be a string`)
      assert.notEqual(value.trim(), '', `${locale.name}.${key} must not be empty`)
    }
    // Keys are kept sorted, as the locale files require.
    const keys = Object.keys(locale.messages.settings.agentSkills)
    assert.deepEqual(keys, [...keys].sort(), `${locale.name} keys are not sorted`)
    assert.equal(
      typeof locale.messages.settings.type.agentSkills,
      'string',
      `${locale.name} is missing the menu label`
    )
  }
})

test('every permission the checker can report has a label', () => {
  const reported = [
    'reads_files',
    'executes_processes',
    'uses_network',
    'downloads_content',
    'uses_dynamic_eval',
    'reads_credentials',
    'writes_sensitive_paths',
    'requires_elevation',
    'contains_binary_content'
  ]
  for (const locale of locales) {
    const permissions = locale.messages.settings.agentSkills.permission
    assert.deepEqual(
      Object.keys(permissions).sort(),
      [...reported].sort(),
      `${locale.name} is missing permission labels`
    )
  }
})
