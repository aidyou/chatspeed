import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import test from 'node:test'

const read = path => readFile(new URL(`../../../${path}`, import.meta.url), 'utf8')

test('workflow terminal stays isolated from the workflow runtime and is workflow-window gated', async () => {
  const [manager, commands, runtime] = await Promise.all([
    read('src-tauri/src/terminal.rs'),
    read('src-tauri/src/commands/terminal.rs'),
    read('src-tauri/src/workflow/react/manager.rs')
  ])

  assert.match(manager, /struct TerminalManager/)
  assert.match(manager, /abort_terminal_session/)
  assert.match(manager, /reader I\/O error or output-event delivery failure/)
  assert.match(manager, /direct user[\s\S]*not AI shell-tool executions/)
  assert.match(commands, /ensure_workflow_window/)
  assert.match(commands, /terminal_workflow_window_required/)
  assert.match(runtime, /struct WorkflowManager/)
})

test('workflow sidebar presents the terminal entry in expanded and compact modes', async () => {
  const sidebar = await read('src/components/workflow/WorkflowSidebar.vue')
  assert.match(sidebar, /compact-terminal-entry/)
  assert.match(sidebar, /expanded-terminal-entry/)
  assert.match(sidebar, /name="bash"/)
  assert.match(sidebar, /terminalMinimized/)
  assert.match(sidebar, /open-terminal/)
})

test('terminal panel exposes independent tab and lifecycle controls', async () => {
  const [panel, composable] = await Promise.all([
    read('src/components/workflow/TerminalPanel.vue'),
    read('src/composables/workflow/useTerminal.ts')
  ])

  for (const icon of ['add', 'minimize', 'fullscreen', 'fullscreen-off', 'caret-down', 'close']) {
    assert.match(panel, new RegExp(`['\"]${icon}['\"]`))
  }
  assert.match(composable, /terminal:\/\/output/)
  assert.match(composable, /terminal:\/\/reset/)
  assert.match(composable, /terminal_close/)
})

test('terminal panel keeps one xterm instance per tab across view transitions', async () => {
  const panel = await read('src/components/workflow/TerminalPanel.vue')
  assert.match(panel, /const instances = new Map/)
  assert.match(panel, /v-show="terminal\.visible"/)
  assert.match(panel, /v-show="tab\.sessionId === terminal\.activeSessionId"/)
  assert.match(panel, /if \(instances\.has\(tab\.sessionId\)\) return/)
  assert.match(panel, /disposeTab\(sessionId\)/)
  assert.doesNotMatch(panel, /mountActiveTerminal/)
})

test('shell switching is transactional and OSC 7 preserves Windows drive paths', async () => {
  const [panel, composable, manager] = await Promise.all([
    read('src/components/workflow/TerminalPanel.vue'),
    read('src/composables/workflow/useTerminal.ts'),
    read('src-tauri/src/terminal.rs')
  ])

  assert.match(panel, /\$\{tab\.cwd\.split\(\/\[\\\\\/\]\/\)\.filter\(Boolean\)\.pop\(\) \|\| tab\.cwd\} - \$\{tab\.shellName\}/)
  assert.match(composable, /const replacement = toTab\(await invokeWrapper\('terminal_create'/)
  assert.match(composable, /cwd: tab\.cwd, shellPath/)
  assert.match(composable, /await invokeWrapper\('terminal_close', \{ sessionId: tab\.sessionId \}\)/)
  assert.match(composable, /await invokeWrapper\('terminal_close', \{ sessionId: replacement\.sessionId \}\)\.catch/)
  assert.match(panel, /const cwdFromOsc7/)
  assert.match(panel, /pathname\.slice\(1\)\.replaceAll\('\/',/)
  assert.match(manager, /CurrentFileSystemLocation\.ProviderPath/)
  assert.match(manager, /\[char\]27/)
})

test('terminal preferences bound output, preserve terminal input, and use detected shell choices', async () => {
  const [panel, composable, general, env, workflow] = await Promise.all([
    read('src/components/workflow/TerminalPanel.vue'),
    read('src/composables/workflow/useTerminal.ts'),
    read('src/components/setting/General.vue'),
    read('src-tauri/src/commands/env.rs'),
    read('src/views/Workflow.vue')
  ])

  assert.match(general, /get_available_terminal_shells/)
  assert.match(general, /v-for="shell in terminalShells"/)
  assert.doesNotMatch(general, /<el-option label="PowerShell"/)
  assert.match(env, /get_available_shells/)
  assert.match(composable, /keepTrailingLines/)
  assert.match(composable, /TERMINAL_OUTPUT_STORAGE_KEY/)
  assert.match(composable, /TERMINAL_PANEL_STORAGE_KEY/)
  assert.match(composable, /restorePersistedOutput/)
  assert.match(composable, /persistPanelState/)
  assert.match(composable, /pagehide/)
  assert.match(composable, /MAX_PERSISTED_BYTES_PER_TAB/)
  assert.match(composable, /terminal:\/\/exit[\s\S]*removeTab/)
  assert.match(composable, /inputQueues/)
  assert.match(composable, /const pending = pendingInput\.get\(sessionId\)/)
  assert.match(composable, /const input = queue\?\.shift\(\)/)
  assert.match(composable, /await invokeWrapper\('terminal_write', \{ sessionId, input \}\)/)
  assert.match(composable, /pendingInput\.delete\(sessionId\)/)
  assert.match(composable, /if \(inputQueues\.has\(sessionId\)\) void queueWrite\(sessionId\)/)
  assert.match(composable, /queue\.push\(input\)/)
  assert.doesNotMatch(composable, /inputBuffers/)
  assert.doesNotMatch(composable, /inputFlushTimers/)
  assert.doesNotMatch(composable, /flushInput/)
  assert.match(composable, /outputLineLimit/)
  assert.match(panel, /scrollback:/)
  assert.match(panel, /overviewRuler: \{ width: 10 \}/)
  assert.match(panel, /instance\.onData/)
  assert.match(panel, /convertEol: false/)
  assert.match(panel, /outputQueue/)
  assert.match(panel, /pendingProgressChunk/)
  assert.match(panel, /mayBeSplitProgressLine/)
  assert.match(panel, /window\.setTimeout\(flushPendingProgressChunk, 8\)/)
  assert.match(panel, /joinOutput\(pendingProgressChunk, data\)/)
  assert.match(panel, /instance\.write\(output, \(\) =>/)
  assert.match(panel, /enqueueOutput/)
  assert.match(panel, /clearOutputQueue/)
  assert.match(panel, /clearPendingProgress/)
  assert.match(panel, /\.workflow-terminal__content \{[^}]*padding: 0[^}]*box-sizing: border-box/s)
  assert.match(panel, /'--workflow-terminal-background': terminalTheme\.background/)
  assert.match(panel, /\.xterm-viewport\) \{[^}]*background-color: var\(--workflow-terminal-background\)/s)
  assert.doesNotMatch(panel, /\.xterm-scrollable-element\s*\{[^}]*padding:/s)
  assert.doesNotMatch(panel, /pendingCarriageReturn/)
  assert.doesNotMatch(panel, /requestAnimationFrame\(flushOutput\)/)
  assert.match(panel, /attachCustomKeyEventHandler/)
  assert.match(panel, /event\.isComposing \|\| event\.key === 'Process' \|\| event\.keyCode === 229/)
  assert.match(panel, /matchesTerminalShortcut/)
  assert.match(panel, /closeConfirmMessage/)
  assert.match(panel, /@command="confirmShellSwitch"/)
  assert.match(panel, /switchShellConfirmMessage/)
  assert.match(panel, /await terminal\.restartWithShell\(shellPath\)/)
  assert.match(workflow, /terminalClearShortcut/)
  assert.match(workflow, /commandOrControlPressed/)
  assert.match(workflow, /<TerminalPanel :terminal="terminal" :preferences="terminalPreferences" \/>/)
  assert.match(workflow, /matchesLocalShortcut/)
})

test('every shipped locale contains the terminal label and toolbar strings', async () => {
  const shippedLocales = (await readdir('src/i18n/locales'))
    .filter(file => file.endsWith('.json'))
    .map(file => file.replace(/\.json$/, ''))
    .sort()

  assert.deepEqual(shippedLocales, ['en', 'zh-Hans', 'zh-Hant'])

  for (const locale of shippedLocales) {
    const content = await read(`src/i18n/locales/${locale}.json`)
    assert.match(content, /"terminal"\s*:\s*\{/)
    assert.match(content, /"title"\s*:/)
    assert.match(content, /"fullscreen"\s*:/)
    assert.match(content, /"terminalSettings"\s*:/)
    assert.match(content, /"terminalClearShortcut"\s*:/)
    assert.match(content, /"closeConfirmTitle"\s*:/)
    assert.match(content, /"switchShellConfirmTitle"\s*:/)
    assert.match(content, /"switchShellConfirmMessage"\s*:/)
  }
})
