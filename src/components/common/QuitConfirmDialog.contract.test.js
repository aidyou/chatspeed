import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { compileScript, compileTemplate, parse as parseSfc } from 'vue/compiler-sfc'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')
const dialog = read('./QuitConfirmDialog.vue')
const app = read('../../App.vue')
const composable = read('../../composables/useQuitConfirmation.js')
const exitCoordinator = read('../../../src-tauri/src/exit_coordinator.rs')
const runtimeClient = read('../../../src-tauri/src/runtime_client.rs')
const titlebar = read('../../components/window/Titlebar.vue')
const windowCommands = read('../../../src-tauri/src/commands/window.rs')

const locales = ['en', 'zh-Hans', 'zh-Hant'].map(name => ({
  name,
  messages: JSON.parse(read(`../../i18n/locales/${name}.json`))
}))

const shape = node =>
  node && typeof node === 'object' && !Array.isArray(node)
    ? Object.fromEntries(Object.keys(node).sort().map(key => [key, shape(node[key])]))
    : null

test('the dialog is a valid SFC whose script uses the quit composable', () => {
  const { descriptor, errors } = parseSfc(dialog, { filename: 'QuitConfirmDialog.vue' })
  assert.deepEqual(errors, [])
  assert.ok(descriptor.template, 'the dialog needs a template')
  assert.ok(descriptor.scriptSetup, 'the dialog needs a <script setup> block')

  const script = compileScript(descriptor, { id: 'quit-confirm' })
  assert.match(script.content, /useQuitConfirmation/)

  const template = compileTemplate({
    source: descriptor.template.content,
    filename: 'QuitConfirmDialog.vue',
    id: 'quit-confirm'
  })
  assert.deepEqual(template.errors, [])
})

test('the dialog follows the approved layout and the shared dialog pattern', () => {
  // Narrow dialog, reusing the existing Element Plus dialog pattern.
  assert.match(dialog, /width="480px"/)
  assert.match(dialog, /:close-on-click-modal="false"/)
  assert.match(dialog, /:append-to-body="true"/)
  // ESC is only allowed to close while no stop is in flight.
  assert.match(dialog, /:close-on-press-escape="!busy"/)
  // The checkbox is bound to the composable and never hardcoded to checked.
  assert.match(dialog, /v-model="stopRuntime"/)
  assert.doesNotMatch(dialog, /v-model="stopRuntime"[^>]*checked/)

  // The sections appear in the approved order: title, impact, checkbox, hint,
  // status/error, then the cancel/exit actions.
  const order = [
    'quitConfirm.title',
    'quitConfirm.impact',
    'v-model="stopRuntime"',
    'quitConfirm.stopRuntimeHint',
    'quitConfirm.stopping',
    'quitConfirm.errors.',
    'quitConfirm.cancel',
    'quitConfirm.exit'
  ]
  let cursor = -1
  for (const marker of order) {
    const index = dialog.indexOf(marker)
    assert.ok(index > cursor, `"${marker}" must follow the previous section`)
    cursor = index
  }
})

test('no user-visible text is hardcoded in the dialog', () => {
  // Only text nodes matter: a label rendered as a literal would never be
  // translated, while identifiers such as `handleCancel` are not user-visible.
  for (const literal of [
    '>Cancel<',
    '>Retry<',
    '>Quit<',
    '>Quit Desktop Only<',
    'Quit ChatSpeed',
    'Stopping the shared runtime'
  ]) {
    assert.ok(!dialog.includes(literal), `the dialog hardcodes "${literal}"`)
  }
  for (const key of [
    'quitConfirm.title',
    'quitConfirm.impact',
    'quitConfirm.stopRuntime',
    'quitConfirm.stopRuntimeHint',
    'quitConfirm.stopping',
    'quitConfirm.cancel',
    'quitConfirm.exit',
    'quitConfirm.retry',
    'quitConfirm.exitDesktopOnly'
  ]) {
    assert.ok(dialog.includes(`t('${key}')`), `the dialog is missing ${key}`)
  }
})

test('only the Workflow window mounts the quit dialog', () => {
  assert.match(app, /import QuitConfirmDialog from '@\/components\/common\/QuitConfirmDialog\.vue'/)
  assert.match(app, /const QUIT_HOST_WINDOW_LABEL = 'workflow'/)
  assert.match(
    app,
    /const isQuitHost = windowStore\.windowLabel === QUIT_HOST_WINDOW_LABEL && !isAgentSkillsChild/
  )
  assert.match(app, /<quit-confirm-dialog v-if="isQuitHost" \/>/)
})

test('every quit string exists in all three locales', () => {
  const shapes = locales.map(({ messages }) => shape(messages.quitConfirm))
  assert.deepEqual(shapes[1], shapes[0], 'zh-Hans must mirror the en keys')
  assert.deepEqual(shapes[2], shapes[0], 'zh-Hant must mirror the en keys')

  for (const { name, messages } of locales) {
    for (const key of Object.keys(shapes[0])) {
      if (key === 'errors') continue
      assert.equal(typeof messages.quitConfirm[key], 'string', `${name}.quitConfirm.${key}`)
    }
  }
})

test('the frontend phases and error codes mirror the Rust coordinator', () => {
  const phases = ['idle', 'confirming', 'stopping_runtime', 'exiting_desktop', 'committing_exit']
  for (const phase of phases) {
    assert.match(
      exitCoordinator,
      new RegExp(`QUIT_PHASE_[A-Z_]+: &str = "${phase}"`),
      `Rust is missing the ${phase} phase`
    )
    assert.match(composable, new RegExp(`'${phase}'`), `the composable is missing ${phase}`)
  }

  const codes = [
    'runtime_not_connected',
    'runtime_instance_changed',
    'runtime_endpoint_gone',
    'runtime_stop_timeout',
    'runtime_stop_failed'
  ]
  for (const code of codes) {
    assert.ok(
      exitCoordinator.includes(`"${code}"`) || runtimeClient.includes(`"${code}"`),
      `no Rust code publishes ${code}`
    )
    for (const { name, messages } of locales) {
      assert.equal(
        typeof messages.quitConfirm.errors[code],
        'string',
        `${name} is missing the ${code} message`
      )
    }
  }
})

test('window menu quit opens the shared confirmation instead of exiting directly', () => {
  assert.match(titlebar, /case 'quit':[\s\S]*?invoke\('quit_window'\)/)
  assert.match(windowCommands, /pub fn quit_window\([\s\S]*?request_quit\(&app\)/)
  assert.doesNotMatch(windowCommands, /pub fn quit_window\([\s\S]*?app\.exit\(0\)/)
})
test('Rust renders no user-visible quit prose', () => {
  // The coordinator and the supervisor only publish stable codes; every message
  // the user reads is localized on the frontend, so the Rust side never reaches
  // for the i18n bundle.
  assert.ok(!exitCoordinator.includes('rust_i18n'), 'the coordinator must not localize prose')
  assert.match(runtimeClient, /pub fn code\(self\) -> &'static str/)
})
