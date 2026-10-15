import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

import { createPluginProvider, verifiedPluginUis } from './pluginView.js'

/** A built-in plugin record with the exact fields the filter reads. */
const builtin = (id, state, verified, entry = `${id}/index.html`) => ({
  id,
  kind: 'builtin',
  state,
  ui: { entry, verified }
})

test('verifiedPluginUis keeps only enabled, verified, built-in plugin UIs', () => {
  const inventory = {
    schema_version: 1,
    plugins: [
      builtin('ready', 'enabled', true),
      builtin('disabled', 'disabled', true),
      builtin('unverified', 'enabled', false),
      builtin('not-installed', 'not_installed', true),
      { id: 'no-ui', kind: 'builtin', state: 'enabled' },
      { id: 'external', kind: 'external', state: 'enabled', ui: { entry: 'x', verified: true } }
    ]
  }

  assert.deepEqual(
    verifiedPluginUis(inventory).map(plugin => plugin.id),
    ['ready']
  )
})

test('verifiedPluginUis fails closed for any inventory it cannot trust', () => {
  const valid = { schema_version: 1, plugins: [builtin('ready', 'enabled', true)] }

  assert.deepEqual(verifiedPluginUis(null), [])
  assert.deepEqual(verifiedPluginUis(undefined), [])
  assert.deepEqual(verifiedPluginUis({}), [])
  assert.deepEqual(verifiedPluginUis({ schema_version: 2, plugins: valid.plugins }), [])
  assert.deepEqual(verifiedPluginUis({ schema_version: 1 }), [])
  assert.deepEqual(verifiedPluginUis({ schema_version: 1, plugins: {} }), [])
  assert.deepEqual(verifiedPluginUis(valid, true), [], 'an unavailable runtime hides every UI')
})

test('the plugin provider maps every dock command to the host command', async () => {
  const calls = []
  const provider = createPluginProvider({
    invoke: (command, args) => {
      calls.push({ command, args })
      return Promise.resolve()
    }
  })
  const bounds = { x: 1240, y: 40, width: 600, height: 700 }

  await provider.show('plugin:demo', { pluginId: 'demo', entry: 'demo/index.html', bounds, cornerRadius: 8 })
  await provider.hide()
  await provider.close('plugin:demo')
  await provider.clear()

  assert.deepEqual(calls, [
    {
      command: 'plugin_ui_open',
      args: { tabId: 'plugin:demo', pluginId: 'demo', entry: 'demo/index.html', bounds, cornerRadius: 8 }
    },
    { command: 'plugin_ui_hide', args: undefined },
    { command: 'plugin_ui_close', args: { tabId: 'plugin:demo' } },
    { command: 'plugin_ui_clear', args: undefined }
  ])
})

test('a plugin caller without a radius retains square-corner compatibility', async () => {
  let args
  const provider = createPluginProvider({ invoke: async (_command, payload) => { args = payload } })
  await provider.show('plugin:legacy', { pluginId: 'demo', entry: 'index.html' })
  assert.equal(args.cornerRadius, 0)
})

test('a provider without an invoke adapter resolves instead of throwing', async () => {
  const provider = createPluginProvider()

  await provider.show('plugin:demo', {})
  await provider.hide()
  await provider.close('plugin:demo')
  await provider.clear()
})

test('every native provider shares the one dock coordinator', () => {
  const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
  const workflow = read('../views/Workflow.vue')
  const sidebar = read('../components/workflow/WorkflowSidebar.vue')
  const entry = read('../components/workflow/PluginEntry.vue')
  const dock = read('../components/workflow/DockedViews.vue')

  // One serial coordinator owns both providers: no provider keeps a queue of its own.
  assert.match(workflow, /createDockedViewsCoordinator\(\{/)
  assert.match(workflow, /chatHub: chatHubProvider/)
  assert.match(workflow, /plugin: pluginProvider/)
  assert.match(workflow, /createPluginProvider\(\{ invoke: invokeWrapper \}\)/)
  assert.match(workflow, /createChatHubProvider\(\{ invoke: invokeWrapper \}\)/)
  assert.doesNotMatch(workflow, /createPluginViewController|createChatHubViewController/)
  // The commands are issued from the provider modules only.
  assert.doesNotMatch(workflow, /plugin_ui_open|plugin_ui_close|plugin_ui_clear|plugin_ui_hide/)
  assert.doesNotMatch(workflow, /show_chat_hub_page|hide_chat_hub_page|destroy_chat_hub_page/)

  // The right dock keeps the entry, the tab strip and the toolbar; the old bottom plugin
  // panel is gone.
  assert.doesNotMatch(workflow, /plugin-panel/)
  assert.match(workflow, /<PluginEntry[\s\S]*?<ChatHubEntry[\s\S]*?class="workflow-side-rail__item workflow-side-rail__terminal"/)
  assert.match(sidebar, /<PluginEntry[\s\S]*?<ChatHubEntry[\s\S]*?class="workflow-terminal-entry compact-terminal-entry"/)

  // The plugin entry only contains the launcher and dropdown. The opened plugin is shown
  // in the dock tab strip; ChatHub keeps its separate entry and latest-logo behavior.
  assert.match(entry, /name="plugin"/)
  assert.doesNotMatch(entry, /name="function"/)
  assert.doesNotMatch(entry, /plugin-entry__current|@click="emit\('toggle'\)"/)
  assert.doesNotMatch(entry, /chatHub|https?:|file:|plugin_ui_open/)

  // Agent Skills is a third native provider of the same dock, launched as a tab named from
  // i18n rather than as a second top-level window.
  assert.match(workflow, /agentSkills: agentSkillsProvider/)
  assert.match(workflow, /createAgentSkillsProvider\(\{ invoke: invokeWrapper \}\)/)
  assert.match(
    workflow,
    /if \(plugin\.id === AGENT_SKILLS_PLUGIN_ID\) \{[\s\S]*?return openDockTab\(\{[\s\S]*?kind: AGENT_SKILLS_TAB_KIND,[\s\S]*?\}\)/
  )
  assert.doesNotMatch(workflow, /open_agent_skills_window|TRUSTED_PLUGIN_ID/)
  assert.match(workflow, /verifiedPluginUis\(pluginStore\.inventory, !!pluginStore\.lastError\)/)
  assert.doesNotMatch(workflow, /pluginTabKind|trustedReloadKey|kind: 'trusted'/)
  assert.doesNotMatch(dock, /AgentSkills|trustedActive|trustedKey/)
  assert.doesNotMatch(dock, /plugin_ui_open|https?:|file:/)

  // A removed, disabled or replaced plugin tab is closed right away, so the dock fails
  // closed instead of showing a page the runtime no longer confirms.
  assert.match(workflow, /allowed\.get\(tab\.pluginId\) !== tab\.entry/)
  assert.match(workflow, /cs:\/\/plugins-changed/)
  assert.match(workflow, /setInterval\(refreshPlugins, 5000\)/)
  assert.match(workflow, /clearInterval\(pluginRefreshTimer\)/)
  assert.match(workflow, /void dockCoordinator\.dispose\(\)/)
})

test('the plugin tab limit matches the host limit and the same entry is deduped', () => {
  const workflow = readFileSync(new URL('../views/Workflow.vue', import.meta.url), 'utf8')

  assert.match(workflow, /const MAX_PLUGIN_TABS = 8/)
  // The tab id is derived from the plugin id, so clicking the same entry reuses its tab.
  assert.match(workflow, /const pluginTabId = pluginId => `plugin:\$\{pluginId\}`/)
  // The trusted surface does not count against the ordinary plugin panel limit.
  assert.match(workflow, /tab\.kind === 'plugin' && tab\.pluginId === plugin\.id/)
})