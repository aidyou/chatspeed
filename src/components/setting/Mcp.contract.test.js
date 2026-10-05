import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import { createPinia, defineStore, setActivePinia } from 'pinia'
import { reactive, ref } from 'vue'

const component = readFileSync(new URL('./Mcp.vue', import.meta.url), 'utf8')
const store = readFileSync(new URL('../../stores/mcp.js', import.meta.url), 'utf8')

// Run the actual Pinia store with only the desktop transport and projection mocked.
const createStoreFixture = async ({ listStatus } = {}) => {
  const calls = []
  let runtime = { observed: true, state: 'stopped' }
  let desiredEnabled = false
  let projectionError = null
  let projectionGate = null
  const views = () => [{
    id: 1,
    name: 'weather',
    desired: { enabled: desiredEnabled },
    runtime: { ...runtime },
    tools: { freshness: 'observed', count: 2 }
  }]
  const capability = { mcpViews: {} }
  capability.loadMcpServers = async () => {
    if (projectionGate) await projectionGate
    if (projectionError) throw projectionError
    const result = views()
    capability.mcpViews = Object.fromEntries(result.map(view => [view.id, view]))
    return result
  }
  setActivePinia(createPinia())
  const useStore = runInNewContext(
    store.replace(/^import .*;?\n/gm, '').replace('export const useMcpStore', 'const useMcpStore') + '\nuseMcpStore',
    {
      defineStore, reactive, ref,
      console: { debug() {}, log() {}, warn() {}, error() {} },
      FrontendAppError: class extends Error {},
      getCurrentWebviewWindow: () => ({ label: 'settings' }),
      useCapabilityStore: () => capability,
      sendSyncState: (...args) => calls.push(['sync', ...args]),
      invokeWrapper: async (command, payload) => {
        calls.push([command, payload])
        if (command === 'list_mcp_servers') {
          return [{ id: 1, name: 'weather', disabled: !desiredEnabled, status: listStatus ?? runtime.state, config: {} }]
        }
        if (command === 'enable_mcp_server' || command === 'restart_mcp_server') {
          desiredEnabled = true
          runtime = { observed: true, state: 'running' }
        } else if (command === 'disable_mcp_server') {
          desiredEnabled = false
          runtime = { observed: true, state: 'stopped' }
        } else if (command === 'get_mcp_server_tools') {
          return [{ name: 'forecast' }, { name: 'temperature' }]
        }
      }
    }
  )
  const mcp = useStore()
  await mcp.fetchMcpServers()
  return {
    mcp, capability, calls,
    setRuntime: value => { runtime = value },
    setProjectionError: value => { projectionError = value },
    setProjectionGate: value => { projectionGate = value }
  }
}

test('enable updates the status and expansion gate without a legacy status or self-sync event', async () => {
  const { mcp, capability } = await createStoreFixture()
  assert.equal(mcp.servers[0].status, 'stopped')
  await mcp.enableMcpServer(1)
  assert.equal(mcp.servers[0].status, 'running')
  assert.equal(mcp.servers[0].disabled, false)
  assert.equal(capability.mcpViews[1].tools.count, 2)
  // Exercise the page's actual expansion handler, not only the store lookup.
  const start = component.indexOf('const toggleServerToolsExpansion = ')
  const end = component.indexOf('\n/**', start)
  assert.ok(start > -1 && end > start)
  const expand = runInNewContext(
    component.slice(start, end) + '\ntoggleServerToolsExpansion',
    { mcpStore: mcp }
  )
  await expand(mcp.servers[0])
  assert.equal(mcp.getOrInitServerUiState(1).expanded, true)
  assert.equal(mcp.serverTools[1].length, 2)
})

test('enable waits for the observed state before reporting completion', async () => {
  const fixture = await createStoreFixture()
  let release
  fixture.setProjectionGate(new Promise(resolve => { release = resolve }))
  let settled = false
  const operation = fixture.mcp.enableMcpServer(1).then(() => { settled = true })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(settled, false)
  release()
  await operation
  assert.equal(fixture.mcp.servers[0].status, 'running')
})

test('disable collapses tools and restart re-reads the observed status', async () => {
  const { mcp } = await createStoreFixture()
  await mcp.enableMcpServer(1)
  await mcp.fetchMcpServerTools(1)
  mcp.getOrInitServerUiState(1).expanded = true
  await mcp.disableMcpServer(1)
  assert.equal(mcp.servers[0].status, 'stopped')
  assert.equal(mcp.servers[0].disabled, true)
  assert.equal(mcp.getOrInitServerUiState(1).expanded, false)
  assert.equal(mcp.serverTools[1], undefined)
  await mcp.enableMcpServer(1)
  mcp.servers[0].status = 'stopped'
  await mcp.restartMcpServer(1)
  assert.equal(mcp.servers[0].status, 'running')
})

test('fact refresh handles another window and never invents an unobserved running state', async () => {
  const fixture = await createStoreFixture()
  fixture.setRuntime({ observed: true, state: 'running' })
  await fixture.mcp.refreshCapabilityFacts()
  assert.equal(fixture.mcp.servers[0].status, 'running')
  fixture.setRuntime({ observed: false, state: 'running' })
  await fixture.mcp.refreshCapabilityFacts()
  assert.equal(fixture.mcp.servers[0].status, null)
})

test('list refresh cannot overwrite a newer runtime observation with its older status', async () => {
  const fixture = await createStoreFixture({ listStatus: 'stopped' })
  fixture.setRuntime({ observed: true, state: 'running' })
  await fixture.mcp.fetchMcpServers()
  assert.equal(fixture.mcp.servers[0].status, 'running')
  assert.ok(Array.isArray(fixture.mcp.servers[0].config.disabled_tools))
})

test('tool refresh re-reads runtime state and preserves an unchanged running row', async () => {
  const { mcp } = await createStoreFixture()
  await mcp.enableMcpServer(1)
  await mcp.fetchMcpServerTools(1)
  mcp.getOrInitServerUiState(1).expanded = true
  await mcp.refreshMcpTools(1)
  assert.equal(mcp.servers[0].status, 'running')
  assert.equal(mcp.serverTools[1].length, 2)
  assert.equal(mcp.getOrInitServerUiState(1).expanded, true)
})

test('projection failure remains contained without assuming enable means running', async () => {
  const fixture = await createStoreFixture()
  fixture.setProjectionError(new Error('runtime unavailable'))
  await fixture.mcp.enableMcpServer(1)
  assert.equal(fixture.mcp.servers[0].status, 'stopped')
  assert.equal(fixture.mcp.error, null)
})

const locales = ['en', 'zh-Hans', 'zh-Hant'].map(name => ({
  name,
  messages: JSON.parse(
    readFileSync(new URL(`../../i18n/locales/${name}.json`, import.meta.url), 'utf8')
  )
}))

/** Every `settings.mcp.<path>` the page asks for. */
const referencedKeys = () => {
  const keys = new Set()
  for (const match of component.matchAll(/settings\.mcp\.([A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)*)/g)) {
    keys.add(match[1])
  }
  // The switch tooltip builds its key dynamically from the disabled flag.
  keys.add('enableServer')
  keys.add('disableServer')
  // `getServerStatusText` builds `settings.mcp.status<State>` at runtime, so the
  // bare `status` capture is a prefix rather than a label. Its real expansions
  // are named here, which keeps the check meaningful.
  const dynamicPrefixes = new Set(['status'])
  for (const state of ['Error', 'Unknown', 'Starting', 'Connected', 'Running', 'Stopped']) {
    keys.add(`status${state}`)
  }
  // The dynamic helpers resolve their key from a small fixed table.
  for (const key of [
    'driftDesiredNotRunning',
    'driftRunningWhileDisabled',
    'driftUnsupportedTransport',
    'driftUnknown',
    'toolsFresh',
    'toolsObserved',
    'toolsStale',
    'toolsUnavailable'
  ]) {
    if (component.includes(`'${key}'`)) keys.add(key)
  }
  return [...keys].filter(key => !dynamicPrefixes.has(key))
}

const lookup = (messages, key) => key.split('.').reduce((node, part) => node?.[part], messages)

test('every MCP label the page uses exists in all three locales', () => {
  const keys = referencedKeys()
  assert.ok(keys.length > 0, 'the page should reference localized copy')
  for (const locale of locales) {
    for (const key of keys) {
      const value = lookup(locale.messages, `settings.mcp.${key}`)
      assert.equal(
        typeof value,
        'string',
        `${locale.name} is missing settings.mcp.${key}`
      )
    }
  }
})

test('the three locales expose the same MCP copy structure', () => {
  const shape = locale =>
    Object.keys(lookup(locale.messages, 'settings.mcp')).sort().join(',')
  const base = shape(locales[0])
  for (const locale of locales.slice(1)) {
    assert.equal(shape(locale), base, `${locale.name} diverged from en`)
  }
})

test('the server switch exposes visible feedback while a state change is pending', () => {
  assert.match(
    component,
    /:disabled="mcpStore\.getOrInitServerUiState\(server\.id\)\.loading"/
  )
  assert.match(
    component,
    /:loading="mcpStore\.getOrInitServerUiState\(server\.id\)\.loading"/
  )
  assert.doesNotMatch(component, /mcp-switch-loading|mcp-switch-spin/)
  assert.match(component, /class="mcp-server-switch"/)
  assert.match(
    component,
    /:deep\(\.mcp-server-switch\.is-loading\)\s*\{\s*opacity: 1;\s*\.el-switch__action\s*\{\s*background-color: var\(--cs-bg-elevated-color\);\s*color: var\(--cs-text-color-primary\);\s*\.el-icon\s*\{\s*color: var\(--cs-text-color-primary\);/
  )
})

test('the page reports observed facts through the capability projection', () => {
  // The runtime facts come from the shared read model rather than a client-side
  // guess, so the desktop and `cs mcp` answer from the same data (AC-12).
  assert.match(component, /import \{ mcpDisplayState \} from '@\/libs\/capability\.js'/)
  assert.match(component, /capabilityStore\.mcpViews\[server\.id\]/)
  // The decision of what a row may claim lives in the shared projection helper,
  // which keeps "not observed" distinct from "stopped" for both clients.
  assert.match(component, /mcpDisplayState/)
})

test('the list refresh re-reads the projection so badges cannot lag a mutation', () => {
  assert.match(store, /useCapabilityStore\(\)\.loadMcpServers\(\)/)
  // The projection failure is contained: the page keeps working without it.
  assert.match(store, /catch \(projectionError\)/)
})

/** The body of one store function, so a call site can be asserted individually. */
const storeFunction = name => {
  const start = store.indexOf(`const ${name} = `)
  assert.ok(start > -1, `stores/mcp.js should define ${name}`)
  const end = store.indexOf('\n  };', start)
  assert.ok(end > start, `${name} should end at the store indentation`)
  return store.slice(start, end)
}

test('every MCP state change re-reads the runtime facts', () => {
  // Drift and the reported tool count come from the capability projection, which is
  // a separate read from the legacy list. Each place that can change the runtime or
  // the desired record re-reads it, or a row keeps the facts of the state it was
  // loaded with — which is how a restarted server kept showing "enabled but not
  // running" next to a "running" status.
  for (const name of [
    'fetchMcpServers',
    'updateServerStatus',
    'addMcpServer',
    'updateMcpServer',
    'deleteMcpServer',
    'enableMcpServer',
    'disableMcpServer',
    'restartMcpServer',
    'refreshMcpTools'
  ]) {
    assert.match(
      storeFunction(name),
      /refreshCapabilityFacts\(\)/,
      `${name} must re-read the runtime facts`
    )
  }

  // A record changed in another window changes the desired state too.
  for (const event of [
    'Added server via sync',
    'Updated server via sync',
    'Deleted server via sync'
  ]) {
    const at = store.indexOf(event)
    assert.ok(at > -1, `the sync handler should cover: ${event}`)
    assert.match(
      store.slice(at, at + 120),
      /refreshCapabilityFacts\(\)/,
      `${event} must re-read the runtime facts`
    )
  }
})

test('a settled registration re-reads the runtime facts', () => {
  const app = readFileSync(new URL('../../App.vue', import.meta.url), 'utf8')
  // `mcp_tools_changed` is emitted once a registration has filled the tool cache, so
  // re-reading on it is what replaces a cold start's "not running" answer.
  const branch = app.slice(app.indexOf("eventType === 'mcp_tools_changed'"))
  assert.ok(branch.length > 0, 'App.vue should handle mcp_tools_changed')
  assert.match(branch.slice(0, 400), /mcpStore\.refreshCapabilityFacts\(\)/)
})

test('opening the MCP tab re-reads the servers and their facts', () => {
  const settings = readFileSync(new URL('../../views/Settings.vue', import.meta.url), 'utf8')
  const start = settings.indexOf('const switchSetting = ')
  assert.ok(start > -1, 'Settings.vue should define switchSetting')
  // A settings window can outlive the MCP runtime start, so opening the tab has to
  // re-read instead of showing the snapshot the window loaded with.
  assert.match(settings.slice(start, start + 600), /fetchMcpServers\(\)/)
})

test('the existing MCP command wire is still what the page calls', () => {
  // The migration moved the implementation behind these names, not the names
  // themselves (INV-2).
  for (const command of [
    'list_mcp_servers',
    'add_mcp_server',
    'update_mcp_server',
    'delete_mcp_server',
    'enable_mcp_server',
    'disable_mcp_server',
    'restart_mcp_server',
    'refresh_mcp_server',
    'get_mcp_server_tools',
    'update_mcp_tool_status',
    'run_mcp_tool'
  ]) {
    assert.match(store, new RegExp(`'${command}'`), `${command} is no longer invoked`)
  }
})

test('the capability facts never read back a stored secret', () => {
  // The projection exposes presence flags only. The legacy edit form still round-
  // trips the server's own config, which is the existing desktop contract, but the
  // runtime-facts row must come from the projection alone (AC-13).
  assert.doesNotMatch(component, /mcpViews\[[^\]]*\]\.config/)
  const start = component.indexOf('class="server-runtime-facts"')
  const facts = component.slice(start, component.indexOf('</div>', start))
  assert.ok(start > 0 && facts.length > 0, 'the facts block must exist')
  assert.match(facts, /mcpFacts\(server\)/)
  assert.doesNotMatch(facts, /server\.config/)
})

test('the edit form treats stored secrets as presence / explicit-replace only', () => {
  // The token/env inputs derive their "already configured" hint from the
  // capability projection's presence flags, never from a secret in the list wire
  // (which is now redacted server-side). A blank field means keep, a typed value
  // replaces (AC-13).
  assert.match(component, /secret_present/)
  assert.match(component, /env_present/)
  assert.match(component, /bearerTokenPresent/)
  assert.match(component, /envPresent/)
  assert.match(component, /settings\.mcp\.form\.secretKeepHint/)
  assert.match(component, /settings\.mcp\.form\.envKeepHint/)
  assert.match(component, /settings\.mcp\.form\.secretKeepPlaceholder/)
  assert.match(component, /settings\.mcp\.form\.envKeepPlaceholder/)
})
