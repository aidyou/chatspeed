import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { createPinia, defineStore, setActivePinia } from 'pinia'
import { ref } from 'vue'
import { parseCapabilityError } from '../../libs/capability.js'

const component = readFileSync(new URL('./AgentSkills.vue', import.meta.url), 'utf8')
const general = readFileSync(new URL('./General.vue', import.meta.url), 'utf8')
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
  assert.match(general, /v-model="generalTab"/)
  assert.match(general, /name="agentSkills"/)
  assert.match(general, /<agent-skills \/>/)
  assert.doesNotMatch(settings, /id: 'agentSkills'/)
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

test('plugin lifecycle stays behind the runtime forwarding surface', () => {
  const pluginStore = readFileSync(new URL('../../stores/plugin.js', import.meta.url), 'utf8')
  assert.match(component, /usePluginStore/)
  for (const command of ['plugin_inventory', 'plugin_load', 'plugin_disable', 'plugin_uninstall']) {
    assert.match(pluginStore, new RegExp(`'${command}'`))
  }
  assert.doesNotMatch(pluginStore, /agent_skills_plugin_|readFile|writeFile|fetch\(/)
  assert.match(component, /pluginStore\.loadInventory\(\)/)
})

test('source documents use the strict backend SkillSource DTO', () => {
  assert.match(component, /value="local_zip"/)
  assert.match(component, /kind: 'github'/)
  assert.match(component, /owner: repositoryParts\.value\[0\]/)
  assert.match(component, /repo: repositoryParts\.value\[1\]/)
  assert.match(component, /git_ref: githubRef\.value\.trim\(\) \|\| undefined/)
  assert.doesNotMatch(component, /kind: 'zip'/)
  assert.doesNotMatch(component, /repository: githubRepository/)
  assert.doesNotMatch(component, /reference: githubRef/)
})
test('plugin store executes only runtime forwarding commands and reconciles responses', async () => {
  const source = readFileSync(new URL('../../stores/plugin.js', import.meta.url), 'utf8')
  assert.match(source, /import \{ invoke \} from '@tauri-apps\/api\/core'/)
  setActivePinia(createPinia())
  const calls = []
  let failure = null
  const useStore = runInNewContext(
    source.replace(/^import .*\n/gm, '').replace('export const usePluginStore', 'const usePluginStore') + '\nusePluginStore',
    {
      defineStore, ref, parseCapabilityError,
      invoke: async command => {
        calls.push(command)
        if (failure) throw failure
        return { installed: command !== 'plugin_uninstall', enabled: command === 'plugin_load' }
      }
    }
  )
  const store = useStore()
  await store.loadInventory()
  await store.install()
  assert.equal(store.inventory.enabled, true)
  await store.disable()
  assert.equal(store.inventory.enabled, false)
  await store.uninstall()
  assert.equal(store.inventory.installed, false)
  assert.deepEqual(calls, ['plugin_inventory', 'plugin_load', 'plugin_disable', 'plugin_uninstall'])
  failure = JSON.stringify({ code: 'unavailable', message: 'runtime unavailable' })
  await assert.rejects(store.install(), error => error.code === 'unavailable')
  assert.equal(store.lastError, 'runtime unavailable')
  assert.equal(store.applying, false)
  await assert.rejects(store.loadInventory(), error => error.code === 'unavailable')
  assert.equal(store.loading, false)
  assert.match(component, /v-if="pluginStore.inventory" class="buttons"/)
})

test('source adapter generates canonical JSON for directory, ZIP and GitHub', () => {
  const start = component.indexOf('const sourceDocument = computed(')
  const end = component.indexOf('\n\n// A changed source', start)
  assert.ok(start > -1 && end > start)
  const sourceKind = { value: 'local_directory' }
  const sourcePath = { value: ' /fixture/demo ' }
  const repositoryParts = { value: ['owner', 'repo'] }
  const githubPath = { value: ' skills/demo ' }
  const githubRef = { value: ' main ' }
  const source = runInNewContext(component.slice(start, end) + '\nsourceDocument', {
    computed: getter => ({ get value() { return getter() } }),
    sourceKind, sourcePath, repositoryParts, githubPath, githubRef
  })
  const json = () => JSON.parse(JSON.stringify(source.value))
  assert.deepEqual(json(), { kind: 'local_directory', path: '/fixture/demo' })
  sourceKind.value = 'local_zip'
  sourcePath.value = ' /fixture/demo.zip '
  assert.deepEqual(json(), { kind: 'local_zip', path: '/fixture/demo.zip' })
  sourceKind.value = 'github'
  assert.deepEqual(json(), { kind: 'github', owner: 'owner', repo: 'repo', git_ref: 'main', path: 'skills/demo' })
  githubPath.value = ''
  githubRef.value = ''
  assert.deepEqual(json(), { kind: 'github', owner: 'owner', repo: 'repo' })
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
