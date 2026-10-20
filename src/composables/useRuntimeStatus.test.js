import test from 'node:test'
import assert from 'node:assert/strict'
import { effectScope } from 'vue'
import { readFileSync } from 'node:fs'
import { isRuntimeReady, useRuntimeStatus } from './useRuntimeStatus.js'

const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
const ready = generation => ({ state: 'attached', heartbeat_ok: null, generation })
function mount(invoke, refresh) {
  const scope = effectScope()
  const runtime = scope.run(() => useRuntimeStatus(invoke, refresh, { interval: 5 }))
  runtime.start()
  return { runtime, stop: () => scope.stop() }
}

test('readiness uses authoritative state and heartbeat', () => {
  for (const state of ['disconnected', 'released', 'unavailable']) {
    assert.equal(isRuntimeReady({ state, heartbeat_ok: true }), false)
  }
  assert.equal(isRuntimeReady(ready(1)), true)
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
    async id => { calls.push(`snapshot/SSE:${id}`); listeners.value = () => {} },
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
    assert.deepEqual(calls, ['models', 'conversations', 'messages:7', 'models', 'conversations', 'messages:9'])
    assert.equal(chat.currentConversationId, 9)
  } finally { app.stop() }
})

test('views refresh through existing data and SSE paths without clearing drafts', () => {
  const workflow = readFileSync(new URL('../views/Workflow.vue', import.meta.url), 'utf8')
  const section = workflow.split('async function refreshRuntimeData(isCurrent) {')[1].split('onMounted(')[0]
  for (const call of ['modelStore.updateModelStore()', 'workflowStore.loadWorkflows()',
    'workflowAutomationStore.fetchAutomations()', 'agentStore.fetchAgents()', 'selectWorkflow(sessionId)']) {
    assert.ok(section.includes(call), call)
  }
  assert.ok(section.includes('workflowStore.currentWorkflowId || resolveInitialWorkflowId()'))
  assert.doesNotMatch(section, /clearInput|clearImageAttachments|inputMessage.value\s*=/)
  assert.ok(section.indexOf('await workflowStore.loadWorkflows()') < section.indexOf('await coreCreateNewWorkflow()'))
  const chat = readFileSync(new URL('../views/Index.vue', import.meta.url), 'utf8')
  const refresh = chat.split('const runtime = useRuntimeStatus')[1].split('const { usable:')[0]
  assert.match(refresh, /await chatStore.loadConversations\(\)/)
  assert.match(refresh, /await chatStore.loadMessages\(chatStore.currentConversationId/)
  assert.doesNotMatch(refresh, /inputMessage.value\s*=|newChat\(/)
})
