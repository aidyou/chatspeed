import test from 'node:test'
import assert from 'node:assert/strict'
import { effectScope } from 'vue'
import { readFileSync } from 'node:fs'
import { isRuntimeInitialized, isRuntimeReady, useRuntimeStatus } from './useRuntimeStatus.js'

const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
const ready = generation => ({
  state: 'attached', heartbeat_ok: null, generation, ready_generation: generation
})
function mount(invoke, refresh) {
  const scope = effectScope()
  const runtime = scope.run(() => useRuntimeStatus(invoke, refresh, { interval: 5, retryDelay: 5 }))
  runtime.start()
  return { runtime, stop: () => scope.stop() }
}

test('readiness uses authoritative state and heartbeat', () => {
  for (const state of ['disconnected', 'released', 'unavailable']) {
    assert.equal(isRuntimeReady({ state, heartbeat_ok: true }), false)
  }
  assert.equal(isRuntimeReady(ready(1)), true)
  assert.equal(isRuntimeInitialized(ready(1)), true)
  assert.equal(isRuntimeInitialized({ ...ready(1), ready_generation: 0 }), false)
  assert.equal(isRuntimeReady({ state: 'spawned', heartbeat_ok: false }), false)

})

test('refreshes once on first ready, generation change and same-generation recovery', async () => {
  let status = { state: 'disconnected' }
  let loads = 0
  const app = mount(async () => status, async () => { loads++ })
  try {
    await wait(20)
    assert.equal(loads, 0)
    status = ready(1)
    await wait(20)
    assert.equal(loads, 1)
    assert.equal(app.runtime.usable.value, true)
    status = ready(2)
    await wait(20)
    assert.equal(loads, 2)
    status = { state: 'disconnected' }
    await wait(20)
    assert.equal(app.runtime.usable.value, false)
    status = ready(2)
    await wait(20)
    assert.equal(loads, 3)
  } finally { app.stop() }
})

test('status request failures are exposed separately from normal startup and clear on recovery', async () => {
  let fail = true
  const app = mount(async () => {
    if (fail) throw new Error('status request failed')
    return ready(1)
  }, async () => {})
  try {
    await wait(20)
    assert.equal(app.runtime.status.value, null)
    assert.ok(app.runtime.statusError.value)
    assert.equal(app.runtime.error.value, null)
    fail = false
    await wait(20)
    assert.equal(app.runtime.statusError.value, null)
    assert.equal(app.runtime.usable.value, true)
  } finally { app.stop() }
})

test('failed data hydration is not empty success and retries while still ready', async () => {
  let fail = true
  const app = mount(async () => ready(1), async () => {
    if (fail) throw new Error('load failed')
  })
  try {
    await wait(20)
    assert.ok(app.runtime.error.value)
    assert.equal(app.runtime.usable.value, false)
    fail = false
    await wait(20)
    assert.equal(app.runtime.error.value, null)
    assert.equal(app.runtime.usable.value, true)
  } finally { app.stop() }
})

test('hydration failures back off, cap retries and reset immediately for a new generation', async t => {
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 })
  let status = ready(1)
  let loads = 0
  let polls = 0
  let fail = true
  const scope = effectScope()
  const runtime = scope.run(() => useRuntimeStatus(async () => { polls++; return status }, async () => {
    loads++
    if (fail) throw new Error('snapshot offline')
  }, { maxRetryDelay: 4000 }))
  const advance = async ms => {
    t.mock.timers.tick(ms)
    await new Promise(resolve => setImmediate(resolve))
  }
  runtime.start()
  try {
    await advance(0)
    assert.equal(loads, 1)
    await advance(1000)
    assert.equal(loads, 1, 'status polling does not bypass hydration backoff')
    await advance(1000)
    assert.equal(loads, 2)
    await advance(3000)
    assert.equal(loads, 2)
    await advance(1000)
    assert.equal(loads, 3)
    await advance(4000)
    assert.equal(loads, 4, 'retry delay is capped')
    assert.ok(polls > loads, 'health polling stays independent')
    status = ready(2)
    fail = false
    await advance(1000)
    assert.equal(loads, 5, 'a new runtime does not inherit the old failure delay')
    assert.equal(runtime.error.value, null)
    assert.equal(runtime.usable.value, true)
    await advance(4000)
    assert.equal(loads, 5, 'successful hydration stays single-shot')
    status = { state: 'unavailable' }
    await advance(1000)
    status = ready(2)
    fail = true
    await advance(1000)
    assert.equal(loads, 6)
    await advance(1000)
    assert.equal(loads, 6)
    await advance(1000)
    assert.equal(loads, 7, 'recovery resets the failure count')
    scope.stop()
    const count = polls
    await advance(30000)
    assert.equal(polls, count, 'disposal cancels pending retries')
  } finally { scope.stop() }
})

test('background snapshot failure propagates to the banner without a toast; manual selection still notifies', async () => {
  const source = readFileSync(new URL('./workflow/useWorkflowCore.ts', import.meta.url), 'utf8')
  const body = source.split('const selectWorkflow = async ')[1].split('\n    const startNewWorkflow')[0]
  const failure = new Error('snapshot transport failed')
  const messages = []
  const select = new Function('canSwitchWorkflow', 'workflowStore', 'currentSessionId', 'safeExecute',
    'resetChatState', 'clearRetryTimer', 'showMessage', 't', 'console', `return async ${body}`)(
    { value: true }, { selectWorkflow: async () => { throw failure } }, { value: null }, fn => fn(),
    () => {}, () => {}, (...args) => messages.push(args), key => key, { error() {} })
  for (let i = 0; i < 3; i++) {
    await assert.rejects(select('chosen', { background: true }), error => error === failure)
  }
  assert.equal(messages.length, 0)
  await select('chosen')
  assert.equal(messages.length, 1)
  assert.equal(messages[0][1], 'error')
})

test('background workflow bootstrap failures also propagate without toasts', async () => {
  const source = readFileSync(new URL('./workflow/useWorkflowCore.ts', import.meta.url), 'utf8')
  const body = source.split('const createNewWorkflow = async ')[1].split('\n    const toggleFinalAuditMode')[0]
  const failure = new Error('create transport failed')
  const messages = []
  const create = new Function('selectedAgent', 'workflowStore', 'getCurrentWorkflowAgentConfig',
    'invokeWrapper', 'showMessage', 'console', 't', `return async ${body}`)(
    { value: { id: 'agent' } }, {}, () => ({}), async () => { throw failure },
    (...args) => messages.push(args), { error() {} }, key => key)
  await assert.rejects(create({ background: true }), error => error === failure)
  assert.equal(messages.length, 0)
  assert.equal(await create(), false)
  assert.equal(messages.length, 1)
})

test('poll and hydration stay single-flight; restart invalidates slow work; disposal cancels polling', async () => {
  let status = ready(1)
  let polls = 0
  let active = 0
  let maxActive = 0
  let release
  let oldIsCurrent
  let loads = 0
  const app = mount(async () => {
    polls++
    active++
    maxActive = Math.max(active, maxActive)
    await wait(8)
    active--
    return status
  }, async isCurrent => {
    loads++
    if (loads === 1) {
      oldIsCurrent = isCurrent
      await new Promise(resolve => { release = resolve })
    }
  })
  await wait(25)
  status = ready(2)
  await wait(30)
  assert.equal(loads, 1)
  assert.equal(oldIsCurrent(), false)
  release()
  await wait(30)
  assert.equal(loads, 2)
  assert.equal(maxActive, 1)
  app.stop()
  const count = polls
  await wait(30)
  assert.equal(polls, count)
})

test('workflow recovery executes snapshot/SSE selection again and never bootstraps on list failure', async () => {
  const source = readFileSync(new URL('../views/Workflow.vue', import.meta.url), 'utf8')
  const body = source.split('async function refreshRuntimeData(isCurrent) {')[1].split('\nonMounted(')[0]
  const calls = []
  const workflowStore = { currentWorkflowId: 'chosen', error: null,
    loadWorkflows: async () => { calls.push('list') } }
  const listeners = { value: null }
  const refresh = new Function('modelStore', 'workflowStore', 'workflowAutomationStore', 'agentStore',
    'resolveInitialWorkflowId', 'selectWorkflow', 'unlistenWorkflowEvents', 'lastHistoryWorkflowId',
    'coreCreateNewWorkflow', `return async function(isCurrent) {${body}`)(
    { updateModelStore: async () => { calls.push('models') } }, workflowStore,
    { fetchAutomations: async () => { calls.push('automations') } },
    { agents: [{}], fetchAgents: async () => { calls.push('agents') } }, () => null,
    async (id, options) => {
      assert.deepEqual(options, { background: true })
      calls.push(`snapshot/SSE:${id}`)
      listeners.value = () => {}
    },
    listeners, { value: null }, async () => { calls.push('create') })
  let status = ready(1)
  const app = mount(async () => status, refresh)
  try {
    await wait(20)
    status = ready(2)
    await wait(20)
    assert.equal(calls.filter(call => call === 'snapshot/SSE:chosen').length, 2)
    assert.equal(calls.filter(call => call === 'models').length, 2)
    assert.equal(calls.includes('create'), false)
    workflowStore.currentWorkflowId = null
    workflowStore.loadWorkflows = async () => { throw new Error('offline') }
    status = ready(3)
    await wait(20)
    assert.equal(app.runtime.usable.value, false)
    assert.equal(calls.includes('create'), false)
  } finally { app.stop() }
})

test('chat recovery reloads models, conversations and selected messages without replacing input', async () => {
  const source = readFileSync(new URL('../views/Index.vue', import.meta.url), 'utf8')
  const body = source.split('const runtime = useRuntimeStatus(invokeWrapper, async isCurrent => {')[1]
    .split('\n})\nconst { usable:')[0]
  const calls = []
  const chat = { currentConversationId: 7,
    loadConversations: async () => { calls.push('conversations') },
    getCurrentConversationId: async () => {},
    loadMessages: async id => { calls.push(`messages:${id}`) } }
  const refresh = new Function('modelStore', 'chatStore', 'messageReady', 'resetPagination', 'settingStore',
    `return async function(isCurrent) {${body}}`)(
    { updateModelStore: async () => { calls.push('models') } }, chat, { value: true }, () => {}, { windowLabel: 'main' })
  let status = ready(1)
  const app = mount(async () => status, refresh)
  try {
    await wait(20)
    status = { state: 'unavailable' }
    await wait(20)
    chat.currentConversationId = 9
    status = ready(1)
    await wait(20)
    assert.deepEqual(calls, ['conversations', 'messages:7', 'conversations', 'messages:9'])
    assert.equal(chat.currentConversationId, 9)
  } finally { app.stop() }
})

test('views refresh through existing data and SSE paths without clearing drafts', () => {
  const workflow = readFileSync(new URL('../views/Workflow.vue', import.meta.url), 'utf8')
  const section = workflow.split('async function refreshRuntimeData(isCurrent) {')[1].split('onMounted(')[0]
  for (const call of ['workflowStore.loadWorkflows()',
    'workflowAutomationStore.fetchAutomations()', 'agentStore.fetchAgents()', 'selectWorkflow(sessionId, { background: true })']) {
    assert.ok(section.includes(call), call)
  }
  assert.ok(section.includes('workflowStore.currentWorkflowId || resolveInitialWorkflowId()'))
  assert.doesNotMatch(section, /clearInput|clearImageAttachments|inputMessage.value\s*=/)
  assert.ok(section.indexOf('await workflowStore.loadWorkflows()') < section.indexOf('await coreCreateNewWorkflow({ background: true })'))
  const chat = readFileSync(new URL('../views/Index.vue', import.meta.url), 'utf8')
  const refresh = chat.split('const runtime = useRuntimeStatus')[1].split('const { usable:')[0]
  assert.doesNotMatch(refresh, /modelStore\.updateModelStore\(\)/)
  assert.match(refresh, /await chatStore.loadConversations\(\)/)
  assert.match(refresh, /await chatStore.loadMessages\(chatStore.currentConversationId/)
  assert.doesNotMatch(refresh, /inputMessage.value\s*=|newChat\(/)
})
