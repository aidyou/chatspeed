import assert from 'node:assert/strict'

import {
  formatWorkflowRetryMessage,
  mergeWorkflowMessagePages,
  normalizeWorkflowErrorAlertContent,
  resolveWorkflowErrorDisplay,
  getWorkflowAlertIdentity,
  hasVisibleWorkflowText,
  reconcileWorkflowTailAlert,
  resolveWorkflowTailAlert,
  projectWorkflowMessageList,
  isWorkflowCompletionMessage,
  isWorkflowMessagePendingApproval
} from './messageProjectionRules.js'
import { deriveInlinePendingApprovals } from '../../stores/workflowApprovalRecovery.js'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { ref, nextTick, watch } from 'vue'
import { useWorkflowMessageScroll } from './useWorkflowMessageScroll.js'

test('approval expansion follows the bottom without observer re-registration or stealing history scroll', async () => {
  const source = await readFile(new URL('../../components/workflow/WorkflowMessageList.vue', import.meta.url), 'utf8')
  const start = source.indexOf('// Approval membership can expand')
  const end = source.indexOf('watch(\n  streamingLayoutState', start)
  for (const reading of [false, true]) {
    const container = { scrollTop: 600, scrollHeight: 1000, clientHeight: 400,
      querySelectorAll: () => [], getBoundingClientRect: () => ({ top: 0 }) }
    const controller = useWorkflowMessageScroll({ containerRef: ref(container) })
    if (reading) {
      controller.onWheel({ deltaY: -100 })
      container.scrollTop = 200
      controller.onScroll()
    }
    const ids = ref([])
    let stop
    runInNewContext(source.slice(start, end), {
      props: { get pendingApprovalIds() { return ids.value } },
      watch: (...args) => { stop = watch(...args) },
      nextTick,
      scrollController: controller,
      syncMessageContentResizeObserver: () => {}
    })
    ids.value = ['approval-1']
    await nextTick()
    // Vue patches the expanded card after the pre-flush snapshot.
    container.scrollHeight = 1400
    await new Promise(resolve => setTimeout(resolve, 30))
    assert.equal(controller.mode.value, reading ? 'reading' : 'following')
    assert.equal(container.scrollTop, reading ? 600 : 1000)
    stop()
    controller.dispose()
  }
})

test('message resize observation refreshes layout notifications after content changes', async () => {
  const source = await readFile(new URL('../../components/workflow/WorkflowMessageList.vue', import.meta.url), 'utf8')
  const start = source.indexOf('function syncMessageContentResizeObserver()')
  const end = source.indexOf('\nonMounted(', start)
  const first = {}
  const second = {}
  const container = { children: [first] }
  const calls = []
  const sync = runInNewContext(`${source.slice(start, end)}; syncMessageContentResizeObserver`, {
    messagesRef: { value: container },
    messageContentResizeObserver: {
      observe: child => calls.push(['observe', child]),
      disconnect: () => calls.push(['disconnect'])
    }
  })
  sync()
  sync()
  assert.deepEqual(calls, [
    ['disconnect'], ['observe', first], ['disconnect'], ['observe', first]
  ], 'existing cards must receive fresh initial size notifications after layout changes')
  calls.length = 0
  container.children = [second]
  sync()
  assert.deepEqual(calls, [['disconnect'], ['observe', second]])
})

const sessionId = 'session-approval-contract'
const toolCallId = 'tool_571ae521'
const command =
  'sqlite3 workflow.db "SELECT InvalidFinishSummary, FinishTask FROM workflow_messages"'
const approvalWaitingStatuses = ['awaiting_approval', 'awaiting_auto_approval']
const executionContext = {
  wait_reason: 'approval',
  pending_tools: [
    {
      tool_call_id: toolCallId,
      tool_name: 'bash',
      arguments: { command },
      details: { command },
      display_type: 'text'
    }
  ]
}
const pendingMessage = {
  id: 63713,
  sessionId,
  role: 'tool',
  message: command,
  metadata: {
    tool_call_id: toolCallId,
    tool_name: 'bash',
    tool_call: {
      id: toolCallId,
      function: {
        name: 'bash',
        arguments: { command }
      }
    },
    details: { command },
    approval_status: 'pending',
    execution_status: 'pending_approval'
  },
  toolDisplay: {
    action: command,
    displayType: 'text'
  }
}

const pendingApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext,
  messages: [pendingMessage],
  approvalWaitingStatuses
})
const pendingIds = pendingApprovals.map(approval => approval.toolCallId)

assert.equal(pendingApprovals.length, 1, 'the Bash request must produce one pending approval')
assert.equal(pendingApprovals[0].toolName, 'bash', 'the pending item must retain Bash identity')
assert.equal(
  isWorkflowCompletionMessage(pendingMessage),
  false,
  'command text containing completion markers must not select the completion presentation'
)
assert.equal(
  isWorkflowMessagePendingApproval(pendingMessage, pendingIds),
  true,
  'the pending Bash message must render its inline approval controls'
)

const resolvedMessage = {
  ...pendingMessage,
  id: 63714,
  metadata: {
    ...pendingMessage.metadata,
    approval_status: 'approved',
    execution_status: 'completed'
  }
}
const resolvedApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext,
  messages: [pendingMessage, resolvedMessage],
  approvalWaitingStatuses
})

assert.equal(resolvedApprovals.length, 0, 'the latest resolved state must remove the pending item')
assert.equal(
  isWorkflowMessagePendingApproval(pendingMessage, resolvedApprovals.map(approval => approval.toolCallId)),
  false,
  'a stale pending transcript row must not keep approval controls visible after resolution'
)

const submittedApprovalIds = new Set([toolCallId])
const submittedApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext: {
    ...executionContext,
    pending_tools: [
      ...executionContext.pending_tools,
      { tool_call_id: 'tool_still_pending', tool_name: 'write_file' }
    ]
  },
  messages: [pendingMessage],
  submittedToolIds: submittedApprovalIds,
  approvalWaitingStatuses
})
assert.deepEqual(
  submittedApprovals.map(approval => approval.toolCallId),
  ['tool_still_pending'],
  'locally submitted child approvals must leave only the remaining structured pending tools'
)

test('workflow error branches use structured reasons and localized presentation', async () => {
  const [approval, core, model] = await Promise.all([
    readFile('src/composables/workflow/useWorkflowApproval.ts', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
    readFile('src/components/setting/Model.vue', 'utf8')
  ])

  assert.match(approval, /normalizedError\.reason === USER_ERROR_REASONS\.NO_ACTIVE_SESSION/)
  assert.doesNotMatch(approval, /No sender|No active session|Session interrupted/)
  assert.match(core, /normalizeUserError\(error\)\.reason/)
  assert.doesNotMatch(core, /errorText\.includes\(['"]Session (?:is stopping|already exists)/)
  assert.match(model, /normalizedError\.reason === USER_ERROR_REASONS\.UNSUPPORTED_LIST_MODELS/)
  assert.doesNotMatch(model, /Method Not Allowed|status_code\\\":405|not available for Coding Agents/)
})
test('workflow retry status has a single footer alert', async () => {
  const [messageList, notifier, ...locales] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/components/workflow/StatusNotifier.vue', 'utf8'),
    ...['en', 'zh-Hans', 'zh-Hant'].map(locale =>
      readFile(`src/i18n/locales/${locale}.json`, 'utf8').then(JSON.parse)
    )
  ])
  const footerCall = messageList.match(/formatWorkflowRetryMessage\(tailAlert\.retryInfo, \$t\)/)?.[0]
  assert.ok(footerCall, 'the tail alert must call the shared formatter')
  assert.equal(
    messageList.match(/formatWorkflowRetryMessage\(/g)?.length,
    1,
    'retry text must be rendered by exactly one footer alert'
  )
  assert.match(messageList, /import \{[^}]*\bformatWorkflowRetryMessage\b[^}]*\} from '@\/composables\/workflow\/messageProjectionRules'/)
  assert.doesNotMatch(messageList, /retry-status-alert|chatState\.retryInfo/)
  assert.doesNotMatch(notifier, /formatWorkflowRetryMessage|retryInfo|hasActiveRetry/)
  for (const source of [messageList, notifier]) {
    assert.doesNotMatch(source, /(?:\$t|\bt)\('workflow\.retrying(?:Classified)?'/)
  }

  for (const locale of locales) {
    const translate = (key, values) =>
      locale.workflow[key.slice('workflow.'.length)].replace(/\{(\w+)\}/g, (_, name) => values[name])
    for (const errorKind of ['rate_limited', 'upstream_server', 'network', 'timeout', 'unknown', undefined]) {
      const retry = { attempt: 2, total: 5, nextRetryIn: 7, errorKind, errorSummary: 'backend detail' }
      const context = {
        formatWorkflowRetryMessage,
        tailAlert: { retryInfo: retry },
        $t: translate,
        t: translate
      }
      const footerText = runInNewContext(footerCall, context)
      if (errorKind === 'rate_limited') {
        assert.equal(footerText, translate('workflow.retrying', { attempt: 2, total: 5, seconds: 7 }))
      } else {
        assert.equal(footerText, translate('workflow.retryingClassified', {
          attempt: 2, total: 5, seconds: 7, kind: errorKind || 'unknown', summary: 'backend detail'
        }))
        assert.ok(footerText.includes('backend detail'), 'non-rate-limit retries must retain backend details')
      }
    }
  }
})

test('workflow terminal model errors use structured localized alert titles', async () => {
  const [messageList, enLocale, zhHansLocale, zhHantLocale] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/i18n/locales/en.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hant.json', 'utf8').then(JSON.parse)
  ])

  assert.match(
    messageList,
    /const localizedErrorTitles = \{[\s\S]*?llm_authentication: 'workflow\.errorTypes\.llmAuthentication',[\s\S]*?llm_billing: 'workflow\.errorTypes\.llmBilling',[\s\S]*?llm_retry_exhausted: 'workflow\.errorTypes\.llmRetryExhausted'[\s\S]*?\}/
  )
  assert.match(messageList, /const rawType = String\(message\?\.metadata\?\.error_type \|\| message\?\.errorType \|\| ''\)/)
  const errorContentStart = messageList.indexOf('const getErrorAlertContent = message =>')
  const errorContentEnd = messageList.indexOf(
    'const getExplorationBatchSummary = message =>',
    errorContentStart
  )
  const errorContentSource = messageList.slice(errorContentStart, errorContentEnd)
  assert.match(
    errorContentSource,
    /(?:resolveWorkflowErrorDisplay\(message, message\?\.message\)|normalizeWorkflowErrorAlertContent\(message\?\.message\))[\s\S]*?metadata\.retry_exhausted !== true[\s\S]*?workflow\.errorTypes\.retryAttemptsExhausted/
  )
  assert.doesNotMatch(errorContentSource, /props\.getParsedMessage\(message\)/)
  assert.doesNotMatch(messageList, /quota|payment required|insufficient balance/i)

  for (const locale of [enLocale, zhHansLocale, zhHantLocale]) {
    assert.equal(typeof locale.workflow.errorTypes.llmAuthentication, 'string')
    assert.equal(typeof locale.workflow.errorTypes.llmBilling, 'string')
    assert.equal(typeof locale.workflow.errorTypes.llmRetryExhausted, 'string')
    assert.equal(typeof locale.workflow.errorTypes.retryAttemptsExhausted, 'string')
    assert.ok(locale.workflow.errorTypes.llmAuthentication.length > 0)
    assert.ok(locale.workflow.errorTypes.llmBilling.length > 0)
    assert.ok(locale.workflow.errorTypes.llmRetryExhausted.length > 0)
    assert.ok(locale.workflow.errorTypes.retryAttemptsExhausted.length > 0)
  }
})

test('terminal provider errors survive live projection and history reload as tail alerts', async () => {
  const [messageList, locale, rules] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8').then(JSON.parse),
    import('./messageProjectionRules.js')
  ])
  const start = messageList.indexOf('const getErrorAlertTitle = message =>')
  const end = messageList.indexOf('const getExplorationBatchSummary = message =>', start)
  assert.ok(start >= 0 && end > start)
  const alerts = runInNewContext(
    `${messageList.slice(start, end)}\n;({ getErrorAlertContent, getErrorAlertTitle })`,
    {
      resolveWorkflowErrorDisplay,
      normalizeWorkflowErrorAlertContent,
      props: { removeSystemReminder: value => value },
      t: key => key.split('.').reduce((value, part) => value?.[part], locale)
    }
  )
  const { resolveWorkflowTailAlert } = rules
  assert.equal(
    typeof resolveWorkflowTailAlert,
    'function',
    'the message pane must resolve terminal errors into one tail alert'
  )
  for (const [status, errorType, detail] of [
    [401, 'llm_authentication', 'Invalid API key'],
    [402, 'llm_billing', 'Insufficient balance'],
    [403, 'engine', 'Forbidden: invalid credentials'],
    [404, 'engine', 'Model not found'],
    [503, 'upstream_server', 'Service unavailable']
  ]) {
    const liveMessage = {
      id: String(9000 + status),
      role: 'assistant',
      message: JSON.stringify({ error: { message: detail } }),
      stepType: 'Observe',
      isError: true,
      errorType,
      metadata: {
        message_kind: 'runtime_observation',
        observation_type: 'terminal_error',
        llm_visibility: 'hide',
        ui_visibility: 'show',
        error_type: errorType,
        upstream_status_code: status,
        provider: 'test-provider'
      }
    }
    const historyMessage = { ...liveMessage, id: Number(liveMessage.id) }
    for (const messages of [[liveMessage], [historyMessage], mergeWorkflowMessagePages([historyMessage], [liveMessage])]) {
      const projected = projectWorkflowMessageList(messages)
      assert.equal(
        projected.filter(message => message?.role !== 'tool' && message?.isError).length,
        0,
        `${status}: a non-tool error must not render as an inline transcript message`
      )
      const tailAlert = resolveWorkflowTailAlert(messages, {
        notification: null,
        retryInfo: null,
        consumedIdentity: ''
      })
      assert.ok(tailAlert, `${status}: terminal errors must survive as one visible tail alert`)
      assert.equal(tailAlert.category, 'error')
      assert.ok(tailAlert.message, `${status}: the tail alert must retain the original error message`)
      assert.equal(alerts.getErrorAlertContent(tailAlert.message), detail)
      assert.notEqual(alerts.getErrorAlertTitle(tailAlert.message), locale.workflow.retrying)
      assert.equal(tailAlert.message.metadata.upstream_status_code, status)
    }
  }
})

test('status notifier reports only workflow activity, not notifications or errors', async () => {
  const notifier = await readFile('src/components/workflow/StatusNotifier.vue', 'utf8')

  assert.doesNotMatch(notifier, /workflowStore\.notification/)
  assert.doesNotMatch(notifier, /setNotification/)
  assert.doesNotMatch(notifier, /retryInfo|hasActiveRetry|formatWorkflowRetryMessage/)
  assert.doesNotMatch(notifier, /latestTerminalError|is_error|isError|resolveWorkflowErrorDisplay/)
  assert.match(notifier, /const latestToolState = computed/)
  assert.match(notifier, /latestTextActivity/)
  assert.match(notifier, /isWaitingForUser/)
  assert.match(notifier, /isWaitingForApproval/)
  assert.match(notifier, /workflowStore\.isRunning/)
  assert.match(notifier, /workflow\.statusNotifier\.runningElapsed/)
})

test('message list renders one tail alert after queued messages', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /tailAlert: \{\s*type: Object,\s*default: \(\) => null\s*\}/
  )
  const queuedIndex = messageList.indexOf('class="queued-list"')
  const tailIndex = messageList.indexOf('v-if="tailAlert"')
  const tailEnd = messageList.indexOf('</el-alert>', tailIndex)
  const templateEnd = messageList.indexOf('</template>', tailEnd)
  assert.ok(queuedIndex >= 0 && tailIndex > queuedIndex && tailEnd > tailIndex)
  assert.ok(templateEnd > tailEnd, 'the tail alert must be the last block before the list template closes')
  assert.doesNotMatch(messageList, /shouldShowErrorAlert/)
  assert.match(messageList, /v-if="getCopyableAiOutput\(message\)"/,
    'normal assistant output must remain independent from the reasoning block')
  assert.doesNotMatch(messageList, /retry-status-alert/)
  assert.doesNotMatch(messageList, /getParsedMessage\(tailAlert/)

  const tailSource = messageList.slice(tailIndex, tailEnd)
  assert.match(tailSource, /:type="tailAlert\.category"/)
  assert.match(tailSource, /:closable="false"/)
  assert.match(tailSource, /show-icon/)
  assert.match(tailSource, /role="alert"/)
  assert.match(tailSource, /workflow-error-alert/)
  assert.match(tailSource, /getErrorAlertTitle\(tailAlert\.message\)/)
  assert.match(tailSource, /formatWorkflowRetryMessage\(tailAlert\.retryInfo, \$t\)/)
  assert.match(tailSource, /common\.warning/)
  assert.match(tailSource, /MarkdownSimple/)
  assert.match(
    messageList,
    /const getTailAlertBody = \(\) => \{[\s\S]*?alert\.message[\s\S]*?getErrorAlertContent\(alert\.message\)[\s\S]*?\}\n/,
    'fatal tail alerts must reuse the formatted error content'
  )
  assert.match(
    messageList,
    /const getCopyableAiOutput = message => \{\s*if \(message\?\.isError/,
    'error bodies must not fall through to the normal AI output renderer'
  )
})

test('tail reminder presentation survives reload and consumes only on new message activity', async () => {
  const source = await readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8')
  const start = source.indexOf('const alertPresentation = ref({})')
  const end = source.indexOf('const isChatting = computed', start)
  assert.ok(start >= 0 && end > start)
  const storage = new Map()
  const fatal = { id: 10, sessionId: 'a', role: 'assistant', message: 'Failure', isError: true }
  const props = { sessionId: 'a', agentRole: 'primary' }
  const store = { notification: null }
  const sourceMessages = { currentWorkflowId: 'a', messages: [fatal] }
  const stream = { value: { content: '', reasoning: '', retryInfo: null } }
  let tick
  const mount = () => runInNewContext(`${source.slice(start, end)};({ tailAlert, alertPresentation })`, {
    ref: value => ({ value }),
    computed: getter => ({ get value() { return getter() } }),
    watch: (getter, callback) => { tick = () => callback(getter()); tick() },
    props, source: sourceMessages, workflowStore: store, resolvedChatState: stream,
    hasVisibleWorkflowText, reconcileWorkflowTailAlert, resolveWorkflowTailAlert,
    localStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) },
    console
  })
  let pane = mount()
  assert.equal(pane.tailAlert.value.message, fatal)
  pane = mount()
  assert.equal(pane.tailAlert.value.message, fatal, 'an unconsumed fatal survives reload')
  stream.value.content = 'Recovered stream'
  tick()
  assert.equal(pane.tailAlert.value, null)
  stream.value.content = ''
  pane = mount()
  assert.equal(pane.tailAlert.value, null, 'consumption survives reload before stream persistence')
  const warning = { sessionId: 'a', message: 'Warning', category: 'warning', timestamp: 1 }
  store.notification = warning
  tick()
  assert.equal(pane.tailAlert.value.text, 'Warning')
  store.notification = { sessionId: 'a', message: '', category: 'info', timestamp: 2 }
  tick()
  pane = mount()
  assert.equal(pane.tailAlert.value.text, 'Warning', 'terminal notification cleanup is not new message activity')
  sourceMessages.messages.push({ id: 11, sessionId: 'a', role: 'user', message: 'Next' })
  tick()
  pane = mount()
  assert.equal(pane.tailAlert.value, null)
  assert.equal(sourceMessages.messages[0], fatal, 'original diagnostics remain available')
  props.sessionId = 'b'
  sourceMessages.currentWorkflowId = 'b'
  sourceMessages.messages = [{ ...fatal, sessionId: 'b' }]
  tick()
  assert.equal(pane.tailAlert.value.message.sessionId, 'b')
  assert.equal(pane.tailAlert.value.message.id, 10, 'session A consumption does not consume B')
  props.sessionId = 'a'
  sourceMessages.currentWorkflowId = 'a'
  sourceMessages.messages = [fatal]
  tick()
  assert.equal(pane.tailAlert.value, null)
  const finalError = { ...fatal, id: 12 }
  sourceMessages.messages.push(finalError)
  tick()
  assert.equal(pane.tailAlert.value.message, finalError)
  stream.value.retryInfo = { attempt: 1, total: 3, nextRetryIn: 3 }
  tick()
  assert.equal(pane.tailAlert.value.retryInfo, stream.value.retryInfo)
  stream.value.retryInfo = null
  tick()
  assert.equal(pane.tailAlert.value, null, 'retry end must not resurrect a superseded fatal')
  assert.equal(pane.alertPresentation.value.consumedIdentity, getWorkflowAlertIdentity(finalError))
})

test('workflow uses a persistent navigation rail beside the collapsible task list', async () => {
  const [workflowView, sidebar] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSidebar.vue', 'utf8')
  ])

  assert.match(workflowView, /<nav v-if="!sidebarCollapsed" class="workflow-side-rail"[^>]*workflow\.sidebarNavigation/)
  assert.match(workflowView, /<el-tooltip :content="\$t\('workflow\.taskTab'\)"[^>]*>\s*<button\s+class="workflow-side-rail__item"/)
  assert.match(workflowView, /@click="openWorkflowSidebarTab\('history'\)"/)
  assert.match(workflowView, /@click="openWorkflowSidebarTab\('automation'\)"/)
  assert.match(workflowView, /<WorkflowSidebar\s+:workflows="filteredWorkflows"[\s\S]*?:navigation-tab="workflowSidebarNavigationTab"/)
  assert.match(workflowView, /v-model:navigation-tab="workflowSidebarNavigationTab"/)
  assert.match(sidebar, /<el-tab-pane\s+v-if="isHistoryTabVisible"[\s\S]*:label="\$t\('workflow\.taskTab'\)"[\s\S]*name="history">/)
  assert.match(sidebar, /<el-tab-pane\s+v-if="isAutomationTabVisible"[\s\S]*:label="\$t\('workflow\.automation\.title'\)"[\s\S]*name="automation">/)
  assert.match(sidebar, /const compactSidebarTab = computed\(\(\) =>[\s\S]*props\.navigationTab/)
  assert.match(sidebar, /const isHistoryTabVisible = computed\(\(\) => props\.navigationTab === 'history'/)
  assert.match(sidebar, /const isAutomationTabVisible = computed\(\(\) => props\.navigationTab === 'automation'/)
  assert.doesNotMatch(workflowView, /<WorkflowSidebar\s+v-if="!sidebarCollapsed"/)
  // Collapsed rail keeps the original compact structure: tab buttons, workflow list and terminal entry.
  assert.match(sidebar, /class="compact-sidebar-tabs"[\s\S]*class="compact-sidebar-tab"[\s\S]*selectCompactSidebarTab\('history'\)[\s\S]*selectCompactSidebarTab\('automation'\)/)
  assert.match(sidebar, /emit\('update:navigationTab', tab\)/)
  assert.match(sidebar, /class="workflow-terminal-entry compact-terminal-entry"[\s\S]*@click="\$emit\('open-terminal'\)"/)
  assert.match(workflowView, /@open-terminal="terminal\.open"/)
  assert.doesNotMatch(workflowView, /workflow-sidebar-collapsed-terminal/)
  assert.match(sidebar, /@reorder-paths="\$emit\('reorder-paths-from-tree', \$event\)"/)
})

test('authorized root drag sorting stays on the existing structured allowed-path update path', async () => {
  const [fileTree, sidebar, workflowView, workflowPaths, workflowStore, workflowCommand, pathGuard, engine] =
    await Promise.all([
      readFile('src/components/workflow/FileTree.vue', 'utf8'),
      readFile('src/components/workflow/WorkflowSidebar.vue', 'utf8'),
      readFile('src/views/Workflow.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowPaths.ts', 'utf8'),
      readFile('src/stores/workflow.js', 'utf8'),
      readFile('src-tauri/crates/backend/src/commands/workflow.rs', 'utf8'),
      readFile('src-tauri/crates/backend/src/workflow/react/security.rs', 'utf8'),
      readFile('src-tauri/crates/backend/src/workflow/react/engine.rs', 'utf8')
    ])

  assert.match(fileTree, /import \{ Sortable \} from 'sortablejs-vue3'/)
  assert.match(fileTree, /<Sortable[\s\S]*?:list="rootItems"[\s\S]*?@update="onRootSort"/)
  assert.match(fileTree, /const rootItems = ref\(\[\]\)/)
  assert.match(fileTree, /filter: '\.root-actions'/)
  assert.match(fileTree, /rootItems\.value = newPaths\.map\(path => \(\{ path \}\)\)/)
  assert.match(fileTree, /emit\('reorderPaths', reorderedPaths\)/)
  assert.doesNotMatch(fileTree, /onRootDrop|onRootDragStart|:draggable=/)
  assert.match(sidebar, /@reorder-paths="\$emit\('reorder-paths-from-tree', \$event\)"/)
  assert.match(workflowView, /@reorder-paths-from-tree="onReorderPathsFromTree"/)
  assert.match(workflowPaths, /const onReorderPathsFromTree = async paths => \{[\s\S]*?updateWorkflowAllowedPaths\(currentWorkflowId\.value, paths\)/)
  assert.match(workflowPaths, /updateAutomationAllowedPaths\(selectedAutomation\.value\.id, paths\)/)
  assert.match(workflowStore, /invokeWrapper\('update_workflow_allowed_paths', \{[\s\S]*?allowedPaths: allowedPaths/)
  assert.match(workflowCommand, /"type": "update_allowed_paths",[\s\S]*?"paths": runtime_paths/)
  assert.match(pathGuard, /self\.primary_root = self\.workspace_roots\.first\(\)/)
  assert.match(engine, /guard\.update_allowed_roots\(paths\.clone\(\)\);[\s\S]*?self\.planning_root = Self::planning_root_for_allowed_paths\(&paths\)/)
})

test('Agent selector stays visible and only enables task-boundary changes', async () => {
  const [inputArea, workflowView, workflowCommand, backendConstitution] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src-tauri/crates/backend/src/commands/workflow.rs', 'utf8'),
    readFile('src-tauri/crates/backend/src/workflow/react/CONSTITUTION.md', 'utf8')
  ])

  assert.match(inputArea, /<div class="selector-wrap">\s*<AgentSelector[\s\S]*?:disabled="!canEditAgent"/)
  assert.match(workflowView, /const hasClearedCurrentContext = computed\(\(\) =>\s*workflowStore\.messages\.at\(-1\)\?\.messageSubtype === 'manual_clear_context'/)
  assert.match(workflowView, /canSwitchCurrentWorkflowAgent = computed\([\s\S]*?hasClearedCurrentContext/)
  assert.match(backendConstitution, /Agent tool changes synchronize into an existing workflow only at canonical task boundaries:[\s\S]*?- the user manually clears context/)
  assert.match(workflowCommand, /let is_manual_clear_boundary = snapshot[\s\S]*?if !is_manual_clear_boundary[\s\S]*?snapshot\.workflow\.user_query/)
  assert.match(workflowCommand, /fn merge_agent_switch_config\([\s\S]*?merge_inherited_workflow_config\(&agent_config, current_config\)/)
  assert.match(workflowCommand, /context\.state != RuntimeState::Pending \|\| !context\.pending_tools\.is_empty\(\)/)
})

test('empty new workflows require an authorized directory before accepting input', async () => {
  const [inputArea, workflowView, workflowPaths] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowPaths.ts', 'utf8')
  ])

  assert.match(inputArea, /:readonly="requiresAuthorizedPathSelection"/)
  assert.match(
    inputArea,
    /Boolean\(props\.currentWorkflowId && props\.currentWorkflow\)[\s\S]*?!props\.currentWorkflow\.userQuery\?\.trim\(\)[\s\S]*?props\.currentPaths\.length === 0/
  )
  assert.match(inputArea, /@click="handleInputClick"/)
  assert.match(
    inputArea,
    /const handleInputClick = async \(\) => \{[\s\S]*?await props\.onAddAuthorizedPath\?\.\(\)[\s\S]*?await nextTick\(\)[\s\S]*?!requiresAuthorizedPathSelection\.value[\s\S]*?inputRef\.value\?\.focus\(\)/
  )
  assert.match(workflowView, /:current-paths="currentPaths"[\s\S]*?:on-add-authorized-path="onAddPath"/)
  assert.match(workflowView, /displayAllowedPath,[\s\S]*?onAddPath,[\s\S]*?onAddPathFromTree/)
  assert.match(
    workflowPaths,
    /const onAddPath = async \(\) => \{[\s\S]*?await open\([\s\S]*?directory: true[\s\S]*?pendingPaths\.value\.push\(selected\)/
  )
})

test('child session panes preserve a confirm received while snapshot hydration is in flight', async () => {
  const [workflowView, messageList, sessionPane, sessionMessages, workflowCore, engine, workflowCommand, workflowDb, workflowStore] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowSessionMessages.ts', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
    readFile('src-tauri/crates/backend/src/workflow/react/engine.rs', 'utf8'),
    readFile('src-tauri/crates/backend/src/commands/workflow.rs', 'utf8'),
    readFile('src-tauri/crates/backend/src/db/workflow.rs', 'utf8'),
    readFile('src/stores/workflow.js', 'utf8')
  ])

  assert.match(messageList, /<cs name="fullscreen-off"\s*\/>/)
  assert.match(messageList, /sub-agent-card__status[\s\S]*?sub-agent-card__open-button/)
  assert.match(messageList, /sub-agent-card__title-wrap[\s\S]*?sub-agent-card__meta/)
  assert.match(messageList, /@click\.stop="\$emit\('open-sub-agent', message\.subAgentCard\.taskId\)"/)
  assert.match(workflowView, /v-if="activeSubAgentSessionId && activeSubAgentParentSessionId === currentWorkflowId"/)
  assert.match(workflowView, /const openSubAgentMessagePane = sessionId => \{[\s\S]*?activeSubAgentParentSessionId\.value = parentSessionId/)
  assert.match(workflowView, /watch\([\s\S]*?\(\) => workflowStore\.currentWorkflowId,[\s\S]*?currentSessionId !== previousSessionId[\s\S]*?closeSubAgentMessagePane\(\)/)
  assert.match(workflowView, /await selectWorkflow\(navigationSessionId\)[\s\S]*?if \(entry\?\.subAgentId\) openSubAgentMessagePane\(entry\.subAgentId\)/)
  assert.match(workflowView, /\.workflow-chat-pane \{[\s\S]*?position: relative;/)
  assert.match(workflowView, /\.sub-agent-message-overlay \{[\s\S]*?position: absolute;[\s\S]*?inset: 0;/)
  assert.match(sessionPane, /:messages="messageProjection\.enhancedMessages\.value"/, 'the message list must receive the projected message array, not its computed ref')
  assert.match(sessionPane, /:hidden-earlier-message-count="messageProjection\.hiddenEarlierMessageCount\.value"/, 'the message-list history count must receive its computed value')
  assert.match(sessionPane, /:last-assistant-message="messageProjection\.lastAssistantMessage\.value"/, 'the message list must receive the resolved last assistant message')
  assert.match(sessionPane, /:approval-loading="approval\.approvalLoading\.value"/, 'the message list must receive the approval loading boolean, not its ref')
  assert.match(sessionPane, /:active-approval-id="approval\.activeApprovalId\.value"/, 'the message list must receive the active approval ID, not its ref')
  assert.match(sessionPane, /:is-approval-submitting="resolvedIsApprovalSubmitting"/, 'child panes must use their session-local approval submission state')
  assert.match(sessionPane, /<cs name="fullscreen"\s*\/>/)
  assert.match(sessionPane, /class="workflow-session-message-pane__status"[\s\S]*?:class="childStatusClass"/)
  assert.match(sessionPane, /workflow\.subAgent\.statusCompleted/)
  assert.match(sessionPane, /workflow\.subAgent\.statusCancelled/)
  assert.match(sessionPane, /agentStore\.agents\.find/)
  assert.doesNotMatch(sessionPane, /WorkflowInputArea/)
  assert.match(sessionMessages, /hydrateWorkflowSession\(/, 'child hydration must install its event buffer before listener registration')
  assert.match(sessionMessages, /registerListener: handleEvent => listen\(`workflow:\/\/event\/\$\{targetSessionId\}`/, 'child hydration must listen to its own channel')
  assert.match(sessionMessages, /fetchSnapshot: \(\) => invokeWrapper\('get_workflow_snapshot', \{ sessionId: targetSessionId \}\)/, 'child hydration must use the canonical snapshot')
  assert.match(
    sessionPane,
    /const loadEarlierMessagePage = async done => \{[\s\S]*?\} catch \(error\) \{[\s\S]*?Failed to load earlier messages[\s\S]*?\} finally \{[\s\S]*?done\?\.\(\)/,
    'session panes must load a backend page when the projected window has no locally hidden messages'
  )
  assert.match(
    workflowStore,
    /const responseShape[\s\S]*?Invalid earlier workflow message page response/,
    'primary pagination must expose the actual malformed response shape'
  )
  assert.match(
    sessionMessages,
    /const responseShape[\s\S]*?Invalid earlier workflow message page response/,
    'child pagination must expose the actual malformed response shape'
  )
  assert.match(sessionMessages, /applyEvent,[\s\S]*?isCurrent: \(\) => revision === loadRevision\.value && targetSessionId === sessionId\.value,[\s\S]*?onListenerRegistered: stop =>/, 'the listener must be retained from registration through snapshot replay for the active child session')
  assert.match(sessionPane, /const resolvedIsApprovalSubmitting = \(sessionId, toolCallId\) =>[\s\S]*?childSession\.isApprovalSubmitting\(toolCallId\)/)
  assert.match(sessionMessages, /const approvalSubmissions = ref\(new Set\(\)\)/)
  assert.match(sessionMessages, /submittedToolIds: approvalSubmissions\.value/)
  assert.match(sessionMessages, /const markApprovalSubmitted = toolCallId =>/)
  assert.match(sessionMessages, /const clearApprovalSubmission = toolCallId =>/)
  assert.match(sessionMessages, /const isApprovalSubmitting = toolCallId =>[\s\S]*?approvalSubmissions\.value\.has/)
  assert.match(sessionMessages, /payload\.type === 'approval_resolved'[\s\S]*?clearApprovalSubmission\(toolCallId\)/)
  assert.match(sessionMessages, /payload\.type === 'tool_started'[\s\S]*?clearApprovalSubmission\(toolCallId\)/)
  assert.match(sessionPane, /markApprovalSubmitted:[\s\S]*?childSession\.markApprovalSubmitted\(toolCallId\)[\s\S]*?clearApprovalSubmission:[\s\S]*?childSession\.clearApprovalSubmission\(toolCallId\)[\s\S]*?isApprovalSubmitted:[\s\S]*?childSession\.isApprovalSubmitting\(toolCallId\)/)
  assert.match(sessionPane, /onApproveAction\(toolCallId, props\.sessionId\)/, 'child approvals must still target the child session')
  assert.match(
    engine,
    /Active execution must consume only signals it owns[\s\S]*?sub-agent approval that arrives while a previous tool runs[\s\S]*?per-session FIFO stash until its compatible wait state is entered/,
    'active child execution must preserve approvals until AwaitingApproval owns them'
  )
  assert.match(
    engine,
    /async fn check_stop_signal[\s\S]*?signal_deferred_non_waiting[\s\S]*?stash_runtime_signal\(&self\.session_id, s\)/,
    'the active signal drain must defer typed wait signals instead of consuming child approvals'
  )
  assert.match(
    engine,
    /let signal_str = if let Some\(signal\)\s*=\s*take_stashed_runtime_signal\(&self\.session_id\)[^]*?Signal receiver returned None while waiting[^]*?Signal channel closed/,
    'AwaitingApproval must consume deferred signals before reading the live channel'
  )
  assert.match(sessionMessages, /workflow:\/\/event\/\$\{targetSessionId\}/, 'child pane must listen to its own channel')
  assert.doesNotMatch(sessionMessages, /selectWorkflow\(/, 'child pane must not replace the root selection')
  assert.match(workflowCore, /targetSessionId: payload\.sub_agent_id,[\s\S]*?navigationSessionId: payload\.parent_session_id/)
  assert.match(
    messageList,
    /const inlineBulkApprovalCount = computed\(\(\) => props\.pendingCount\)/,
    'the bulk approval count must follow the canonical pending ID count instead of rendered message timing'
  )
  assert.match(
    messageList,
    /const pendingIdSet = new Set\(pendingIds\)[\s\S]*?pendingIdSet\.has\(toolCallId\)/,
    'bulk approval ordering must be constrained by the canonical pending ID set'
  )
  assert.match(
    workflowCore,
    /const handleSubAgentApprovalRequested = payload =>[\s\S]*?playApprovalNotificationSound\(\)/,
    'sub-agent approval requests must use the shared reminder and notification path'
  )
  assert.match(
    workflowCore,
    /if \(payload\.type === 'sub_agent_approval_requested'\) \{\s*handleSubAgentApprovalRequested\(payload\)/,
    'active workflow listeners must process bridged sub-agent approvals'
  )
  assert.match(
    workflowCore,
    /workflowStore\.clearApprovalSubmission\(sessionId, payload\.tool_call_id\)/,
    'background approval resolution must clear local submission state'
  )
  assert.match(
    workflowCore,
    /if \(workflow\.id === activeSessionId\) return false/,
    'the active workflow must have one authoritative event listener'
  )
  assert.match(sessionPane, /:pending-count="resolvedPendingApprovalIds\.length"/)
  assert.match(
    engine,
    /SubAgentApprovalRequested \{[\s\S]*?arguments: arguments\.clone\(\),[\s\S]*?details: details\.clone\(\)/,
    'sub-agent approval bridges must preserve structured approval payloads'
  )
  assert.match(workflowCore, /const clearPendingApprovalEntriesBySubAgent = subAgentId =>/)
  assert.match(workflowCore, /nextEntries\[key\]\?\.subAgentId !== normalizedSubAgentId/)
  assert.match(workflowCore, /sub_agent_progress[\s\S]*?clearPendingApprovalEntriesBySubAgent\(payload\.sub_agent_id\)/)
  assert.match(workflowCore, /\['completed', 'failed', 'cancelled', 'interrupted', 'error'\]/)
  assert.match(engine, /GatewayPayload::SubAgentApprovalRequested/)
  assert.match(engine, /GatewayPayload::SubAgentApprovalResolved/)
  assert.match(
    engine,
    /WorkflowState::Completed\s*\| WorkflowState::Cancelled\s*\| WorkflowState::Error/,
    'terminal child workflows must clear their authoritative pending approvals'
  )
  assert.match(
    workflowCore,
    /const activeParentWorkflowIds = new Set\([\s\S]*?!TERMINAL_STATUSES\.includes\(status\)/,
    'the frontend must skip restoration when every parent workflow is terminal'
  )
  assert.match(workflowCore, /if \(!activeParentWorkflowIds\.size\) return/)
  assert.match(workflowCore, /if \(!activeParentWorkflowIds\.has\(parentSessionId\)\) continue/)
  assert.match(workflowCommand, /list_child_workflows_with_pending_approvals\(\)/)
  assert.match(
    workflowDb,
    /status IN \('awaiting_approval', 'awaiting_auto_approval'\)/,
    'the backend query must only load child workflows currently waiting for approval'
  )
  assert.match(
    workflowDb,
    /parent\.status NOT IN \('completed', 'failed', 'error', 'cancelled'\)/,
    'the backend query must exclude children of terminal parents before loading snapshots'
  )
})

test('MCP tool calls show their arguments and format only valid JSON results', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /const isMcpToolMessage = message =>[\s\S]*?isWorkflowMcpTool\(getMessageToolName\(message\), getMessageToolCategory\(message\)\)/
  )
  assert.match(messageList, /class="mcp-tool-arguments"/)
  assert.match(messageList, /getFormattedMcpToolArguments\(tool\)/)
  assert.match(messageList, /getFormattedMcpToolArguments\(message\)/)
  assert.match(messageList, /JSON\.stringify\(JSON\.parse\(value\), null, 2\)/)
  assert.match(messageList, /catch \{\s*return value\s*\}/)
  assert.match(messageList, /getMcpToolContentForDisplay\(tool\)/)
  assert.match(messageList, /getMcpToolContentForDisplay\(message\)/)
})

test('ask-user responses stay hidden from the transcript and render on their source tool card', async () => {
  const [workflowView, workflowCore, workflowMessages, messageList, workflowEngine, workflowStore, sessionPane, sessionMessages] =
    await Promise.all([
      readFile('src/views/Workflow.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
      readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
      readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
      readFile('src-tauri/crates/backend/src/workflow/react/engine.rs', 'utf8'),
      readFile('src/stores/workflow.js', 'utf8'),
      readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowSessionMessages.ts', 'utf8')
    ])

  assert.match(
    workflowView,
    /const submitAskUserResponse = async response => \{[\s\S]*?coreOnSendMessage\(content, \{[\s\S]*?ui_visibility: 'hide',[\s\S]*?ask_user_response: true,[\s\S]*?requested_tool_call_id/,
    'ask-user answers must remain hidden and carry an association hint'
  )
  assert.match(
    workflowCore,
    /if \(options\.metadata\) \{\s*signalPayload\.metadata = options\.metadata/,
    'hidden-message metadata must continue through the user-message signal sent to the runtime'
  )
  assert.match(
    sessionPane,
    /const submitAskUserResponse = response => \{[\s\S]*?sessionId: props\.sessionId,[\s\S]*?waitReason: resolvedWaitReason\.value,[\s\S]*?hasLiveSession: childWorkflow\.hasLiveSession === true/
  )
  assert.match(
    workflowView,
    /@submit-ask-user="submitAskUserResponse"[\s\S]*?const submitAskUserResponse = async response => \{[\s\S]*?response\?\.target/
  )
  assert.match(
    workflowCore,
    /const showSubAgentAskUserNotification = \(sessionId, payload = \{\}\) =>[\s\S]*?parentSessionId !== sessionId[\s\S]*?kind: 'ask_user'[\s\S]*?targetSessionId: subAgentId/
  )
  assert.match(
    workflowCore,
    /payload\.wait_reason === WORKFLOW_WAIT_REASONS\.USER_INPUT[\s\S]*?showSubAgentAskUserNotification\(sessionId, payload\)/
  )
  assert.match(
    sessionMessages,
    /hasLiveSession: snapshot\.hasLiveSession === true/
  )
  const awaitingUserMetadataWrites = workflowEngine.match(
    /WorkflowSignal::UserMessage \{\s*content, metadata, \.\.\s*\}[\s\S]*?canonicalize_ask_user_response_metadata\(metadata\)[\s\S]*?add_message_and_notify_internal\([\s\S]*?metadata,\s*\)/g
  )
  assert.equal(
    awaitingUserMetadataWrites?.length,
    2,
    'both awaiting-user signal branches must persist backend-canonicalized answer metadata'
  )
  assert.match(
    workflowMessages,
    /const askUserResponsesByToolCallId = computed\(\(\) => \{[\s\S]*?getMessageToolCallId\(message\)[\s\S]*?responses\.set\(toolCallId, message\.message \|\| ''\)/,
    'the hidden answer must be indexed by its canonical tool_call_id'
  )
  assert.match(
    workflowMessages,
    /if \(!getMessageToolCallId\(message\)\) \{\s*askUserResponsesBySourceOrder\.set\(pendingAskUserSourceOrder, message\.message\)/,
    'positional ask-user response matching must be limited to legacy rows without tool_call_id'
  )
  assert.match(
    workflowMessages,
    /askUserResponse: resolveAskUserResponse\([\s\S]*?askUserResponsesByToolCallId\.value,[\s\S]*?askUserResponsesBySourceOrder/,
    'the source ask_user tool must receive its response through the ID-first projection rule'
  )
  assert.match(
    workflowMessages,
    /if \(isApprovedPlanAnchor\(message\) \|\| isHiddenAskUserResponse\(message\)\) return false/,
    'the hidden answer must stay out of the transcript after its association is derived'
  )
  assert.match(
    workflowMessages,
    /askUserResponse: message\?\.askUserResponse \|\| ''/,
    'ask-user response attachment must invalidate the enhanced-message cache'
  )
  assert.match(
    messageList,
    /@focus="selectAskUserOtherIfUnset\(message, group\.title\)"/,
    'focusing the supplemental input must select Other when the user has not selected an option'
  )
  assert.match(
    messageList,
    /const selectAskUserOtherIfUnset = \(message, title\) => \{[\s\S]*?OTHER_ASK_USER_VALUE/,
    'the focus handler must select the fixed Other value only when unset'
  )
  assert.match(
    messageList,
    /const getToolRenderStatusClass = message => \{[\s\S]*?isToolGroupItemRunning\(message\)/,
    'standalone and grouped tools must share the running/error/success class mapping'
  )
  assert.match(
    workflowView,
    /const batchApprovalSessionId = ref\(''\)[\s\S]*?if \(!sessionId \|\| batchApprovalSessionId\.value\) return/,
    'bulk approval must be locked per active workflow session'
  )
  assert.match(
    messageList,
    /props\.isBatchApprovalSubmitting[\s\S]*?orderedToolCallIds: pendingToolCallIds/,
    'all inline approval cards must disable duplicate batch submissions while the batch is active'
  )
  assert.match(
    workflowStore,
    /invokeWrapper\('get_earlier_workflow_message_page'/,
    'history pagination must use the regular message page endpoint'
  )
  assert.doesNotMatch(
    workflowStore,
    /hasMoreInCurrentTask|hiddenCompletedTaskCount|get_earlier_workflow_messages/,
    'task-segment pagination state and endpoint must not remain in the frontend store'
  )
  assert.doesNotMatch(
    workflowView,
    /withinCurrentTask|loadEarlierTaskGroup|revealEarlierTaskGroup/,
    'the workflow view must request ordinary message pages'
  )
  assert.match(
    messageList,
    /<template v-if="getAskUserResponseItems\(message\)\.length > 0">[\s\S]*?v-for="group in getChoiceGroups\(message\)"[\s\S]*?choice-options--readonly[\s\S]*?isAskUserOptionSelected[\s\S]*?isAskUserAnswerOther[\s\S]*?getAskUserAnswerSupplement/,
    'an answered ask_user card must retain original choices, mark the selected option, and show supplied text'
  )
  assert.match(
    messageList,
    /const getAskUserResponseForGroup = \(message, title\) =>[\s\S]*?item\?\.title === title/,
    'answered choices must resolve their associated response by question title from the canonical tool response'
  )
  assert.match(
    messageList,
    /const OTHER_ASK_USER_VALUE = '__other__'/,
    'the fixed Other choice must have a distinct non-user option value'
  )
  assert.match(
    messageList,
    /selection === OTHER_ASK_USER_VALUE[\s\S]*?validationOtherRequired/,
    'Other must require supplemental text before submission'
  )
  assert.match(
    messageList,
    /choice: supplement \? `\$\{selection\}\\n\$\{supplement\}` : selection/,
    'standard choices must include an optional supplemental line in their submitted result'
  )
})

test('workflow quick actions expose manual compression through the slash-command handler', async () => {
  const [inputArea, workflowView, signalTypes] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/signalTypes.ts', 'utf8')
  ])

  assert.match(
    inputArea,
    /<el-dropdown-item command="manualCompress">[\s\S]*?<cs name="compress"[\s\S]*?manualCompressShort/,
    'the quick-actions menu must place the manual compression action beside attachments'
  )
  assert.match(inputArea, /emit\('trigger-manual-compress'\)/)
  assert.match(workflowView, /@trigger-manual-compress="triggerManualCompression"/)
  assert.match(workflowView, /type: SIGNAL_TYPES\.MANUAL_COMPRESS/)
  assert.match(signalTypes, /MANUAL_COMPRESS: 'manual_compress'/)
})

test('workflow warnings use only the tail reminder, not a duplicate toast', async () => {
  const source = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  const branch = source.match(
    /else if \(payload.type === 'notification'\) \{([\s\S]*?)\n\s*} else if \(payload.type === 'auto_approved_tools_updated'\)/
  )?.[1]
  assert.ok(branch, 'the canonical notification event must be handled')

  const notifications = []
  const toasts = []
  const handle = runInNewContext(`payload => {${branch}\n}`, {
    markSessionLiveFromNonTerminalEvent: () => {},
    workflowStore: { setNotification: (...args) => notifications.push(args) },
    showMessage: (...args) => toasts.push(args)
  })
  const message = '当前没有可安全压缩的上下文片段'
  handle({ type: 'notification', category: 'warning', message })
  assert.deepEqual(notifications, [[message, 'warning']])
  assert.deepEqual(toasts, [])

  // Empty/progress notifications still follow the existing event path without duplicate toasts.
  notifications.length = 0
  handle({ type: 'notification', category: 'info', message: '' })
  handle({ type: 'notification', category: 'warning', message: '   ' })
  handle({ type: 'notification', category: 'warning' })
  handle({ type: 'notification', category: 'info', message: 'Compressing context...' })
  assert.equal(toasts.length, 0, 'notifications must not create a second reminder surface')
})

test('workflow slash commands manage authorized directories', async () => {
  const [inputArea, workflowView] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8')
  ])

  assert.match(workflowView, /name: 'add-dir'[\s\S]*?commandDirAddDesc[\s\S]*?requiresArgument: true/)
  assert.match(workflowView, /name: 'remove-dir'[\s\S]*?commandDirRemoveDesc/)
  assert.match(
    workflowView,
    /commandName === 'add-dir'[\s\S]*?unwrapDirectoryCommandPath\(commandArgument\)[\s\S]*?addAuthorizedPathFromCommand\(path\)/,
    'add-dir must pass its path argument through the existing authorized-path update flow'
  )
  assert.match(
    workflowView,
    /import \{ homeDir \} from '@tauri-apps\/api\/path'[\s\S]*?const expandDirectoryCommandPath = async value =>[\s\S]*?homeDir\(\)[\s\S]*?return `\$\{home\}\$\{path\.slice\(1\)\}`/,
    'add-dir must expand a leading tilde through the Tauri home directory'
  )
  assert.match(
    workflowView,
    /commandName === 'remove-dir'[\s\S]*?openDirectoryRemovalPanel\?\.\(\)/,
    'remove-dir must open the directory removal panel'
  )
  assert.match(
    inputArea,
    /directoryRemovalPanelVisible[\s\S]*?v-for="path in currentPaths"[\s\S]*?directory-command-remove[\s\S]*?removeDirectoryPath\(path\)/,
    'the removal panel must list current paths with an action button for each path'
  )
  assert.match(inputArea, /const openDirectoryRemovalPanel = \(\) =>/)
  assert.match(inputArea, /defineExpose\(\{[\s\S]*?openDirectoryRemovalPanel/)

  const workflowInput = await readFile('src/composables/workflow/useWorkflowInput.ts', 'utf8')
  assert.match(
    workflowInput,
    /skill\.type === 'command' && skill\.requiresArgument === true[\s\S]*?inputMessage\.value = '\/' \+ skill\.name \+ ' '/,
    'argument-taking commands must be inserted into the composer before execution'
  )
  assert.match(
    workflowInput,
    /skill\.type === 'command' && skill\.requiresArgument === true[\s\S]*?return\n\s*}\n\n\s*if \(skill\.type === 'command' && typeof onBuiltinCommandSelect === 'function'\)[\s\S]*?onBuiltinCommandSelect\(skill\)/,
    'argument-taking commands must return before the ordinary command callback'
  )
})

test('auto-compression starts disabled until explicitly enabled', async () => {
  const [workflowView, workflowCore] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  ])

  assert.match(workflowView, /const autoCompressEnabled = ref\(false\)/)
  assert.match(workflowView, /autoCompressEnabled\.value = agentConfig\?\.autoCompress \?\? false/)
  assert.match(workflowCore, /autoCompressEnabled\.value = config\.autoCompress \?\? false/)
})

test('final audit is consumed only after successful workflow completion', async () => {
  const workflowCore = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  const activeStateHandlerStart = workflowCore.indexOf("if (payload.type === 'state')")
  const activeStateHandlerEnd = workflowCore.indexOf("} else if (payload.type === 'chunk')", activeStateHandlerStart)
  const activeStateHandler = workflowCore.slice(activeStateHandlerStart, activeStateHandlerEnd)

  assert.match(
    workflowCore,
    /const consumeFinalAuditMode = async \(sessionId\) =>[\s\S]*?update_workflow_final_audit[\s\S]*?finalAudit: false/
  )
  assert.match(
    activeStateHandler,
    /if \(\(payload\.state \|\| ''\)\.toLowerCase\(\) === WORKFLOW_STATUSES\.COMPLETED\) \{[\s\S]*?consumeFinalAuditMode\(sessionId\)/,
    'the active workflow must consume final audit only after completed'
  )
  assert.equal(
    (activeStateHandler.match(/consumeFinalAuditMode\(sessionId\)/g) || []).length,
    1,
    'the active workflow must have one final-audit consumption path'
  )
  assert.match(
    activeStateHandler,
    /if \(\(payload\.state \|\| ''\)\.toLowerCase\(\) === WORKFLOW_STATUSES\.COMPLETED\) \{[\s\S]*?consumeFinalAuditMode\(sessionId\)/,
    'failed, cancelled, and error terminal states must keep final audit enabled'
  )
})

test('execution style popover uses a DOM reference and preserves Agent-scoped choices', async () => {
  const [inputArea, workflowView, workflowCore] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  ])

  assert.match(
    inputArea,
    /<el-dropdown-item command="skillsConfig"[\s\S]*?<el-popover[\s\S]*?v-model:visible="autoApprovedPopoverVisible"[\s\S]*?placement="right-start"[\s\S]*?<span class="tool-config-reference">[\s\S]*?<el-dropdown-item class="tool-config-dropdown-trigger"[\s\S]*?<cs name="caret-right"[\s\S]*?<!-- execution style -->/,
    'the quick-actions menu must place the auto-approval submenu after Skills and before execution style'
  )
  assert.doesNotMatch(
    inputArea,
    /<el-dropdown-item class="tool-config-dropdown-trigger" @click\.stop>/,
    'the auto-approval submenu trigger must let click events reach its popover reference'
  )
  assert.match(
    inputArea,
    /<template #reference>\s*<span class="execution-style-reference">[\s\S]*?<el-dropdown-item class="execution-style-dropdown-trigger"[\s\S]*?<cs name="caret-right"/,
    'the quick-actions execution-style submenu must use a concrete DOM popover reference'
  )
  assert.doesNotMatch(
    inputArea,
    /<el-dropdown-item class="execution-style-dropdown-trigger" @click\.stop>/,
    'the submenu trigger must let click events reach its popover reference'
  )
  assert.match(inputArea, /getExecutionStyleOptions\(props\.selectedAgent\)/)
  assert.match(inputArea, /emit\('update-personality', style\)/)
  assert.match(workflowView, /@update-personality="updateWorkflowPersonality"/)
  assert.match(
    workflowCore,
    /resolveExecutionStylePreference\(\s*personality,\s*getWorkflowAgentForExecutionStyle\(\)\s*\)/,
    'the workflow preference must be checked against the current Agent before persistence'
  )
})

test('workflow composer keeps Tab as four spaces outside suggestion selection', async () => {
  const input = await readFile('src/composables/workflow/useWorkflowInput.ts', 'utf8')

  assert.match(
    input,
    /if \(event\.key === 'Tab'\) \{[\s\S]*?event\.preventDefault\(\)[\s\S]*?const indentation = '    '[\s\S]*?inputMessage\.value = value\.slice\(0, start\) \+ indentation \+ value\.slice\(end\)/,
    'Tab must insert four spaces instead of moving focus in the composer'
  )
  assert.match(
    input,
    /if \(showSkillSuggestions\.value\) \{[\s\S]*?event\.key === 'Tab'[\s\S]*?onSkillSelect/,
    'Tab must continue selecting a visible skill suggestion'
  )
  assert.match(
    input,
    /if \(showFileSuggestions\.value\) \{[\s\S]*?event\.key === 'Tab'[\s\S]*?onFileSelect/,
    'Tab must continue selecting a visible file suggestion'
  )
})

test('sending a workflow message forces the message list back to the latest content', async () => {
  const workflowView = await readFile('src/views/Workflow.vue', 'utf8')

  assert.match(
    workflowView,
    /inputComposable\.onSendMessage\.value = async \(\) => \{[\s\S]*?scrollMessageListToBottom\(true\)[\s\S]*?await coreOnSendMessage/,
    'an explicit user send must leave history-reading mode before message rendering starts'
  )
})

test('applied compression clears the indicator and updates context usage', async () => {
  const workflowCore = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')

  assert.match(
    workflowCore,
    /payload\.type === 'compression_applied'[\s\S]*?setCompressionStatus\(sessionId, false, ''\)[\s\S]*?setCurrentContextTokens\([\s\S]*?payload\.current_context_tokens[\s\S]*?payload\.max_context_tokens/,
    'a persisted compression must clear the loading indicator and apply its authoritative usage'
  )
})

test('message resize observer is hoisted for immediate watchers', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /function syncMessageContentResizeObserver\(\)/,
    'the resize observer helper must use a hoisted function declaration'
  )
  assert.match(
    messageList,
    /const messageTailLayoutState = computed\(\(\) => \[[\s\S]*?props\.isCompressing[\s\S]*?props\.compressionMessage[\s\S]*?props\.queuedMessages[\s\S]*?\]\)/,
    'tail status blocks must participate in message layout tracking'
  )
  assert.match(
    messageList,
    /watch\(\n  \[visibleMessages, collapsedMessages, messageTailLayoutState\][\s\S]*?scrollController\.beforeContentChange\(\)[\s\S]*?nextTick\(\(\) => \{[\s\S]*?syncMessageContentResizeObserver\(\)[\s\S]*?scrollController\.requestContentChange\(\)/,
    'message, queue, and compression changes must share the centralized layout reconciliation path'
  )
})

test('compression indicator counts elapsed seconds while it is running', async () => {
  const [messageList, enLocale, zhHansLocale, zhHantLocale] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/i18n/locales/en.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hant.json', 'utf8').then(JSON.parse)
  ])

  assert.match(
    messageList,
    /<span class="compression-text">\{\{ compressionStatusText \}\}<\/span>/,
    'the compression indicator must render the timed status text'
  )
  assert.match(
    messageList,
    /watch\(\n  \(\) => props\.isCompressing,[\s\S]*?stopCompressionTimer\(\)[\s\S]*?setInterval\(\(\) => \{\n      compressionNow\.value = Date\.now\(\)\n    \}, 1000\)/,
    'the elapsed counter must restart with the backend compression status and tick once per second'
  )
  assert.match(
    messageList,
    /onBeforeUnmount\(stopCompressionTimer\)/,
    'the elapsed counter must stop when the message list unmounts'
  )

  for (const locale of [enLocale, zhHansLocale, zhHantLocale]) {
    assert.match(locale.workflow.compressionElapsed, /\{text\}/)
    assert.match(locale.workflow.compressionElapsed, /\{seconds\}/)
  }
})

test('off-bottom readers preserve a message window anchor while new messages render', async () => {
  const [workflowSessionPane, workflowMessages, messageList] = await Promise.all([
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  ])

  assert.match(
    workflowSessionPane,
    /@message-window-anchor-change="messageProjection\.setMessageWindowAnchor"/,
    'the workflow view must pass the reader anchor from the message list to the projection'
  )
  assert.match(
    workflowMessages,
    /selectVisibleWorkflowMessageWindow\([\s\S]*?loadedTaskMessageCount\.value,[\s\S]*?messageWindowAnchorId\.value/,
    'the upstream message window must retain the reported reader anchor'
  )
  assert.match(
    messageList,
    /:data-window-anchor-id="message\.windowAnchorId \|\| null"/,
    'rendered messages must expose the upstream-stable anchor identity'
  )
  assert.match(
    messageList,
    /scrollController\.onScroll[\s\S]*?scrollController\.onWheel/,
    'message scrolling must use one centralized scroll controller'
  )
  assert.match(
    messageList,
    /scrollController\.onContentResize\(\)/,
    'content resize must be delegated to the centralized scroll controller'
  )
  assert.doesNotMatch(
    messageList,
    /const (?:handleScroll|handleWheel|performScrollToBottom|restoreReadingScrollAnchor|restoreScrollPosition)\s*=|container\.scrollTop\s*[+]?=/,
    'the component must not keep independent scroll mutation or restoration paths'
  )
  assert.doesNotMatch(
    messageList,
    /slice\(-visibleMessageLimit\.value\)/,
    'the message list must not apply a second tail-only window that can discard the reader anchor'
  )
})

test('switching workflows clears an unobserved compression indicator from the previous session', async () => {
  const workflowCore = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')

  assert.match(
    workflowCore,
    /if \(previousWorkflowId && previousWorkflowId !== id\) \{\s*setCompressionStatus\(previousWorkflowId, false, ''\)[\s\S]*?currentSessionId\.value = id/,
    'switching away must discard compression UI state whose completion event is no longer observed'
  )
})

test('workflow completion tool is presented as a finished state', async () => {
  const [enLocale, zhHansLocale, zhHantLocale] = await Promise.all([
    readFile('src/i18n/locales/en.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8').then(JSON.parse),
    readFile('src/i18n/locales/zh-Hant.json', 'utf8').then(JSON.parse)
  ])

  assert.equal(enLocale.workflow.finishTask, 'Task finished')
  assert.equal(zhHansLocale.workflow.finishTask, '任务完成')
  assert.equal(zhHantLocale.workflow.finishTask, '任務完成')
})

test('context snapshots render the v2 handoff contract without losing legacy snapshot support', async () => {
  const [messageList, enLocale, zhHansLocale, zhHantLocale] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/i18n/locales/en.json', 'utf8'),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8'),
    readFile('src/i18n/locales/zh-Hant.json', 'utf8')
  ])

  assert.match(messageList, /const getContextSnapshotV2Kind = snapshot =>/)
  assert.match(messageList, /\['pressure_handoff', 'completed_task_rollup'\]/)
  assert.match(messageList, /const formatV2ContextSnapshot = snapshot =>/)
  assert.match(messageList, /snapshot\.user_directives/)
  assert.match(messageList, /snapshot\.boundary_open_items/)
  assert.match(messageList, /snapshot\.unresolved_carryovers/)
  assert.match(messageList, /snapshot\.completed_work/)
  assert.match(messageList, /snapshot\.file_changes/)
  assert.match(messageList, /snapshot\.review_rounds/)
  assert.match(messageList, /const formatBoundaryOpenItems = items =>/)
  assert.match(messageList, /const formatReviewRound = review =>/)
  assert.match(messageList, /const findings = snapshotMarkdownList\(review\.findings\)/)
  assert.match(messageList, /const requiredFixes = snapshotMarkdownList\(review\.required_fixes\)/)
  assert.match(messageList, /const evidence = snapshotMarkdownList\(review\.evidence_refs\)/)
  assert.match(messageList, /\.filter\(section => section\?\.\[1\]\)/)
  assert.match(messageList, /const v2Display = formatV2ContextSnapshot\(parsed\)/)
  assert.match(messageList, /kind === 'pressure_handoff'/)
  assert.match(messageList, /kind === 'completed_task_rollup'/)
  assert.match(
    messageList,
    /\['Overall Goal', jsonSnapshotSectionText\(parsed\.overall_goal\)\]/,
    'legacy JSON snapshots must retain their formatter'
  )
  assert.match(messageList, /if \(!content\.includes\('<state_snapshot'\)\) return content/)

  for (const locale of [enLocale, zhHansLocale, zhHantLocale]) {
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"pressureHandoff"/)
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"completedTaskRollup"/)
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"reviewStatus"/)
  }
})

test('sandbox profile suggestions inherit the effective scheme runtime preference', async () => {
  const sandboxSettings = await readFile('src/components/setting/Sandbox.vue', 'utf8')

  assert.match(
    sandboxSettings,
    /const effectiveRuntimePreference = profile =>[\s\S]*?profile\?\.runtimePreference && profile\.runtimePreference !== 'auto'[\s\S]*?draft\.value\.config\?\.runtimePreference \|\| 'auto'/,
    'an auto Profile must inherit the scheme runtime preference used by the resolver'
  )
  assert.equal(
    sandboxSettings.match(/runtimeKeys\(effectiveRuntimePreference\(/g)?.length,
    3,
    'image candidates, instance candidates, and image-size tie-breaking must share the effective preference'
  )
  assert.doesNotMatch(
    sandboxSettings,
    /runtimeKeys\(profileDraft\.value\?\.runtimePreference \|\| 'auto'\)/,
    'a Docker scheme with an auto Profile must not present MSB candidates'
  )
})

test('cost analysis interaction is gated by accepted completion and terminal child summaries', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  const styles = await readFile('src/styles/workflow/messages.scss', 'utf8')

  assert.match(messageList, /!message\?\.isApproved \|\| message\?\.toolDisplay\?\.isError/)
  assert.match(messageList, /\['completed', 'failed', 'cancelled', 'interrupted'\]/)
  assert.match(messageList, /getSubAgentCostExpandId/)
  assert.match(messageList, /getFinishTaskCostExpandId/)
  assert.match(messageList, /finish-task-cost-card/)
  assert.doesNotMatch(messageList, /finish-task-display--in-card/)
  assert.match(styles, /margin-left: var\(--cs-space\)/)
  assert.match(styles, /:hover \.finish-task-cost-arrow/)
})

test('message history loading renders an Element Plus skeleton from the store loading state', async () => {
  const [messageList, workflowSessionPane, workflowStore, styles] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/stores/workflow.js', 'utf8'),
    readFile('src/styles/workflow/messages.scss', 'utf8')
  ])

  assert.match(messageList, /v-if="props\.isLoading" class="message-skeleton"/)
  assert.match(messageList, /<el-skeleton animated>/)
  assert.match(workflowSessionPane, /:is-loading="isLoadingMessages \|\| \(agentRole === 'primary' && isInitializing\)"/)
  assert.match(
    workflowStore,
    /const requestRevision = \+\+messageLoadRevision;\s*isLoadingMessages\.value = true;/
  )
  assert.match(
    workflowStore,
    /messages\.value = appendMissingPendingToolMessages\([\s\S]*?isLoadingMessages\.value = false;/
  )
  assert.match(styles, /\.message-skeleton/)
})

test('workflow startup uses the primary message skeleton without covering the window', async () => {
  const [workflowView, workflowSessionPane] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8')
  ])

  assert.match(workflowView, /const isInitializing = ref\(true\)/)
  assert.match(workflowView, /agent-role="primary"\s+:is-initializing="isInitializing"/)
  assert.match(workflowView, /isInitializing\.value = false/)
  assert.doesNotMatch(workflowView, /workflow-startup-overlay/)
  assert.match(workflowSessionPane, /isInitializing: \{ type: Boolean, default: false \}/)
  assert.match(workflowSessionPane, /:is-loading="isLoadingMessages \|\| \(agentRole === 'primary' && isInitializing\)"/)
})

test('workflow runtime banner stays hidden during startup and only displays actual failures', async () => {
  const source = await readFile('src/views/Workflow.vue', 'utf8')
  assert.match(source, /<el-alert v-if="runtimeMessage" :title="runtimeMessage"/)
  const start = source.indexOf('const runtimeMessage = computed(')
  const end = source.indexOf('\nconst isInitializing', start)
  assert.ok(start >= 0 && end > start)
  // A mounted view may still hold the composable result from before a hot update.
  const state = { status: { value: null } }
  const error = { value: null }
  const message = runInNewContext(
    `${source.slice(start, end)}\n;runtimeMessage`,
    { computed: getter => ({ get value() { return getter() } }), runtime: state, runtimeError: error, t: key => key }
  )
  for (const status of [null, { state: 'disconnected' }, { state: 'spawned' }, { state: 'attached', heartbeat_ok: true }]) {
    state.status.value = status
    assert.equal(message.value, '', 'normal startup and hydration use only the skeleton')
  }
  for (const status of [
    { state: 'unavailable' },
    { state: 'disconnected', last_error: 'connection failed' },
    { state: 'attached', heartbeat_ok: false }
  ]) {
    state.status.value = status
    assert.equal(message.value, 'chat.runtimeUnavailable')
  }
  state.status.value = { state: 'attached', heartbeat_ok: true }
  error.value = new Error('load failed')
  assert.equal(message.value, 'chat.runtimeLoadFailed')
  error.value = null
  state.statusError = { value: new Error('status request failed') }
  assert.equal(message.value, 'chat.runtimeUnavailable')
  state.statusError.value = null
  assert.equal(message.value, '', 'the error banner disappears after recovery')
})

test('tool duration badges use structured backend metadata and preserve the requested exclusions', async () => {
  const [messageList, workflowEngine] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src-tauri/crates/backend/src/workflow/react/engine.rs', 'utf8')
  ])

  assert.match(messageList, /message\?\.metadata\?\.duration_ms/)
  assert.match(messageList, /toolName\.startsWith\('todo_'\)/)
  assert.match(messageList, /\['ask_user', 'submit_plan', 'submit_result', 'skill'\]/)
  assert.match(messageList, /durationMs <= 60_000/)
  assert.match(messageList, /\$\{minutes\}m\$\{seconds\}s/)
  assert.match(messageList, /const approvalStatus = String\(message\?\.metadata\?\.approval_status \|\| ''\)\.toLowerCase\(\)/)
  assert.match(
    messageList,
    /message\.isApproved && !\['ask_user', 'submit_plan'\]\.includes\(getMessageToolName\(message\)\)[\s\S]*?name="check"/
  )
  assert.match(workflowEngine, /metadata\["duration_ms"\]\s*=\s*serde_json::json!\(duration_ms\)/)
  assert.match(workflowEngine, /fn should_expose_tool_duration\(tool_name: &str\)/)
})

test('persisted awaiting-user status restores answer controls when an older snapshot lacks wait_reason', async () => {
  const workflowStore = await readFile('src/stores/workflow.js', 'utf8')

  assert.match(
    workflowStore,
    /const persistedWaitReason =[\s\S]*?waitReason\.value =[\s\S]*?persistedWaitReason \|\|[\s\S]*?status === WORKFLOW_STATUSES\.AWAITING_USER[\s\S]*?WORKFLOW_WAIT_REASONS\.USER_INPUT/
  )
})

test('workflow markdown links use the default browser opener', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  const markdownInstances = messageList.match(/<MarkdownSimple\b[^>]*>/g) || []

  assert.ok(markdownInstances.length > 0, 'the workflow message list must render MarkdownSimple content')
  assert.ok(
    markdownInstances.every(instance => /\bv-link\b/.test(instance)),
    'every workflow MarkdownSimple instance must open links through the default browser'
  )
})

test('tool activity grouping keeps only explicit independent segments as boundaries', async () => {
  const [messageList, workflowMessages, projectionRules] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
    readFile('src/composables/workflow/messageProjectionRules.js', 'utf8')
  ])

  assert.match(
    projectionRules,
    /const currentIsThought = isThinkOnlyAssistantMessage\(current\)[\s\S]*?collapsed\.push\(buildToolGroupMessage\(\[\], index, thoughts, true\)\)/,
    'a thought without a prior visible tool group can become its own ongoing group'
  )
  assert.match(
    projectionRules,
    /currentIsThought && isStandaloneOngoingThoughtRun\(input, index\)/,
    'a thought followed by later tool activity must start grouped instead of flashing outside the tool group'
  )
  assert.match(
    projectionRules,
    /if \(getCollapsibleToolGroupKind\(message\) \|\| isCollapsedWorkflowToolGroupMessage\(message\)\) \{\s*return false/,
    'standalone ongoing thought groups must stop being standalone once tool activity appears after them'
  )
  assert.match(
    projectionRules,
    /while \(nextIndex < input\.length && !isToolGroupBoundaryMessage\(input\[nextIndex\]\)\)/,
    'non-independent thoughts and tools must be collected into one tool group until a boundary appears'
  )
  assert.match(
    projectionRules,
    /if \(isCompletionReportMessage\(message\) \|\| isWorkflowCompletionMessage\(message\)\) return true/,
    'completion tool messages must split tool groups so the finish-task badge remains visible after history is expanded'
  )
  assert.doesNotMatch(
    projectionRules,
    /isCollapsedWorkflowToolGroupMessage\(current\) && current\.metadata\?\.tool_group_is_ongoing/,
    'ongoing thought groups must remain eligible to merge back into adjacent non-independent tool activity'
  )
  assert.match(
    projectionRules,
    /!hasVisibleWorkflowText\(removeSystemReminder\(message\?\.message \|\| ''\)\) &&\s*hasVisibleWorkflowText\(message\?\.reasoning \|\| ''\)/,
    'only assistant messages without visible content may be treated as thought-only tool activity'
  )
  assert.doesNotMatch(
    messageList,
    /return isThinkStep \? !!\(content \|\| reasoning\) : !content && !!reasoning/,
    'stepType Think must not make visible assistant output collapse into a tool group'
  )
  assert.match(
    projectionRules,
    /if \(message\.role === 'assistant'\) \{\s*return \(\s*!isThinkOnlyAssistantMessage\(message\) &&\s*hasVisibleWorkflowText\(removeSystemReminder\(message\?\.message \|\| ''\)\)/,
    'a visible non-thought assistant text message must still split tool groups, while thought text does not'
  )
  assert.match(
    projectionRules,
    /const getToolGroupIcon = \(kind, groupMessages\) => \{\s*if \(groupMessages\.some\(isCollapsibleMutationToolMessage\)\) return TOOL_GROUP_ICONS\.mutation_tools/,
    'a mixed tool group containing file edits must prefer the edit icon'
  )
  assert.match(
    messageList,
    /const isStreamingThoughtMergedIntoToolGroup = message =>[\s\S]*?hasStreamingThoughtOnly\.value[\s\S]*?message === lastVisibleMessage\.value[\s\S]*?isCollapsedToolGroupMessage\(message\)[\s\S]*?\(message\.groupedTools\?\.length \|\| 0\) > 0/,
    'a reasoning-only stream must attach only to the final tool group, never an independent message or thought-only group'
  )
  assert.match(
    messageList,
    /const shouldShowStandaloneStreamingChat = computed\([\s\S]*?!isStreamingThoughtMergedIntoToolGroup\(lastVisibleMessage\.value\)/,
    'the standalone streaming card must stay hidden only while its thought is attached to the preceding tool group'
  )
  assert.match(
    messageList,
    /v-if="isStreamingThoughtMergedIntoToolGroup\(message\)"[\s\S]*?STREAMING_REASONING_ID/,
    'an attached streaming thought must retain its expandable reasoning UI inside the tool group'
  )
  assert.match(
    workflowMessages,
    /const toolGroupOrderById = new Map\(\)[\s\S]*?rawMsgs\.forEach\(\(message, messageIndex\) => \{[\s\S]*?toolGroupOrderById\.set\(toolCallId, messageIndex \+ \(callIndex \+ 1\) \/ \(callCount \+ 1\)\)/,
    'tool-call declaration order must be recorded before assistant tool messages can be hidden'
  )
  assert.match(
    workflowMessages,
    /id: call\.id,[\s\S]*?groupOrder:[\s\S]*?toolGroupOrderById\.get\(getToolCallId\(call\)\)/,
    'pending tool placeholders must carry their original tool-call order'
  )
  assert.match(
    workflowMessages,
    /message\.role === 'tool'[\s\S]*?toolGroupOrderById\.get\(String\(message\.metadata\?\.tool_call_id \|\| ''\)\.trim\(\)\) \?\? message\.sourceOrder/,
    'completed tool messages must carry the same original order even after their assistant declaration is filtered'
  )
  assert.match(
    projectionRules,
    /call\.groupOrder \?\? index \+ \(callIndex \+ 1\) \/ \(pendingCalls\.length \+ 1\)/,
    'the message projection must consume the stable pending-tool order and only fall back for old data'
  )
  assert.match(
    projectionRules,
    /tools\.push\(\{ \.\.\.message, groupOrder: message\.groupOrder \?\? index \}\)/,
    'tool collection must not overwrite a stable order with a transient projection index'
  )
  assert.match(
    workflowMessages,
    /const processedMsgs = rawMsgs\.map\(\(m, sourceOrder\) => \{[\s\S]*?sourceOrder \}/,
    'each live message must retain its durable transcript position through the display projection'
  )
  assert.match(
    workflowMessages,
    /message\.role === 'tool'[\s\S]*?\?\? message\.sourceOrder[\s\S]*?: message\.groupOrder \?\? message\.sourceOrder/,
    'thoughts and tools must share the same durable order axis while tool calls retain declaration order'
  )
  assert.match(
    projectionRules,
    /thoughts\.push\([\s\S]*?thought\.groupOrder \?\? index \+ thoughtIndex \/ 1000/,
    'nested tool groups must retain each thought’s durable order instead of replacing it with a transient group index'
  )
  assert.match(
    projectionRules,
    /thoughts\.push\(buildGroupedThoughtItem\(message, index, message\.groupOrder \?\? index\)\)/,
    'grouped thoughts must use their durable order instead of the transient filtered-list index'
  )
  assert.match(
    projectionRules,
    /const \{ thoughtOrders, toolOrders \} = getWorkflowToolGroupRenderOrders\(thoughts, groupMessages\)[\s\S]*?renderOrder: thoughtOrders\[thoughtIndex\][\s\S]*?renderOrder: toolOrders\[toolIndex\]/,
    'thoughts and tools must be assigned integer render ranks from their shared durable order axis'
  )
  assert.match(
    messageList,
    /:style="\{ order: thought\.renderOrder \?\? thoughtIndex \}"[\s\S]*?:style="\{ order: tool\.renderOrder \?\? toolIndex \}"/,
    'the expanded tool group must pass only integer render ranks to CSS flex order'
  )
})
