import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
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

test('workflow terminal is exposed from the navigation rail and the collapsed compact rail', async () => {
  const [sidebar, workflow] = await Promise.all([
    read('src/components/workflow/WorkflowSidebar.vue'),
    read('src/views/Workflow.vue')
  ])
  assert.match(sidebar, /class="workflow-terminal-entry compact-terminal-entry"/)
  assert.match(sidebar, /@click="\$emit\('open-terminal'\)"/)
  assert.doesNotMatch(sidebar, /expanded-terminal-entry|terminal-minimized=\(/)
  assert.match(workflow, /class="workflow-side-rail__item workflow-side-rail__terminal"/)
  assert.match(workflow, /:terminal-minimized="terminal\.hasSessions && !terminal\.visible"/)
  assert.match(workflow, /@open-terminal="terminal\.open"/)
  assert.match(workflow, /@click="terminal\.open"/)
  assert.match(workflow, /<WorkflowSidebar\s+:workflows="filteredWorkflows"/)
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
  const [panel, composable, general, env, environment, workflow] = await Promise.all([
    read('src/components/workflow/TerminalPanel.vue'),
    read('src/composables/workflow/useTerminal.ts'),
    read('src/components/setting/General.vue'),
    read('src-tauri/src/commands/env.rs'),
    read('src-tauri/src/environment.rs'),
    read('src/views/Workflow.vue')
  ])

  assert.match(general, /get_available_terminal_shells/)
  assert.match(general, /v-for="shell in terminalShells"/)
  assert.doesNotMatch(general, /<el-option label="PowerShell"/)
  assert.match(env, /get_available_shells/)
  // `/bin/bash` and `/usr/bin/bash` are one shell on merged-usr systems, so candidates are deduped
  // on the resolved executable instead of the spelling of the path.
  assert.match(environment, /fn dedupe_shells/)
  assert.match(environment, /std::fs::canonicalize\(&self\.path\)/)
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
  assert.match(panel, /host\.addEventListener\('keydown', onKeyDown, true\)[\s\S]*instance\.open\(host\)/)
  assert.match(panel, /event\.stopImmediatePropagation\(\)/)
  assert.match(panel, /host\.addEventListener\('keydown', onKeyDown, true\)[\s\S]*instance\.open\(host\)/)
  assert.match(panel, /event\.stopImmediatePropagation\(\)/)
  assert.match(panel, /host\.removeEventListener\('keydown', onKeyDown, true\)/)
  assert.match(panel, /host\.addEventListener\('keyup', onKeyUp, true\)/)
  assert.match(panel, /host\.removeEventListener\('keyup', onKeyUp, true\)/)
  assert.match(panel, /commandModifierDown/)
  assert.match(panel, /matchesTerminalShortcut\(event, props\.preferences\.toggleShortcut, commandModifierDown\)/)
  assert.match(panel, /matchesTerminalShortcut\(event, props\.preferences\.clearShortcut, commandModifierDown\)/)
  // Preferences must be read through the props: destructuring them into a plain const froze the
  // values captured at mount, so later setting changes never reached the mounted terminal.
  assert.doesNotMatch(panel, /const preferences = props\.preferences/)
  assert.match(panel, /props\.preferences\.colorScheme/)
  assert.match(panel, /props\.preferences\.usesCommandKey/)
  assert.match(panel, /props\.preferences\.outputLineLimit/)
  // ghostty-web bakes the output limit and the colour palette into a terminal when it is created, so
  // both preferences rebuild the mounted instances instead of patching a live canvas.
  assert.match(panel, /watch\(\[terminalTheme, \(\) => props\.preferences\.outputLineLimit\]/)
  assert.match(
    panel,
    /mountedScrollback !== configuredScrollback\(\) \|\| mountedTheme !== terminalTheme\.value/
  )
  assert.match(panel, /mountedInstancesAreStale\(\)/)
  assert.match(panel, /rebuildMountedInstances\(\)/)
  assert.doesNotMatch(panel, /options\.theme = /)
  assert.match(composable, /outputBuffers\.get\(sessionId\) \?\? outputHistory\.get\(sessionId\)/)
  assert.match(general, /setSetting\(shortcutKey, defaultShortcutMap\[shortcutKey\] \|\| null\)/)
  assert.match(panel, /terminalBlockTopRow/)
  assert.match(panel, /terminalClearSequence/)
  assert.match(composable, /const retained = writers\.get\(sessionId\)\?\.clear\(\)/)
  assert.match(
    composable,
    /outputBuffers\.set\(sessionId, \{ chunks: \[history\], lines: countLines\(history\) \}\)/
  )
  assert.match(
    composable,
    /outputHistory\.set\(sessionId, \{ chunks: \[history\], lines: countLines\(history\) \}\)/
  )
  assert.match(composable, /new TextEncoder\(\)\.encode\(`\$\{tab\?\.cwd \|\| ''\} > `\)/)
  // Clearing happens in the emulator: writing Ctrl+L to the PTY disturbed running programs and left
  // the erased rows in the scrollback, which is the behaviour this contract now forbids.
  assert.match(panel, /instance\.write\(terminalClearSequence/)
  assert.doesNotMatch(panel, /instance\.clear\(\)/)
  assert.doesNotMatch(panel, /attachCustomKeyEventHandler/)
  assert.doesNotMatch(composable, /\\u000c/)
  assert.doesNotMatch(composable, /void write\(sessionId, 'clear\\n'\)/)
  assert.match(panel, /\.workflow-terminal__content \{[^}]*padding: var\(--cs-space-sm\);[^}]*box-sizing: border-box/s)
  assert.match(panel, /\.workflow-terminal__content \{[^}]*caret-color: transparent;/s)
  assert.match(panel, /\.workflow-terminal__content \{[^}]*background: var\(--workflow-terminal-background\)[^}]*\}/s)
  assert.match(panel, /\.workflow-terminal__content :deep\(canvas\) \{\s*display: block;\s*\}/s)
  assert.doesNotMatch(panel, /pendingCarriageReturn/)
  assert.doesNotMatch(panel, /requestAnimationFrame\(flushOutput\)/)
  assert.match(panel, /UrlRegexProvider/)
  assert.match(panel, /openUrl\(link\.text\)/)
  assert.doesNotMatch(panel, /event\.isComposing \|\| event\.key === 'Process' \|\| event\.keyCode === 229/)
  assert.match(panel, /closeConfirmMessage/)
  assert.match(panel, /@command="confirmShellSwitch"/)
  assert.match(panel, /switchShellConfirmMessage/)
  assert.match(panel, /await terminal\.restartWithShell\(shellPath\)/)
  assert.match(workflow, /terminalClearShortcut/)
  assert.match(workflow, /commandOrControlPressed/)
  assert.match(workflow, /<TerminalPanel :terminal="terminal" :preferences="terminalPreferences" \/>/)
  assert.match(workflow, /matchesLocalShortcut/)
})

test('ghostty fit uses all available width because its scrollbar is drawn inside the canvas', async () => {
  const patch = await read('src/patches/ghostty-web@0.4.0.patch')
  assert.match(patch, /\+    const k = s - i - w, M = N - I - D/)
  assert.match(patch, /-    const k = s - i - w - gA, M = N - I - D/)
})

test('ghostty keeps input-method keys out of its own key encoder', async () => {
  const patch = await read('src/patches/ghostty-web@0.4.0.patch')
  // WebKitGTK reports the first key of an input-method session as Process/Unidentified without
  // keyCode 229. Encoding that key swallowed the letter and cancelled the browser insertion, so the
  // character never reached the PTY.
  assert.match(patch, /A\.keyCode === 229/)
  assert.match(patch, /A\.key === "Process"/)
  assert.match(patch, /A\.key === "Unidentified"/)
  // Text insertions are forwarded when the encoder did not already deliver them: some input methods
  // report the insertion without any keydown at all, which must still reach the PTY exactly once.
  assert.match(patch, /this\.lastKeyDownData = A\.key\.length === 1/)
  assert.match(patch, /C\.lastKeyDownData === E\.data && Date\.now\(\)/)
  assert.match(patch, /C\.imeSkippedKeydown \|\| E\.inputType === "insertReplacementText" \|\| !I/)
})

test('the ghostty patch stays in sync with the lockfile and its own hunk counters', async () => {
  const [patch, lockfile] = await Promise.all([
    read('src/patches/ghostty-web@0.4.0.patch'),
    read('pnpm-lock.yaml')
  ])

  // pnpm refuses to install when the recorded hash no longer matches the patch file.
  const hash = createHash('sha256').update(patch).digest('hex')
  assert.match(lockfile, new RegExp(`ghostty-web@0\\.4\\.0:\\n\\s+hash: ${hash}`))

  // A wrong counter makes the patch unapplicable, and the built file is not readable from here.
  let hunk = null
  const counters = []
  for (const line of patch.split('\n')) {
    const header = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line)
    if (header) {
      hunk = { old: Number(header[2] ?? 1), next: Number(header[4] ?? 1), oldSeen: 0, nextSeen: 0 }
      counters.push(hunk)
      continue
    }
    if (!hunk) continue
    if (line.startsWith('+')) hunk.nextSeen += 1
    else if (line.startsWith('-')) hunk.oldSeen += 1
    else if (line.startsWith(' ')) {
      hunk.oldSeen += 1
      hunk.nextSeen += 1
    }
  }
  assert.equal(counters.length, 5)
  for (const hunk of counters) {
    assert.equal(hunk.oldSeen, hunk.old)
    assert.equal(hunk.nextSeen, hunk.next)
  }
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
