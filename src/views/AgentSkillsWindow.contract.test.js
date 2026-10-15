import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { compileStyleAsync, compileTemplate, parse as parseSfc } from 'vue/compiler-sfc'

import { ref } from 'vue'
import { agentSkillsVerified } from '../libs/agentSkillsView.js'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

const view = read('./AgentSkills.vue')
const router = read('../router/index.js')
const appVue = read('../App.vue')
const moduleRs = read('../../src-tauri/src/agent_skills/mod.rs')
const carrierRs = read('../../src-tauri/src/agent_skills/carrier.rs')
const viewRs = read('../../src-tauri/src/agent_skills/view.rs')
const windowRs = read('../../src-tauri/src/window.rs')
const commandRs = read('../../src-tauri/src/commands/window.rs')
const pluginCommandRs = read('../../src-tauri/src/commands/plugin.rs')
const libRs = read('../../src-tauri/src/lib.rs')
const capabilities = JSON.parse(read('../../src-tauri/capabilities/default.json'))
const tauriConf = JSON.parse(read('../../src-tauri/tauri.conf.json'))

const builtin = (id, state, verified) => ({
  id,
  kind: 'builtin',
  state,
  ui: { entry: `${id}/index.html`, verified }
})

test('the router still serves the trusted surface at /agent-skills for the child view', () => {
  assert.match(router, /path:\s*"\/agent-skills"/)
  assert.match(router, /import\("@\/views\/AgentSkills\.vue"\)/)
})

test('the surface renders the panel only, never window chrome', () => {
  assert.match(view, /from '@\/components\/setting\/AgentSkills\.vue'/)
  assert.match(view, /<AgentSkillsPanel v-if="pluginVerified" \/>/)
  // A titlebar or a window control would close the Workflow window this view is docked into.
  assert.doesNotMatch(view, /Titlebar|TitleBar/)
  assert.doesNotMatch(view, /getCurrentWebviewWindow|appWindow/)
  assert.doesNotMatch(view, /window controls/)
  assert.doesNotMatch(view, /[\u4e00-\u9fff]/, 'hardcoded CJK copy in the surface view')
})

test('the surface view compiles as a Vue SFC', async () => {
  const { descriptor, errors } = parseSfc(view)
  assert.deepEqual(errors, [], 'the SFC must parse cleanly')
  assert.ok(descriptor.scriptSetup, 'the surface must be a <script setup> component')
  assert.ok(descriptor.template, 'the surface must have a template')

  const template = compileTemplate({
    source: descriptor.template.content,
    filename: 'AgentSkills.vue',
    id: 'data-v-agent-skills',
    scoped: true
  })
  assert.deepEqual(template.errors, [], 'the template must compile')

  const style = descriptor.styles.find(entry => entry.lang === 'scss')
  assert.ok(style, 'the surface must style itself with scoped SCSS')
  const compiled = await compileStyleAsync({
    source: style.content,
    filename: 'AgentSkills.vue',
    id: 'data-v-agent-skills',
    scoped: true,
    preprocessLang: 'scss'
  })
  assert.deepEqual(compiled.errors, [], 'the SCSS must compile')
})

test('the surface fails closed and reports through its controlled event', async () => {
  const { descriptor } = parseSfc(view)
  let mounted
  let unmounted
  let eventHandler
  let unlistened = 0
  let cleared = 0
  const emitted = []
  const pluginStore = { inventory: null, lastError: null, loadInventory: async () => {} }

  const page = runInNewContext(
    descriptor.scriptSetup.content.replace(/^import .*\n/gm, '') +
      '\n;({ pluginVerified, refreshPluginState })',
    {
      ref,
      emit: async (name, payload) => {
        emitted.push({ name, payload })
      },
      listen: async (_, callback) => {
        eventHandler = callback
        return () => {
          unlistened += 1
        }
      },
      setInterval: () => 1,
      clearInterval: () => {
        cleared += 1
      },
      onMounted: callback => {
        mounted = callback
      },
      onUnmounted: callback => {
        unmounted = callback
      },
      usePluginStore: () => pluginStore,
      agentSkillsVerified,
      console
    }
  )

  assert.equal(page.pluginVerified.value, false, 'unconfirmed content must not mount')
  pluginStore.inventory = { schema_version: 1, plugins: [builtin('agent-skills', 'enabled', true)] }
  await mounted()
  assert.equal(page.pluginVerified.value, true)
  assert.equal(emitted.length, 0, 'a confirmed surface must not report')

  pluginStore.inventory = { schema_version: 1, plugins: [builtin('agent-skills', 'disabled', true)] }
  await eventHandler()
  assert.equal(page.pluginVerified.value, false)
  assert.equal(emitted.length, 1)
  assert.equal(emitted[0].name, 'cs://agent-skills-invalidated')
  assert.deepEqual(Object.keys(emitted[0].payload), [])
  await eventHandler()
  assert.equal(emitted.length, 1, 'an unchanged refusal must not repeat the event')

  unmounted()
  assert.equal(unlistened, 1)
  assert.equal(cleared, 1)
  await page.refreshPluginState()
  assert.equal(emitted.length, 1, 'an unmounted surface must not report again')
})

test('the surface imports the shared, fail-closed plugin decision', () => {
  assert.match(view, /import \{ agentSkillsVerified \} from '@\/libs\/agentSkillsView'/)
  assert.match(view, /agentSkillsVerified\(pluginStore\.inventory, !!pluginStore\.lastError\)/)
  assert.match(view, /listen\('cs:\/\/plugins-changed', refreshPluginState\)/)
  assert.match(view, /setInterval\(refreshPluginState, 5000\)/)
  assert.match(view, /clearInterval\(refreshTimer\)/)
})

test('the surface is a same-window child webview, never a second window', () => {
  // The carrier lives in its own module and uses the child webview API.
  assert.match(moduleRs, /mod carrier;/)
  assert.match(carrierRs, /add_child\(/)
  assert.match(carrierRs, /agent_skills_builder\(app\)/)
  assert.match(viewRs, /WebviewUrl::App\(AGENT_SKILLS_ROUTE\.into\(\)\)/)
  assert.match(viewRs, /pub const AGENT_SKILLS_WEBVIEW_LABEL: &str = "agent_skills";/)
  // The navigation rule is the application's own origin (the configured dev server or the bundled
  // scheme), never a bare scheme or any loopback port.
  assert.match(viewRs, /on_navigation\(move \|url\| is_agent_skills_origin\(url, &expected\)\)/)
  assert.match(viewRs, /app\.config\(\)\.build\.dev_url/)
  assert.match(viewRs, /fn production\(\) -> Self/)
  // A window builder or the removed standalone-window functions must never come back.
  assert.doesNotMatch(carrierRs, /WebviewWindowBuilder/)
  assert.doesNotMatch(viewRs, /WebviewWindowBuilder/)
  assert.doesNotMatch(windowRs, /create_agent_skills_window|open_or_focus_agent_skills_window/)
  assert.doesNotMatch(windowRs, /AGENT_SKILLS_WINDOW_LABEL/)
})

test('a half-created child webview is closed and an untracked page is released', () => {
  // Every failure after the webview exists closes it before detaching its holder, so a live
  // webview is never left behind with no handle to it.
  assert.match(carrierRs, /fn rollback\(app: &AppHandle<Wry>, webview: &Webview<Wry>, failure: String\)/)
  assert.match(carrierRs, /return Err\(rollback\(/)
  assert.match(carrierRs, /if let Err\(error\) = webview\.close\(\)/)
  // The original failure is reported even when the rollback itself fails.
  assert.match(carrierRs, /log::warn!\(/)
  // A holder is shown only once its webview is packed inside it.
  assert.match(carrierRs, /holder\.set_visible\(false\);/)
  assert.match(carrierRs, /holder\.set_visible\(true\);/)
  // A show that fails after creation releases the untracked page instead of leaking its label.
  assert.match(viewRs, /let _ = page\.destroy\(app\);/)
  // A close that fails keeps the tracked page so it can be retried instead of looking released.
  assert.match(viewRs, /if inner\.page\.is_none\(\)/)
  assert.match(viewRs, /if let Some\(page\) = inner\.page\.as_ref\(\)/)
})

test('only the trusted child drops the window frame and the draggable titlebar', () => {
  // The gate is the child webview's own label, read from the Tauri webview metadata.
  assert.match(appVue, /import \{ getCurrentWebview \} from '@tauri-apps\/api\/webview'/)
  assert.match(appVue, /const AGENT_SKILLS_CHILD_LABEL = 'agent_skills'/)
  assert.match(appVue, /getCurrentWebview\(\)\.label === AGENT_SKILLS_CHILD_LABEL/)
  // The global, window-dragging titlebar is rendered for every other webview and window only.
  assert.match(
    appVue,
    /<div\s+v-if="!isAgentSkillsChild"\s+class="titlebar"\s+data-tauri-drag-region/
  )
  // The child's container drops the radius, border and shadow the shared window already draws.
  assert.match(appVue, /'app-container--docked': isAgentSkillsChild/)
  const docked = appVue.slice(appVue.indexOf('&.app-container--docked'))
  assert.match(docked, /border-radius: 0;/)
  assert.match(docked, /border: 0;/)
  assert.match(docked, /box-shadow: none;/)
  // The rule wins over the per-OS frame rules, so it is declared after them.
  assert.ok(appVue.indexOf('&.app-container--docked') > appVue.lastIndexOf('&.windows'))
  // No new window control is added, and the shell still bootstraps every webview.
  assert.match(appVue, /await settingStore\.getEnv\(true\)/)
})

test('on Linux the child webview is moved into the bounded dock holder', () => {
  assert.match(carrierRs, /use gtk::prelude::\*;/)
  // The inner GTK webview is taken out of the window box and packed into the dock holder, whose
  // rectangle and frame band are what keep the surface inside the dock.
  assert.match(carrierRs, /container\.remove\(&inner\)/)
  assert.match(carrierRs, /container\(\)\.pack_start\(&inner, true, true, 0\)/)
  assert.match(carrierRs, /ViewShape::install\(&inner\)/)
  assert.match(carrierRs, /surface\.holder\(app, DockOwner::AgentSkills, bounds\)/)
  assert.match(carrierRs, /surface\.remove_owner\(app, DockOwner::AgentSkills\)/)
  assert.match(carrierRs, /with_webview\(/)
  // The child is closed before its holder is detached, so the window content is never disturbed.
  assert.match(carrierRs, /self\.webview\s*\n?\s*\.close\(\)/)
})

test('the backend shows the surface only for an enabled, verified built-in bundle', () => {
  assert.match(commandRs, /pub async fn show_agent_skills_view/)
  assert.match(commandRs, /pub async fn hide_agent_skills_view/)
  assert.match(commandRs, /pub async fn close_agent_skills_view/)
  // The gate is a runtime read taken under the plugin UI lock, so a mutation cannot race the open.
  assert.match(commandRs, /let _gate = runtime\.serialize\(\)\.await;/)
  assert.match(commandRs, /runtime_plugin::inventory\(/)
  assert.match(commandRs, /fn agent_skills_ui_verified\(inventory: &Value\) -> bool/)
  assert.match(commandRs, /PluginError::not_installed\(/)
  // The creation result travels back to the caller through the shared main-thread runner.
  assert.match(commandRs, /crate::agent_skills::run_on_view_thread\(&app, move \|view, app\|/)
  // Every command is called from the Workflow window only.
  assert.match(commandRs, /crate::commands::plugin::ensure_workflow_caller\(&window\)\?;/)

  assert.match(libRs, /app\.manage\(agent_skills::AgentSkillsView::new\(\)\);/)
  assert.match(libRs, /show_agent_skills_view,\s*hide_agent_skills_view,\s*close_agent_skills_view,/)
  assert.match(libRs, /agent_skills\.release\(\);/)
})

test('the child never closes the parent window when the plugin is invalidated', () => {
  // The cleanup releases the child view; it never calls `window.close()` on the Workflow window.
  const cleanup = pluginCommandRs.slice(
    pluginCommandRs.indexOf('async fn invalidate_plugin_ui'),
    pluginCommandRs.indexOf('fn broadcast_plugins_changed')
  )
  assert.match(cleanup, /crate::agent_skills::run_on_view_thread\(app, \|view, app\| view\.close\(app\)\)/)
  assert.doesNotMatch(cleanup, /get_webview_window|AGENT_SKILLS_WINDOW_LABEL/)
  assert.match(pluginCommandRs, /pub\(crate\) fn ensure_workflow_caller/)
})

test('the platform no longer declares an Agent Skills window', () => {
  // There is no `agent_skills` window entry, so no top-level window can be opened for the surface.
  assert.equal(
    (tauriConf.app.windows || []).some(window => window.label === 'agent_skills'),
    false
  )
  assert.equal(capabilities.windows.includes('agent_skills'), false)
  // The child lives in the Workflow window, which is already fully trusted.
  assert.ok(capabilities.windows.includes('workflow'))
})