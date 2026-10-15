import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { createPinia, defineStore, setActivePinia } from 'pinia'
import { ref, computed } from 'vue'
import { compileStyleAsync, compileTemplate, parse as parseSfc } from 'vue/compiler-sfc'
import { mutationOutcomes, parseCapabilityError } from '../../libs/capability.js'

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

test('the Agent Skills page is separate from the prompt Skill settings page', () => {
  // File-based Agent Skills never share the database prompt skill store.
  assert.doesNotMatch(component, /useSkillStore|stores\/skill/)
  assert.match(component, /import { useCapabilityStore } from '@\/stores\/capability'/)
  // The original prompt Skill page remains in the settings center; Agent Skills is a docked page.
  assert.match(settings, /id: 'skill'/)
  assert.match(settings, /components\/setting\/Skill\.vue/)
  assert.match(settings, /settingType === 'skill'/)
  assert.match(settings, /<skill \/>/)
  assert.doesNotMatch(settings, /id: 'agentSkills'/)
  assert.doesNotMatch(settings, /components\/setting\/AgentSkills\.vue/)
  // The plugin tab keeps the dedicated plugin icon.
  assert.match(settings, /icon: 'plugin', id: 'plugin'/)
  assert.match(settings, /<plugin \/>/)
  // The general page is a single pane: no nested tab, no leftover tab state or styles.
  assert.doesNotMatch(general, /el-tabs|el-tab-pane|generalTab|general-tabs/)
  assert.doesNotMatch(general, /name="agentSkills"|<agent-skills \/>/)
})

test('the page splits the inventory and the install flow into ordered tabs', () => {
  // The list tab is declared first and is the default active tab.
  assert.match(component, /const activeTab = ref\('list'\)/)
  assert.match(component, /<el-tabs v-model="activeTab" class="skill-tabs">/)
  const names = [...component.matchAll(/<el-tab-pane[^>]*name="([^"]+)"/g)].map(match => match[1])
  assert.deepEqual(names, ['list', 'install'], 'the install tab must be the second tab')
  // Both tab labels come from i18n, never from a literal.
  assert.match(component, /:label="t\('settings\.agentSkills\.tabList'\)"/)
  assert.match(component, /:label="t\('settings\.agentSkills\.tabInstall'\)"/)

  // The first pane owns the installed list plus the doctor and reconcile reports.
  const listPane = component.slice(
    component.indexOf('name="list"'),
    component.indexOf('name="install"')
  )
  for (const key of ['installed', 'doctor', 'reconcile']) {
    assert.match(listPane, new RegExp(`settings\\.agentSkills\\.${key}`), `list pane is missing ${key}`)
  }
  assert.doesNotMatch(
    listPane,
    /settings\.agentSkills\.(source|targets|check)\b/,
    'install controls leaked into the list pane'
  )

  // The second pane owns source, check report, targets and install.
  const installPane = component.slice(
    component.indexOf('name="install"'),
    component.indexOf('</el-tabs>')
  )
  for (const key of ['source', 'targets', 'install']) {
    assert.match(
      installPane,
      new RegExp(`settings\\.agentSkills\\.${key}`),
      `install pane is missing ${key}`
    )
  }
  assert.match(installPane, /t\('settings\.agentSkills\.check'\)/)
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

test('the page shows mutation failures in a dialog without a last-operation table', () => {
  assert.match(component, /const failures = results\.filter/)
  assert.match(component, /result\.detail \|\| result\.status/)
  assert.match(component, /v-model="operationErrorVisible"/)
  assert.doesNotMatch(component, /statusTagType|const outcomes =|settings\.agentSkills\.result|:data="outcomes"/)
  // Findings and permissions from the check report are rendered.
  assert.match(component, /report\.findings/)
  assert.match(component, /report\.permissions/)
  assert.match(component, /report\.checker_version/)
  // An uninstall is only offered when the backend says it is permitted.
  assert.match(component, /:disabled="!row\.uninstallable"/)
  // Uninstall shows the exact directory for confirmation before forwarding the mutation.
  assert.match(component, /<el-dialog[\s\S]*uninstallDialogVisible/)
  assert.match(component, /append-to-body/)
  assert.doesNotMatch(component, /:teleported=/)
  assert.match(component, /openUninstallDialog\(row\)/)
  assert.match(component, /row\.directory/)
  assert.doesNotMatch(component, /ElMessageBox/)
})

test('uninstall opens confirmation before mutation and cancel leaves the skill untouched', async () => {
  const { descriptor } = parseSfc(component)
  const calls = []
  const store = {
    applying: false,
    uninstall: async payload => {
      calls.push(payload)
      return { result: { outcomes: [{ target_id: 'chatspeed', status: 'removed' }] } }
    }
  }
  const page = runInNewContext(
    descriptor.scriptSetup.content.replace(/^import .*;\n/gm, '') +
      '\n({ uninstall, closeUninstallDialog, confirmUninstall, uninstallDialogVisible, uninstallDialogMessage })',
    {
      ref,
      computed,
      watch: () => {},
      onMounted: () => {},
      useCapabilityStore: () => store,
      useI18n: () => ({ t: (key, values) => values ? `${values.name}\n${values.directory}` : key }),
      mutationOutcomes,
      verdictAllowsInstall: () => false,
      ElMessage: { success: () => {}, error: error => assert.fail(error) }
    }
  )
  const row = { name: 'demo', directory: '/sandbox/skills/demo', target_id: 'chatspeed' }
  page.uninstall(row)
  assert.equal(page.uninstallDialogVisible.value, true)
  assert.equal(page.uninstallDialogMessage.value, 'demo\n/sandbox/skills/demo')
  assert.equal(calls.length, 0)
  page.closeUninstallDialog()
  assert.equal(page.uninstallDialogVisible.value, false)
  assert.equal(calls.length, 0)
  page.uninstall(row)
  await page.confirmUninstall()
  assert.equal(calls.length, 1)
  assert.equal(calls[0].skillName, row.name)
  assert.deepEqual([...calls[0].targets], ['chatspeed'])
  assert.equal(page.uninstallDialogVisible.value, false)
})

test('uninstall feedback follows the effect outcome, not transport success', async () => {
  const { descriptor } = parseSfc(component)
  const successes = []
  let result
  let failure
  const store = {
    uninstall: async () => {
      if (failure) throw failure
      return result
    }
  }
  const page = runInNewContext(
    descriptor.scriptSetup.content.replace(/^import .*;\n/gm, '') +
      '\n({ uninstall, confirmUninstall, install, uninstallDialogVisible, operationErrorVisible, operationError })',
    {
      ref, computed, watch: () => {}, onMounted: () => {},
      useCapabilityStore: () => store,
      useI18n: () => ({ t: key => key }),
      mutationOutcomes, verdictAllowsInstall: () => false,
      ElMessage: { success: message => successes.push(message), error: error => assert.fail(error) }
    }
  )
  const row = { name: 'demo', target_id: 'chatspeed', directory: '/sandbox/skills/demo' }
  for (const status of ['removed', 'finalized']) {
    result = { result: { outcomes: [{ status }] } }
    page.uninstall(row)
    await page.confirmUninstall()
    assert.equal(page.uninstallDialogVisible.value, false)
    assert.equal(page.operationErrorVisible.value, false)
  }
  assert.deepEqual(successes, Array(2).fill('settings.agentSkills.uninstallDone'))
  for (const status of ['refused', 'failed', 'unsupported', 'not_found', 'unknown']) {
    result = { result: { outcomes: [{ status, detail: 'Permission denied (os error 13)' }] } }
    page.operationErrorVisible.value = false
    page.uninstall(row)
    await page.confirmUninstall()
    assert.equal(page.uninstallDialogVisible.value, false)
    assert.equal(page.operationErrorVisible.value, true)
    assert.equal(page.operationError.value, 'Permission denied (os error 13)')
  }
  for (const malformed of [null, { result: { outcomes: [] } }, { result: { outcomes: [{}] } }]) {
    result = malformed
    page.operationErrorVisible.value = false
    page.uninstall(row)
    await page.confirmUninstall()
    assert.equal(page.operationErrorVisible.value, true)
    assert.equal(page.operationError.value, 'settings.agentSkills.operationFailed')
  }
  failure = new Error('inventory refresh failed')
  page.uninstall(row)
  await page.confirmUninstall()
  assert.equal(page.uninstallDialogVisible.value, false)
  assert.equal(page.operationError.value, failure.message)
  assert.equal(successes.length, 2, 'failures must never produce a success toast')

  failure = null
  store.install = async () => ({ result: { install: { outcomes: [{ status: 'blocked' }] } } })
  await page.install()
  assert.equal(page.operationError.value, 'blocked')
  assert.equal(successes.length, 2)
  store.install = async () => ({ result: { install: { outcomes: [{ status: 'installed' }] } } })
  await page.install()
  assert.equal(successes.at(-1), 'settings.agentSkills.installDone')
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

test('the report enums and finding codes render through localized helpers, never as raw values', () => {
  // A backend enum or code must never be the visible text of a user-facing field.
  assert.doesNotMatch(component, /\{\{\s*report\.verdict\s*\}\}/)
  assert.doesNotMatch(component, /\{\{\s*row\.severity\s*\}\}/)
  assert.doesNotMatch(component, /\{\{\s*row\.rule\s*\}\}/)
  assert.doesNotMatch(component, /\{\{\s*target\.id\s*\}\}/)
  assert.doesNotMatch(component, /\{\{\s*row\.target_id\s*\|\|\s*'-'\s*\}\}/)
  assert.doesNotMatch(component, /store\.doctor\.findings\.join/)

  // Each enum family has a helper and the template routes the field through it.
  for (const helper of [
    'verdictLabel',
    'severityLabel',
    'ruleLabel',
    'sourceLabel',
    'targetLabel',
    'doctorFindingLabel'
  ]) {
    assert.match(
      component,
      new RegExp(`(const ${helper} =|function ${helper}\\()`),
      `missing localized helper ${helper}`
    )
  }
  assert.match(component, /verdictLabel\(report\.verdict\)/)
  assert.match(component, /sourceLabel\(report\.source_kind\)/)
  assert.match(component, /severityLabel\(row\.severity\)/)
  assert.match(component, /ruleLabel\(row\.rule\)/)
  assert.match(component, /targetLabel\(target\.id\)/)
  assert.match(component, /targetLabel\(row\.target_id\)/)
  assert.match(component, /sourceLabel\(row\.source\)/)
  assert.match(component, /doctorFindingsText/)
})

test('every locale localizes the same enum families, sorted and non-empty', () => {
  const families = {
    verdictLabel: ['pass', 'blocked', 'inconclusive'],
    severityLabel: ['block', 'inconclusive', 'warn'],
    ruleLabel: ['permission', 'content', 'manifest', 'generic'],
    sourceLabel: ['github', 'local_directory', 'local_zip', 'installed', 'directory'],
    targetLabel: [
      'chatspeed',
      'agents',
      'claude-code',
      'codex',
      'opencode',
      'cursor',
      'windsurf',
      'cline',
      'trae-cn',
      'dsh',
      'workbuddy',
      'qoder'
    ],
    doctorFinding: [
      'operation_interrupted',
      'operation_needs_reconcile',
      'skill_drifted',
      'skill_directory_missing',
      'mcp_drift',
      'staging_residue',
      'quarantine_residue'
    ]
  }
  const reference = Object.fromEntries(
    Object.keys(families).map(family => [
      family,
      Object.keys(locales[0].messages.settings.agentSkills[family])
    ])
  )
  for (const locale of locales) {
    const surface = locale.messages.settings.agentSkills
    for (const [family, values] of Object.entries(families)) {
      assert.ok(surface[family], `${locale.name} is missing the ${family} map`)
      assert.deepEqual(
        Object.keys(surface[family]),
        reference[family],
        `${locale.name}.${family} surface differs`
      )
      assert.deepEqual(
        Object.keys(surface[family]),
        [...Object.keys(surface[family])].sort(),
        `${locale.name}.${family} keys are not sorted`
      )
      for (const value of values) {
        assert.equal(
          typeof surface[family][value],
          'string',
          `${locale.name}.${family}.${value} must be a string`
        )
        assert.notEqual(
          surface[family][value].trim(),
          '',
          `${locale.name}.${family}.${value} must not be empty`
        )
      }
    }
  }
})

test('the static-check warning is visible in every locale and marks findings as possible risk', () => {
  for (const locale of locales) {
    const warning = locale.messages.settings.agentSkills.staticCheckWarning
    assert.equal(typeof warning, 'string', `${locale.name} is missing the static-check warning`)
    assert.ok(warning.trim().length > 40, `${locale.name} warning is too short to be meaningful`)
  }
  // The warning is rendered next to the report/install surface, not just defined.
  assert.match(component, /t\('settings\.agentSkills\.staticCheckWarning'\)/)
  // The passing verdict must read as a heuristic, never as a proven-safe statement.
  const english = locales.find(locale => locale.name === 'en').messages.settings.agentSkills
  assert.match(english.verdict, /heuristic/i)
  assert.match(english.verdictLabel.pass, /heuristic|cannot prove|possible risk/i)
})

test('the localized helpers resolve known codes and keep unknown codes as raw evidence', () => {
  const { descriptor } = parseSfc(component)
  const english = locales.find(locale => locale.name === 'en').messages
  const resolve = key =>
    key.split('.').reduce((node, part) => (node == null ? undefined : node[part]), english)
  const page = runInNewContext(
    descriptor.scriptSetup.content.replace(/^import .*;\n/gm, '') +
      '\n({ verdictLabel, severityLabel, ruleLabel, sourceLabel, targetLabel, doctorFindingLabel })',
    {
      ref,
      computed,
      watch: () => {},
      onMounted: () => {},
      useCapabilityStore: () => ({}),
      useI18n: () => ({
        t: (key, values) => {
          const value = resolve(key)
          if (value === undefined) return key
          return values
            ? value.replace(/\{(\w+)\}/g, (_, name) => String(values[name] ?? ''))
            : value
        },
        te: key => resolve(key) !== undefined
      }),
      mutationOutcomes,
      verdictAllowsInstall: () => true,
      ElMessage: { success: () => {}, error: () => {} }
    }
  )
  const surface = english.settings.agentSkills

  assert.equal(page.verdictLabel('pass'), surface.verdictLabel.pass)
  assert.equal(page.verdictLabel('blocked'), surface.verdictLabel.blocked)
  assert.equal(page.severityLabel('warn'), surface.severityLabel.warn)
  assert.equal(page.sourceLabel('local_directory'), surface.sourceLabel.local_directory)
  assert.equal(page.sourceLabel('managed_drifted'), surface.sourceLabel.managed_drifted)
  assert.equal(page.targetLabel('chatspeed'), surface.targetLabel.chatspeed)
  assert.equal(page.targetLabel('claude-code'), surface.targetLabel['claude-code'])
  assert.equal(page.doctorFindingLabel('mcp_drift'), surface.doctorFinding.mcp_drift)

  // A rule id is reduced to its family; an unknown family still reads as the generic label.
  assert.equal(page.ruleLabel('skill.permission.sensitive_path'), surface.ruleLabel.permission)
  assert.equal(page.ruleLabel('skill.content.binary'), surface.ruleLabel.content)
  assert.equal(page.ruleLabel('skill.something.new'), surface.ruleLabel.generic)
  assert.equal(page.ruleLabel('not-a-rule'), surface.ruleLabel.generic)

  // Unknown and missing codes fall back to the raw machine value, never to a guess.
  assert.equal(page.verdictLabel('future-verdict'), 'future-verdict')
  assert.equal(page.targetLabel('future-target'), 'future-target')
  assert.equal(page.targetLabel(undefined), '-')
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
  // Findings remain responsive; operation feedback now lives in toasts and dialogs.
  assert.equal((component.match(/<el-table[^>]*skill-table/g) || []).length, 1)
  assert.equal((component.match(/<ul[^>]*class="[^"]*skill-cards/g) || []).length, 1)

  // The cards mirror every findings column instead of dropping any of them. Severity and rule
  // are localized labels; the path and detail stay the raw machine evidence.
  for (const field of ['path', 'detail']) {
    assert.match(component, new RegExp(`prop="${field}"`), `missing findings column ${field}`)
  }
  for (const field of ['severity', 'rule', 'path', 'detail']) {
    assert.match(component, new RegExp(`row\\.${field}\\b`), `missing card value ${field}`)
  }

  // Installed skills render as a resident, bordered data table keeping name, source and state.
  assert.match(component, /class="data-table"/)
  assert.match(component, /<el-table[^>]*border/, 'installed data table keeps its grid')
  assert.match(component, /prop="name"/, 'missing installed name column')
  assert.match(component, /sourceLabel\(row\.source\)/, 'the installed source column is localized')
  assert.match(component, /row\.target_id\b/, 'installed data table keeps the target column')
  for (const state of ['protected', 'managed', 'drifted', 'present']) {
    assert.match(component, new RegExp(`row\\.${state}\\b`), `missing state tag ${state}`)
  }
  // The installed uninstall is a gated and loading-aware control, never an unconditional one.
  assert.equal((component.match(/:disabled="!row\.uninstallable"/g) || []).length, 1)
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
  // Findings and installed action column retain their widths.
  for (const width of [110, 220, 76]) {
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
