import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

import {
  AGENT_SKILLS_PLUGIN_ID,
  agentSkillsVerified,
  createAgentSkillsProvider
} from './agentSkillsView.js'

/** A built-in plugin record with the exact fields the filter reads. */
const builtin = (id, state, verified, entry = `${id}/index.html`) => ({
  id,
  kind: 'builtin',
  state,
  ui: { entry, verified }
})

const inventory = (...plugins) => ({ schema_version: 1, plugins })

/** An external (non-built-in) record, which the filter must never select. */
const externalRecord = (id, state, verified) => ({
  id,
  kind: 'external',
  state,
  ui: { entry: `${id}/index.html`, verified }
})

test('agentSkillsVerified keeps only an enabled, verified, built-in agent-skills bundle', () => {
  assert.equal(agentSkillsVerified(inventory(builtin('agent-skills', 'enabled', true))), true)
  assert.equal(agentSkillsVerified(inventory(builtin('agent-skills', 'disabled', true))), false)
  assert.equal(agentSkillsVerified(inventory(builtin('agent-skills', 'enabled', false))), false)
  assert.equal(
    agentSkillsVerified(inventory(builtin('agent-skills', 'not_installed', false))),
    false
  )
  assert.equal(agentSkillsVerified(inventory(builtin('other', 'enabled', true))), false)
  assert.equal(agentSkillsVerified(inventory(externalRecord('external', 'enabled', true))), false)
})

test('agentSkillsVerified fails closed for any inventory it cannot trust', () => {
  const valid = inventory(builtin('agent-skills', 'enabled', true))

  assert.equal(agentSkillsVerified(null), false)
  assert.equal(agentSkillsVerified(undefined), false)
  assert.equal(agentSkillsVerified({}), false)
  assert.equal(agentSkillsVerified({ schema_version: 2, plugins: valid.plugins }), false)
  assert.equal(agentSkillsVerified({ schema_version: 1 }), false)
  assert.equal(agentSkillsVerified(valid, true), false, 'an unavailable runtime hides the surface')
})

test('the provider maps every dock command to the carrier command', async () => {
  const calls = []
  const provider = createAgentSkillsProvider({
    invoke: (command, args) => {
      calls.push({ command, args })
      return Promise.resolve()
    }
  })
  const bounds = { x: 1240, y: 40, width: 600, height: 700 }

  await provider.show('agentSkills', { bounds, cornerRadius: 8 })
  await provider.hide()
  await provider.close('agentSkills')

  assert.deepEqual(calls, [
    { command: 'show_agent_skills_view', args: { bounds, cornerRadius: 8 } },
    { command: 'hide_agent_skills_view', args: undefined },
    { command: 'close_agent_skills_view', args: undefined }
  ])
})

test('a provider caller without a radius retains square-corner compatibility', async () => {
  let args
  const provider = createAgentSkillsProvider({
    invoke: async (_command, payload) => {
      args = payload
    }
  })

  await provider.show('agentSkills', { bounds: { x: 0, y: 0, width: 10, height: 10 } })

  assert.equal(args.cornerRadius, 0)
})

test('a provider without an invoke adapter resolves instead of throwing', async () => {
  const provider = createAgentSkillsProvider()

  await provider.show('agentSkills', {})
  await provider.hide()
  await provider.close('agentSkills')
})

test('the trusted surface is a third provider of the one dock coordinator', () => {
  const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
  const workflow = read('../views/Workflow.vue')
  const dock = read('../components/workflow/DockedViews.vue')

  // One serial coordinator owns all three providers: none keeps a queue of its own.
  assert.match(
    workflow,
    /createDockedViewsCoordinator\(\{\s*chatHub: chatHubProvider,\s*plugin: pluginProvider,\s*agentSkills: agentSkillsProvider,/
  )
  assert.match(workflow, /const agentSkillsProvider = createAgentSkillsProvider\(\{ invoke: invokeWrapper \}\)/)
  assert.match(workflow, /import \{[\s\S]*?AGENT_SKILLS_PLUGIN_ID,[\s\S]*?createAgentSkillsProvider[\s\S]*?\} from '@\/libs\/agentSkillsView'/)

  // The command payloads live in the provider module, never in the view.
  assert.doesNotMatch(workflow, /show_agent_skills_view|hide_agent_skills_view|close_agent_skills_view/)

  // The trusted surface opens as a dock tab named from i18n, never as a separate window.
  assert.match(workflow, /if \(plugin\.id === AGENT_SKILLS_PLUGIN_ID\) \{/)
  assert.match(workflow, /kind: AGENT_SKILLS_TAB_KIND,\s*pluginId: plugin\.id/)
  assert.match(workflow, /return t\('settings\.agentSkills\.title'\)/)
  assert.doesNotMatch(workflow, /open_agent_skills_window|TRUSTED_PLUGIN_ID/)

  // The dock itself stays a plain surface placeholder; the trusted surface is a native view.
  assert.doesNotMatch(dock, /AgentSkills|trustedActive|trustedKey/)
  assert.doesNotMatch(dock, /https?:|file:|iframe|src=/)
})

test('the child page carries no window chrome and reports through a controlled event', () => {
  const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
  const view = read('../views/AgentSkills.vue')

  // A titlebar, a window control or window.close would close the Workflow window the surface is
  // docked into, so none of them may appear.
  assert.doesNotMatch(view, /Titlebar/)
  assert.doesNotMatch(view, /getCurrentWebviewWindow|appWindow|\.close\(\)/)
  assert.doesNotMatch(view, /window controls|traffic/)

  // The plugin decision fails closed through the shared filter, and an invalidation is reported
  // as one controlled event instead of a window close.
  assert.match(view, /agentSkillsVerified\(pluginStore\.inventory, !!pluginStore\.lastError\)/)
  assert.match(view, /emit\(AGENT_SKILLS_INVALIDATED_EVENT, \{\}\)/)
  assert.match(view, /const AGENT_SKILLS_INVALIDATED_EVENT = 'cs:\/\/agent-skills-invalidated'/)
  assert.match(view, /<AgentSkillsPanel v-if="pluginVerified" \/>/)
  // The listeners and the polling timer are always released, so no timer leaks.
  assert.match(view, /await listen\('cs:\/\/plugins-changed', refreshPluginState\)/)
  assert.match(view, /setInterval\(refreshPluginState, 5000\)/)
  assert.match(view, /clearInterval\(refreshTimer\)/)
  assert.match(view, /unlistenPlugins\?\.\(\)/)
})

test('the workflow window releases the tab from the controlled event', () => {
  const workflow = readFileSync(new URL('../views/Workflow.vue', import.meta.url), 'utf8')

  assert.match(workflow, /listen\('cs:\/\/agent-skills-invalidated', \(\) => \{\s*void closeAgentSkillsTab\(\)\s*\}\)/)
  assert.match(workflow, /const closeAgentSkillsTab = \(\) => \{\s*const tab = dockTabs\.value\.find\(item => item\.kind === AGENT_SKILLS_TAB_KIND\)/)
  assert.match(workflow, /unlistenAgentSkills\.value\?\.\(\)/)
  // The inventory watcher fails the trusted tab closed instead of by its entry.
  assert.match(workflow, /if \(tab\.kind === AGENT_SKILLS_TAB_KIND\) \{\s*if \(!allowed\.has\(AGENT_SKILLS_PLUGIN_ID\)\) \{\s*void closeDockTab\(tab\.id\)/)
})

test('the built-in plugin id is shared, so the desktop and the view agree', () => {
  assert.equal(AGENT_SKILLS_PLUGIN_ID, 'agent-skills')

  const command = readFileSync(
    new URL('../../src-tauri/src/commands/window.rs', import.meta.url),
    'utf8'
  )
  assert.match(command, /const AGENT_SKILLS_PLUGIN_ID: &str = "agent-skills";/)
})