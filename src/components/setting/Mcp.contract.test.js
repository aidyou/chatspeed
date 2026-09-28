import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const component = readFileSync(new URL('./Mcp.vue', import.meta.url), 'utf8')
const store = readFileSync(new URL('../../stores/mcp.js', import.meta.url), 'utf8')

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
