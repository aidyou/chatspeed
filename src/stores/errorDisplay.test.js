import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import ts from 'typescript'
import {
  getWorkflowPersistedMessageId,
  inferWorkflowToolExecutionStatus,
  resolveWorkflowErrorDisplay,
  resolveWorkflowUserError
} from '../composables/workflow/messageProjectionRules.js'

import { formatUserError, setUserErrorFallback } from '../libs/error.js'

const fallback = 'Something went wrong. Please try again.'
setUserErrorFallback(() => fallback)

const read = file => readFileSync(new URL(`./${file}`, import.meta.url), 'utf8')

// The real store handler is extracted rather than copied, so the test fails if
// production code drifts back to a raw error conversion.
const extract = (file, name) => {
  const source = read(file)
  const match = source.match(new RegExp(`  const ${name} = [^\\n]+\\{[\\s\\S]*?\\n  \\};?`))
  assert.ok(match, `${file}: ${name} should remain an extractable handler`)
  return match[0]
}

const run = (file, name, context) => {
  const handler = extract(file, name)
  return runInNewContext(`${handler}\n${name}`, context)
}

const ref = value => ({ value })

class FrontendAppError extends Error {
  constructor(message, originalError) {
    super(message)
    this.name = 'FrontendAppError'
    this.module = 'Ai'
    this.kind = 'Upstream'
    this.originalError = originalError
    this.code = 'blocked'
    this.reason = 'runtime_stopping'
  }

  toFormattedString() {
    return `[${this.module}:${this.kind}] ${this.message}`
  }
}

const oldKeywords = 'session already exists; Session is stopping; No active session'

for (const [file, name] of [
  ['agent.js', '_handleError'],
  ['mcp.js', '_handleError']
]) {
  test(`${file} ${name} shows only the normalized message, preserves unrelated state and rethrows the exact error`, async () => {
    const logs = []
    const state = {
      loading: ref(true),
      error: ref(null),
      status: ref('idle'),
      FrontendAppError,
      formatUserError,
      console: { error: (...args) => logs.push(args) }
    }
    const handler = run(file, name, state)

    const originalError = new Error('raw transport detail')
    const error = new FrontendAppError(oldKeywords, originalError)
    await assert.rejects(Promise.resolve().then(() => handler(error)), thrown => thrown === error)

    assert.equal(state.error.value, oldKeywords)
    assert.equal(state.loading.value, false)
    assert.equal(state.status.value, 'idle', 'error wording must not drive other state')
    assert.equal(logs.length, 1)
    assert.ok(logs[0].some(arg => typeof arg === 'string' && arg.includes('Ai:Upstream')))
    assert.equal(logs[0].at(-1), originalError)

    const unknown = {}
    await assert.rejects(Promise.resolve().then(() => handler(unknown)), thrown => thrown === unknown)
    assert.equal(state.error.value, fallback)
    assert.doesNotMatch(state.error.value, /\[object Object\]/)
  })
}

test('workflow tool failures finalize safely without changing the raw result or typed status', () => {
  const updates = []
  const messages = []
  const raw = 'legacy tool failure: token=CANARY'
  const state = {
    currentTaskLedger: ref({ tools: new Map([['call-1', { toolName: 'read_file', errorDisplay: 'safe provider cause' }]]) }),
    taskLedgerEnabled: ref(true),
    formatUserError,
    resolveWorkflowUserError,
    getToolStatusSummary: (_name, _status, summary) => summary,
    upsertToolViewState: update => updates.push(update),
    patchToolMessage: (_id, patch) => messages.push(patch({ message: raw }, {}))
  }
  const finalize = run('workflow.js', 'finalizeToolExecution', state)
  finalize('call-1', false, raw, 'execution_error')
  assert.equal(updates[0].status, 'final_error')
  assert.equal(updates[0].result, raw)
  assert.equal(updates[0].summary, 'safe provider cause')
  assert.equal(updates[0].errorDisplay, 'safe provider cause')
  assert.equal(updates[0].errorDisplayCopy, 'safe provider cause')
  assert.equal(messages[0].message, raw)
  assert.equal(messages[0].metadata.execution_status, 'failed')
  assert.equal(messages[0].metadata.summary, 'safe provider cause')
  state.currentTaskLedger.value.tools.clear()
  finalize('call-2', false, raw, 'execution_error')
  assert.doesNotMatch(updates[1].summary, /CANARY/)
  finalize('call-3', true, 'Error: ordinary output', undefined)
  assert.equal(updates[2].status, 'final_success')
  assert.equal(updates[2].summary, 'Error: ordinary output')
})

// Compile the real pure history mapper while supplying only its presentation dependencies.
const mapperSource = read('../composables/workflow/useToolStateMapper.ts')
const mapperJs = ts.transpileModule(mapperSource, {
  compilerOptions: { module: ts.ModuleKind.CommonJS }
}).outputText
const displayDependencies = {
  formatDisplayPath: value => value,
  normalizeShellCommandForDisplay: value => value,
  normalizeToolDisplayText: value => value,
  getToolStatusSummary: (_name, _status, value) => value
}
const mapperExports = {}
runInNewContext(mapperJs, {
  exports: mapperExports,
  require: name => name.includes('messageProjectionRules')
    ? { resolveWorkflowErrorDisplay, getStructuredWorkflowToolName: message => message.metadata?.tool_name }
    : name.includes('toolApproval') ? { isAutoExecuteWorkflowTool: () => false } : displayDependencies
})

for (const [label, legacy, safe] of [
  ['typed origin', 'Execution failed: outer wrapper', 'socket closed'],
  ['security', 'Security violation: PRIVATE_POLICY_DETAIL', 'Safe refusal'],
  ['sandbox', 'Sandbox failure: PRIVATE_POLICY_DETAIL', 'Safe refusal']
]) {
  for (const seeded of [true, false]) {
    test(`tool_failed then message reconciles ${label}, seeded ledger=${seeded}`, () => {
      const ledgerMap = ref(new Map([['session', { tools: new Map() }]]))
      if (seeded) ledgerMap.value.get('session').tools.set('call', {
        toolCallId: 'call', toolName: 'custom_tool', status: 'approved_running'
      })
      const context = {
        ...displayDependencies,
        currentWorkflowId: ref('session'),
        currentTaskLedger: { get value() { return ledgerMap.value.get('session') } },
        taskLedgerMap: ledgerMap,
        taskLedgerEnabled: ref(true),
        messages: ref([]),
        messageWindowBeforeId: ref(null),
        formatUserError,
        resolveWorkflowUserError,
        resolveWorkflowErrorDisplay,
        getWorkflowPersistedMessageId,
        inferWorkflowToolExecutionStatus,
        hasStreamingOutput: () => false
      }
      for (const handler of ['normalizeWorkflowMessage', 'comparePersistedMessageOrder',
        'upsertToolViewState', 'patchToolMessage', 'inferLedgerToolStatus', 'getLedgerStatusSummary',
        'finalizeToolExecution', 'addMessage']) {
        context[handler] = run('workflow.js', handler, context)
      }
      const userError = { schema_version: 1, message: safe, reason: 'unknown' }
      const payload = { tool_call_id: 'call', error: legacy, error_type: label,
        error_details: { message: legacy, user_error: userError } }
      const core = read('../composables/workflow/useWorkflowCore.ts')
      const branch = core.split("} else if (payload.type === 'tool_failed') {")[1]
        .split("} else if (payload.type === 'task_completed')")[0]
      runInNewContext(branch, {
        payload, sessionId: 'session', workflowStore: {
          finalizeToolExecution: context.finalizeToolExecution,
          resolvePendingTool() {}, clearApprovalSubmission() {}
        }, markSessionLiveFromNonTerminalEvent() {}, clearPendingApprovalEntry() {}
      })
      const getTool = () => context.currentTaskLedger.value.tools.get('call')
      assert.equal(getTool().errorDisplay, safe, 'the first failure event must use its typed projection')
      assert.equal(getTool().summary, safe)
      assert.equal(getTool().result, legacy)
      const message = { id: 1, role: 'tool', message: legacy, isError: true,
        metadata: { tool_call_id: 'call', tool_name: 'custom_tool', execution_status: 'failed',
          approval_status: 'approved', summary: legacy, user_error: { ...userError, message: `${safe} final` } } }
      context.addMessage(message)
      assert.equal(getTool().errorDisplay, `${safe} final`, 'later durable message must replace an earlier display')
      assert.equal(getTool().errorDisplayCopy, `${safe} final`)
      assert.equal(getTool().summary, `${safe} final`)
      assert.equal(getTool().status, 'final_error')
      assert.equal(getTool().result, legacy)
      assert.equal(context.messages.value[0].message, legacy)
      const rebuilt = mapperExports.deriveToolViewState(context.messages.value, new Map(), 'session').get('call')
      assert.equal(rebuilt.errorDisplay, getTool().errorDisplay)
      assert.equal(rebuilt.summary, getTool().summary)

      const taskSource = read('../components/workflow/TaskLedger.vue')
      const taskDisplay = taskSource.match(/const getTaskErrorDisplay = task =>\n[^\n]+/)[0]
      assert.equal(runInNewContext(`${taskDisplay}\ngetTaskErrorDisplay`, { resolveWorkflowErrorDisplay })(getTool()), `${safe} final`)
      const panelSource = read('../components/workflow/StatusPanel.vue')
      const panelBlock = panelSource.slice(panelSource.indexOf('const recentOperations = computed('), panelSource.indexOf('const totalToolCalls = computed('))
      const panel = runInNewContext(`${panelBlock}\nrecentOperations`, {
        computed: fn => ({ get value() { return fn() } }), toolLedger: { get value() { return [getTool()] } },
        displayRoots: ref([]), getToolInfo: () => ({ shortName: 'Tool' }),
        ledgerStatusToPanelStatus: () => 'error', removeSystemReminder: value => value,
        normalizeToolDisplayText: value => value, resolveWorkflowErrorDisplay
      })
      assert.equal(panel.value[0].fullText, `${safe} final`)
      const cardSource = read('../components/workflow/WorkflowMessageList.vue')
      const cardBlock = cardSource.slice(cardSource.indexOf('const getErrorAlertContent ='), cardSource.indexOf('const getExplorationBatchSummary ='))
      const card = runInNewContext(`${cardBlock}\ngetErrorAlertContent`, { resolveWorkflowErrorDisplay })
      assert.equal(card(context.messages.value[0]), `${safe} final`)

      // A legacy event without a sidecar must still yield to the later typed message.
      context.finalizeToolExecution('legacy', false, legacy, label)
      context.addMessage({ ...message, id: 2, metadata: { ...message.metadata, tool_call_id: 'legacy' } })
      assert.equal(context.currentTaskLedger.value.tools.get('legacy').summary, `${safe} final`)
    })
  }
}

test('handleDownloadFailed presents object and presentation payloads without [object Object]', () => {
  const logs = []
  const state = {
    downloadError: ref('previous failure'),
    downloadProgress: ref(42),
    isCheckingForUpdates: ref(true),
    formatUserError,
    console: { error: (...args) => logs.push(args) }
  }
  const handleDownloadFailed = run('update.js', 'handleDownloadFailed', state)

  handleDownloadFailed('download interrupted')
  assert.equal(state.downloadError.value, 'download interrupted')
  assert.equal(state.downloadProgress.value, 0)
  assert.equal(state.isCheckingForUpdates.value, false)

  handleDownloadFailed({ error: { code: 'upstream', message: 'legacy text', presentation: {
    schema_version: 1,
    message: 'provider declined the download',
    reason: 'unknown'
  } } })
  assert.equal(state.downloadError.value, 'provider declined the download')
  assert.doesNotMatch(state.downloadError.value, /\[object Object\]/)

  handleDownloadFailed({})
  assert.equal(state.downloadError.value, fallback)
  assert.doesNotMatch(state.downloadError.value, /\[object Object\]/)
  assert.equal(logs.length, 3)
})

test('restartApp shows one safe notification and keeps the technical detail in the log', async () => {
  const notifications = []
  const logs = []
  const originalError = new Error('raw restart detail')
  const error = new FrontendAppError('restart refused', originalError)
  const state = {
    FrontendAppError,
    formatUserError,
    ElMessage: { error: message => notifications.push(message) },
    invokeWrapper: async () => {
      throw error
    },
    console: { log: () => {}, error: (...args) => logs.push(args) }
  }
  const restartApp = run('update.js', 'restartApp', state)

  await restartApp()
  assert.deepEqual(notifications, ['restart refused'])
  assert.equal(logs.length, 1)
  assert.equal(logs[0].at(-1), originalError)
})

test('moveWindowToScreenEdge shows one safe notification and no duplicated title', async () => {
  const notifications = []
  const logs = []
  const state = {
    FrontendAppError,
    formatUserError,
    windowLabel: 'main',
    t: (key, params) => `${key}: ${params.error}`,
    showMessage: (message, type) => notifications.push([message, type]),
    invokeWrapper: async () => {
      throw new Error(oldKeywords)
    },
    console: { error: (...args) => logs.push(args) }
  }
  const moveWindowToScreenEdge = run('window.js', 'moveWindowToScreenEdge', state)

  await moveWindowToScreenEdge('left')
  assert.deepEqual(notifications, [[`chat.errorOnMoveWindow: ${oldKeywords}`, 'error']])
  assert.equal(logs.length, 1)
  assert.doesNotMatch(notifications[0][0], /\[object Object\]/)
})

test('setSetting rethrows the shortcut failure itself instead of a wrapped duplicate', async () => {
  const logs = []
  const notifications = []
  const settings = ref({ theme: 'system' })
  const originalError = new Error('shortcut rejected')
  const state = {
    FrontendAppError,
    formatUserError,
    invokeWrapper: async () => {
      throw originalError
    },
    normalizeInterfaceLocale: value => value,
    camelToSnake: value => value,
    settings,
    sendSyncState: () => {},
    windowLabel: 'main',
    showMessage: (...args) => notifications.push(args),
    console: { error: (...args) => logs.push(args) }
  }
  const setSetting = run('setting.js', 'setSetting', state)

  await assert.rejects(setSetting('mainWindowVisibleShortcut', 'Control+J'), thrown => thrown === originalError)
  assert.equal(settings.value.theme, 'system')
  assert.deepEqual(notifications, [])
  assert.equal(logs.length, 1)
})

test('workflowAutomation error fields stay safe while the original failure is rethrown', async () => {
  const logs = []
  const automations = ref([{ id: 'kept', title: 'Kept' }])
  const selectedAutomationId = ref('kept')
  const state = {
    formatUserError,
    loading: ref(true),
    error: ref(null),
    automations,
    selectedAutomationId,
    normalizeAutomation: item => item,
    invokeWrapper: async () => {
      throw new Error(oldKeywords)
    },
    console: { error: (...args) => logs.push(args) }
  }
  const fetchAutomations = run('workflowAutomation.js', 'fetchAutomations', state)
  const error = new Error(oldKeywords)

  await assert.rejects(fetchAutomations(), thrown => thrown.message === oldKeywords)
  assert.equal(state.error.value, oldKeywords)
  assert.equal(state.loading.value, false)
  assert.deepEqual(automations.value, [{ id: 'kept', title: 'Kept' }])
  assert.equal(selectedAutomationId.value, 'kept')
  assert.equal(logs.length, 0, 'the store must not add notification logs')

  const failure = { module: 'Workflow', details: { message: 'automation unavailable' } }
  state.invokeWrapper = async () => {
    throw failure
  }
  await assert.rejects(fetchAutomations(), thrown => thrown === failure)
  assert.equal(state.error.value, 'automation unavailable')
  assert.deepEqual(automations.value, [{ id: 'kept', title: 'Kept' }])
  assert.equal(selectedAutomationId.value, 'kept')
})

test('sensitive status keeps the fixed retry sequence and formats the retained error', async () => {
  const logs = []
  let invokes = 0
  const status = ref({ healthy: true, error: null })
  const state = {
    formatUserError,
    status,
    invoke: async () => {
      invokes++
      throw { module: 'Sensitive', details: { kind: 'IoError', message: oldKeywords } }
    },
    setTimeout: callback => callback(),
    console: { error: (...args) => logs.push(args) }
  }
  const fetchStatus = run('sensitiveStore.js', 'fetchStatus', state)

  assert.equal(await fetchStatus(), false)
  assert.equal(invokes, 6, 'the existing five retries plus the first attempt must stay unchanged')
  assert.equal(status.value.healthy, false)
  assert.equal(status.value.error, oldKeywords)
  assert.equal(logs.length, 6)
})

test('assigned stores use the shared formatter for user-facing fields only', () => {
  for (const file of ['agent.js', 'mcp.js', 'update.js', 'window.js', 'workflowAutomation.js', 'sensitiveStore.js']) {
    assert.match(read(file), /import \{ formatUserError \} from '@\/libs\/error'/, file)
  }

  assert.doesNotMatch(read('agent.js'), /error\.value = err\.(?:toFormattedString|message)/)
  assert.doesNotMatch(read('mcp.js'), /error\.value = err\.(?:toFormattedString|message)/)
  assert.doesNotMatch(read('update.js'), /ElMessage\.error\([^\n]*(?:toFormattedString|String\()/)
  assert.doesNotMatch(read('window.js'), /showMessage\([^\n]*(?:toFormattedString|String\()/)
  assert.doesNotMatch(read('workflowAutomation.js'), /error\.value = err\.message \|\| String\(err\)/)
  assert.doesNotMatch(read('sensitiveStore.js'), /error: String\(error\)/)
  assert.doesNotMatch(read('setting.js'), /new Error\(i18n\.global\.t\('settings\.general\.updateShortcutFailed'/)

  const main = readFileSync(new URL('../main.js', import.meta.url), 'utf8')
  assert.match(main, /setUserErrorFallback\(\(\) => i18n\.global\.t\('common\.errorFallback'\)\)/)
  assert.ok(
    main.indexOf('setUserErrorFallback(') < main.indexOf('createApp(App)'),
    'the locale-aware fallback must be installed before the app is created'
  )
})