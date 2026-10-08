import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { createPinia, defineStore, setActivePinia } from 'pinia'
import { ref, computed } from 'vue'
import { compileStyleAsync, compileTemplate, parse as parseSfc } from 'vue/compiler-sfc'
import { parseCapabilityError } from '../../libs/capability.js'

const component = readFileSync(new URL('./AgentSkills.vue', import.meta.url), 'utf8')
const plugin = readFileSync(new URL('./Plugin.vue', import.meta.url), 'utf8')
const general = readFileSync(new URL('./General.vue', import.meta.url), 'utf8')
const settings = readFileSync(new URL('../../views/Settings.vue', import.meta.url), 'utf8')

const locales = ['en', 'zh-Hans', 'zh-Hant'].map(name => ({
  name,
  messages: JSON.parse(
    readFileSync(new URL(`../../i18n/locales/${name}.json`, import.meta.url), 'utf8')
  )
}))

const pluginStoreSource = readFileSync(new URL('../../stores/plugin.js', import.meta.url), 'utf8')

/**
 * Loads the store module in an isolated context against a mock `invoke`, so the
 * forwarding contract can be exercised without Tauri or a live runtime.
 */
function loadPluginStore(invoke) {
  return runInNewContext(
    pluginStoreSource.replace(/^import .*\n/gm, '').replace('export const usePluginStore', 'const usePluginStore') + '\nusePluginStore',
    { defineStore, ref, computed, parseCapabilityError, invoke }
  )
}

test('the Agent Skills page is separate from the prompt Skill page', () => {
  // File-based Agent Skills never share the database prompt skill store.
  assert.doesNotMatch(component, /useSkillStore|stores\/skill/)
  assert.match(component, /import { useCapabilityStore } from '@\/stores\/capability'/)
  // The prompt page keeps its own menu entry, and the new page is additive.
  assert.match(settings, /id: 'skill'/)
  assert.match(general, /v-model="generalTab"/)
  assert.doesNotMatch(general, /name="agentSkills"|<agent-skills \/>/)
  assert.match(settings, /id: 'plugin'/)
  assert.match(settings, /<plugin \/>/)
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
  assert.doesNotMatch(component, /usePluginStore|pluginStore/)
  assert.match(plugin, /usePluginStore/)
  for (const command of ['plugin_inventory', 'plugin_load', 'plugin_disable', 'plugin_uninstall']) {
    assert.match(pluginStore, new RegExp(`'${command}'`))
  }
  assert.doesNotMatch(pluginStore, /agent_skills_plugin_|readFile|writeFile|fetch\(/)
  assert.match(plugin, /pluginStore\.loadInventory\(\)/)
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
  assert.match(pluginStoreSource, /import \{ invoke \} from '@tauri-apps\/api\/core'/)
  setActivePinia(createPinia())
  const calls = []
  let failure = null
  const useStore = loadPluginStore(async command => {
    calls.push(command)
    if (failure) throw failure
    const state = command === 'plugin_uninstall'
      ? 'not_installed'
      : command === 'plugin_disable'
        ? 'disabled'
        : 'enabled'
    return { schema_version: 1, plugins: [{ id: 'agent-skills', kind: 'builtin', state }] }
  })
  const store = useStore()
  await store.loadInventory()
  await store.install()
  assert.equal(store.plugins[0].state, 'enabled')
  await store.disable()
  assert.equal(store.plugins[0].state, 'disabled')
  await store.uninstall()
  assert.equal(store.plugins[0].state, 'not_installed')
  assert.deepEqual(calls, ['plugin_inventory', 'plugin_load', 'plugin_disable', 'plugin_uninstall'])
  failure = JSON.stringify({ code: 'unavailable', message: 'runtime unavailable' })
  await assert.rejects(store.install(), error => error.code === 'unavailable')
  assert.equal(store.lastError, 'runtime unavailable')
  assert.equal(store.applying, false)
  // A failed mutation keeps the last authoritative snapshot instead of guessing.
  assert.equal(store.plugins[0].state, 'not_installed')
  await assert.rejects(store.loadInventory(), error => error.code === 'unavailable')
  assert.equal(store.loading, false)
  // A read the runtime could not answer is not a valid snapshot: fail closed.
  assert.equal(store.inventory, null)
  assert.doesNotMatch(component, /v-if="pluginStore.inventory" class="buttons"/)
  assert.match(plugin, /v-for="plugin in pluginStore.builtinPlugins"/)
})

test('verified UI plugins require enabled and verified, and hide on any failure', async () => {
  setActivePinia(createPinia())
  let failure = null
  const store = loadPluginStore(async () => {
    if (failure) throw failure
    return {
      schema_version: 1,
      plugins: [
        { id: 'ready', kind: 'builtin', state: 'enabled', ui: { entry: 'a', verified: true } },
        { id: 'off', kind: 'builtin', state: 'disabled', ui: { entry: 'b', verified: true } },
        { id: 'unverified', kind: 'builtin', state: 'enabled', ui: { entry: 'c', verified: false } },
        { id: 'external', kind: 'external', state: 'enabled', ui: { entry: 'd', verified: true } }
      ]
    }
  })()
  await store.loadInventory()
  assert.deepEqual(store.verifiedUiPlugins.map(plugin => plugin.id), ['ready'])

  // A failed mutation keeps the snapshot but hides the UI behind lastError.
  failure = JSON.stringify({ code: 'unavailable', message: 'runtime unavailable' })
  await assert.rejects(store.disable(), error => error.code === 'unavailable')
  assert.equal(store.plugins.length, 4, 'the last snapshot is retained')
  assert.equal(store.verifiedUiPlugins.length, 0, 'a failed mutation hides the surface')

  // A later successful read clears the error and restores the verified list.
  failure = null
  await store.loadInventory()
  assert.equal(store.lastError, null)
  assert.deepEqual(store.verifiedUiPlugins.map(plugin => plugin.id), ['ready'])
})

test('a slow refresh cannot overwrite a newer mutation snapshot', async () => {
  setActivePinia(createPinia())
  const pending = []
  const store = loadPluginStore(command => new Promise((resolve, reject) => {
    pending.push({ command, resolve, reject })
  }))()

  const refresh = store.loadInventory()
  const mutation = store.install()
  assert.deepEqual(pending.map(entry => entry.command), ['plugin_inventory', 'plugin_load'])

  pending[1].resolve({
    schema_version: 1,
    plugins: [{ id: 'agent-skills', kind: 'builtin', state: 'enabled', ui: { verified: true } }]
  })
  await mutation
  assert.equal(store.plugins[0].state, 'enabled')

  // The earlier refresh resolves last with a stale disabled snapshot.
  pending[0].resolve({
    schema_version: 1,
    plugins: [{ id: 'agent-skills', kind: 'builtin', state: 'disabled', ui: { verified: true } }]
  })
  const settled = await refresh
  assert.equal(store.plugins[0].state, 'enabled', 'the stale refresh must not overwrite the mutation')
  assert.equal(settled, store.inventory)
})

test('polls cannot overtake an active mutation or overlap another inventory read', async () => {
  setActivePinia(createPinia())
  const pending = []
  const store = loadPluginStore(command => new Promise(resolve => pending.push({ command, resolve })))()
  const mutation = store.disable()
  await store.loadInventory()
  assert.deepEqual(pending.map(item => item.command), ['plugin_disable'])
  pending[0].resolve({ schema_version: 1, plugins: [{ id: 'agent-skills', kind: 'builtin', state: 'disabled' }] })
  await mutation
  assert.equal(store.plugins[0].state, 'disabled')
  const read = store.loadInventory()
  await store.loadInventory()
  assert.deepEqual(pending.map(item => item.command), ['plugin_disable', 'plugin_inventory'])
  pending[1].resolve({ schema_version: 1, plugins: [] })
  await read
  assert.equal(store.loading, false)
  assert.match(plugin, /:title="t\('settings.plugin.operationFailed'\)"/)
  assert.doesNotMatch(plugin, /:title="pluginStore.lastError"/)
})

test('the store exposes only the collection views the UI consumes', async () => {
  setActivePinia(createPinia())
  const store = loadPluginStore(async () => ({ schema_version: 1, plugins: [] }))()
  // The single-plugin compatibility getters have no consumer and were removed.
  assert.equal(store.plugin, undefined)
  assert.equal(store.installed, undefined)
  assert.equal(store.enabled, undefined)
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

test('plugin management contains only plugin lifecycle and status, not skill management', () => {
  assert.doesNotMatch(plugin, /AgentSkills|agent-skills|useCapabilityStore|stores\/capability/)
  assert.doesNotMatch(plugin, /useSkillStore|stores\/skill/)
  assert.match(plugin, /usePluginStore/)
  for (const action of ['install', 'enable', 'disable', 'uninstall']) {
    assert.match(plugin, new RegExp(`@click="${action}"`))
  }
})

test('every locale defines the same settings.plugin and workflow.plugin surfaces', () => {
  const requiredWorkflowKeys = ['title', 'empty', 'close', 'hide', 'openFailed', 'reload', 'newTab']
  const settingsReference = Object.keys(locales[0].messages.settings.plugin)
  const workflowReference = Object.keys(locales[0].messages.workflow.plugin)

  assert.deepEqual(settingsReference, [...settingsReference].sort(), 'settings.plugin keys are not sorted')
  assert.deepEqual(workflowReference, [...workflowReference].sort(), 'workflow.plugin keys are not sorted')
  assert.deepEqual(workflowReference, [...requiredWorkflowKeys].sort())

  for (const locale of locales) {
    const settingsPlugin = locale.messages.settings.plugin
    const workflowPlugin = locale.messages.workflow.plugin
    assert.deepEqual(Object.keys(settingsPlugin), settingsReference, `${locale.name} settings.plugin surface differs`)
    assert.deepEqual(Object.keys(workflowPlugin), workflowReference, `${locale.name} workflow.plugin surface differs`)
    assert.deepEqual(Object.keys(settingsPlugin), [...Object.keys(settingsPlugin)].sort(), `${locale.name} settings.plugin keys are not sorted`)
    assert.deepEqual(Object.keys(workflowPlugin), [...Object.keys(workflowPlugin)].sort(), `${locale.name} workflow.plugin keys are not sorted`)
    for (const [path, messages] of [['settings.plugin', settingsPlugin], ['workflow.plugin', workflowPlugin]]) {
      for (const [key, value] of flatten(messages)) {
        assert.equal(typeof value, 'string', `${locale.name}.${path}.${key} must be a string`)
        assert.notEqual(value.trim(), '', `${locale.name}.${path}.${key} must not be empty`)
      }
    }
  }
})

test('the page adapts to the dock container width, never the window viewport', () => {
  // A named inline-size container is the width the dock actually hands the page, so the
  // layout reacts to the panel and not to whatever size the whole application window has.
  assert.match(component, /container-name:\s*agent-skills/)
  assert.match(component, /container-type:\s*inline-size/)
  assert.match(component, /@container agent-skills \(max-width: 860px\)/)
  assert.match(component, /@container agent-skills \(max-width: 480px\)/)
  // Viewport media queries would answer the wrong question and are deliberately absent.
  assert.doesNotMatch(component, /@media\b/)
})

test('each wide table collapses into a full-data card list inside a narrow container', () => {
  // Findings, outcomes and installed skills are the three tables that cannot fit 600px.
  assert.equal((component.match(/<el-table[^>]*skill-table/g) || []).length, 3)
  assert.equal((component.match(/<ul[^>]*class="[^"]*skill-cards/g) || []).length, 3)

  // The cards mirror every findings column instead of dropping any of them.
  for (const field of ['severity', 'rule', 'path', 'detail']) {
    assert.match(component, new RegExp(`prop="${field}"`), `missing findings column ${field}`)
    assert.match(component, new RegExp(`row\\.${field}\\b`), `missing card value ${field}`)
  }

  // Outcomes keep their target, status tag and detail, including the install path fallback.
  assert.match(component, /prop="target_id"/)
  assert.match(component, /statusTagType\(row\.status\)/)
  assert.match(component, /row\.detail \|\| row\.install_path \|\| '-'/)

  // Installed skills keep name, source, target, every state tag and the uninstall action.
  for (const field of ['name', 'source']) {
    assert.match(component, new RegExp(`prop="${field}"`), `missing installed column ${field}`)
    assert.match(component, new RegExp(`row\\.${field}\\b`), `missing card value ${field}`)
  }
  for (const state of ['protected', 'managed', 'drifted', 'present']) {
    assert.match(component, new RegExp(`row\\.${state}\\b`), `missing state tag ${state}`)
  }
  // The card uninstall is the same gated and loading-aware control, never an unconditional one.
  assert.equal((component.match(/:disabled="!row\.uninstallable"/g) || []).length, 2)
  assert.equal(
    (component.match(/:loading="store\.applying"/g) || []).length >= 2,
    true,
    'the uninstall controls keep their loading state'
  )
  assert.match(component, /v-loading="store\.loading"/)
})

test('the desktop tables and their column widths are preserved', () => {
  // The card lists are hidden until the container is measured narrow.
  assert.match(component, /\.skill-cards\s*\{[^}]*display:\s*none/)
  const compact = component.slice(
    component.indexOf('@container agent-skills (max-width: 860px)'),
    component.indexOf('@container agent-skills (max-width: 480px)')
  )
  assert.match(compact, /\.skill-table\s*\{\s*display:\s*none/)
  // The fixed widths that define the wide desktop tables are unchanged.
  for (const width of [110, 220, 140, 160, 180, 120]) {
    assert.match(component, new RegExp(`width="${width}"`), `missing table width ${width}`)
  }
})

test('long values and narrow form controls wrap instead of overflowing the page', () => {
  // Paths and other unbreakable values are allowed to break inside their card.
  assert.match(component, /\.skill-cards__value\s*\{[^}]*overflow-wrap:\s*anywhere/)
  assert.match(component, /list-style:\s*none/)
  const narrow = component.slice(component.indexOf('@container agent-skills (max-width: 480px)'))
  // The narrow breakpoint stacks the checkbox targets and the action buttons.
  assert.match(narrow, /\.el-checkbox-group/)
  assert.match(narrow, /\.el-checkbox\b/)
  assert.match(narrow, /\.buttons[^}]*\.el-button/)
})

test('the component compiles as a Vue SFC and its scoped SCSS emits the container queries', async () => {
  const { descriptor, errors } = parseSfc(component)
  assert.deepEqual(errors, [], 'the SFC template must parse cleanly')
  assert.ok(descriptor.template, 'the page must have a template')
  assert.ok(descriptor.styles.some(entry => entry.lang === 'scss'), 'the page must have scoped SCSS')

  const template = compileTemplate({
    source: descriptor.template.content,
    filename: 'AgentSkills.vue',
    id: 'data-v-agent-skills',
    scoped: true
  })
  assert.deepEqual(template.errors, [], 'the template must compile')

  const style = descriptor.styles.find(entry => entry.lang === 'scss')
  const compiled = await compileStyleAsync({
    source: style.content,
    filename: 'AgentSkills.vue',
    id: 'data-v-agent-skills',
    scoped: true,
    preprocessLang: 'scss'
  })
  assert.deepEqual(compiled.errors, [], 'the SCSS must compile')
  // Both breakpoints survive compilation with the container name and the scope attribute intact.
  assert.match(
    compiled.code,
    /@container agent-skills \(max-width: 860px\)\s*\{\s*\.agent-skills \.skill-table\[data-v-agent-skills\]/
  )
  assert.match(compiled.code, /@container agent-skills \(max-width: 480px\)/)
})
