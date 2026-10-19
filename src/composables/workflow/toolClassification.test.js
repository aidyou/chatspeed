import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { computed, reactive, ref, nextTick, watch } from 'vue'

import {
  getWorkflowToolFamily,
  isWorkflowMcpTool,
  isWorkflowTodoTool
} from './toolClassification.js'

for (const toolName of ['todo_create', 'todo_list', 'todo_update']) {
  assert.equal(isWorkflowTodoTool(toolName), true, `${toolName} must be an exact Todo tool`)
  assert.equal(getWorkflowToolFamily(toolName), 'todo')
}

for (const toolName of ['sub_agent_run', 'sub_agent_output']) {
  assert.equal(getWorkflowToolFamily(toolName), 'task')
}

for (const toolName of [
  'web_fetch',
  'web_search',
  'server__MCP__search',
  'SERVER__mcp__WRITE',
  'mcp_tool_expand',
  'mcp_tool_execute',
  'mcp_tool_load'
]) {
  assert.equal(isWorkflowMcpTool(toolName), true, `${toolName} must be classified as MCP`)
}

assert.equal(isWorkflowMcpTool('codegraph_callees', 'MCP'), true, 'MCP aliases use category metadata')

for (const toolName of ['mcp', 'mcp_search', 'server_mcp_search', 'load_mcp_tool']) {
  assert.equal(isWorkflowMcpTool(toolName), false, `${toolName} must not be inferred as MCP`)
}

for (const toolName of [
  'todo',
  'todo_archive',
  'todoist_import',
  'create_file',
  'task',
  'taskmaster',
  'sub_agent',
  'sub_agent_custom'
]) {
  assert.equal(isWorkflowTodoTool(toolName), false, `${toolName} must not be inferred as Todo`)
  assert.equal(getWorkflowToolFamily(toolName), null, `${toolName} must remain unclassified`)
}

const inputSource = readFileSync(new URL('../../components/workflow/WorkflowInputArea.vue', import.meta.url), 'utf8')
const sourceSection = (source, start, end) => {
  const from = source.indexOf(start)
  const to = source.indexOf(end, from)
  assert.ok(from >= 0 && to > from)
  return source.slice(from, to)
}

test('workflow catalog keeps native and MCP projections separate', () => {
  const nativeProjection = sourceSection(inputSource, 'const agentAvailableTools = computed(() => {', 'const workflowMcpTools = computed(() => {')
  const mcpProjection = sourceSection(inputSource, 'const workflowMcpTools = computed(() => {', 'const workflowAvailableToolIds = computed(() => {')
  assert.match(nativeProjection, /\.filter\(id => !isMcpToolId\(id\)\)/)
  assert.match(mcpProjection, /currentlyAvailableMcpTools = new Set\(/)
  assert.match(mcpProjection, /tool\.category === 'MCP'/)
  assert.doesNotMatch(mcpProjection, /\.filter\(id => currentlyAvailableMcpTools\.has\(id\)\)/)
  assert.match(mcpProjection, /configuredMcpTools = new Set\(\[\.\.\.available, \.\.\.autoApprove, \.\.\.autoExpand\]\)/)

  const agentSource = readFileSync(new URL('../../components/setting/Agent.vue', import.meta.url), 'utf8')
  assert.match(agentSource, /!isWorkflowMcpTool\(t\.id, t\.category\)/)
  assert.match(agentSource, /const mcpToolOptions = computed\(\(\) => \{[\s\S]*?tool\.category === 'MCP'/)
})

test('MCP panel refreshes late discovery, avoids overlapping reads and stops polling on close', async () => {
  const visible = ref(false)
  let refreshes = 0
  let stateReads = 0
  let tick
  let cleared = false
  let release
  const gate = new Promise(resolve => { release = resolve })
  const stop = runInNewContext(
    sourceSection(inputSource, 'let mcpPanelRefreshing = false', "const approvalToolsTab = ref('available')"),
    {
      watch: (...args) => watch(...args), mcpConfigPopoverVisible: visible,
      agentStore: { fetchAvailableTools: async () => { refreshes++; await gate } },
      capabilityStore: { loadMcpServers: async () => { stateReads++; await gate } },
      console, setInterval: callback => { tick = callback; return 1 },
      clearInterval: () => { cleared = true }
    }
  )
  try {
    assert.equal(refreshes, 0)
    visible.value = true
    await nextTick()
    assert.equal(refreshes, 1)
    await tick()
    assert.equal(refreshes, 1, 'slow discovery must not pile up reads')
    assert.equal(stateReads, 1)
    const closedTick = tick
    visible.value = false
    await nextTick()
    assert.equal(cleared, true)
    await closedTick()
    visible.value = true
    await nextTick()
    assert.equal(refreshes, 1, 'reopening must not overlap a read still in flight')
    release()
    await new Promise(resolve => setImmediate(resolve))
    await tick()
    assert.equal(refreshes, 2)
    visible.value = false
    await nextTick()
    assert.equal(cleared, true)
    await tick()
    assert.equal(refreshes, 2, 'closed callbacks must not restart reads')
    assert.equal(stateReads, 2)
  } finally {
    release()
    stop()
  }
})

test('an older tool catalog read cannot overwrite a newer catalog or surface a stale failure', async () => {
  const storeSource = readFileSync(new URL('../../stores/agent.js', import.meta.url), 'utf8')
  const reads = []
  const availableTools = ref([])
  const loading = ref(false)
  const error = ref(null)
  const fetchCatalog = runInNewContext(
    sourceSection(storeSource, '  let toolCatalogRevision = 0;', '  const getAgent = async') + '\nfetchAvailableTools',
    {
      availableTools, loading, error,
      invokeWrapper: () => new Promise((resolve, reject) => reads.push({ resolve, reject })),
      _handleError: failure => { error.value = failure.message; throw failure }
    }
  )
  const initial = fetchCatalog()
  const refresh = fetchCatalog()
  const tools = [{ id: 'chatspeed_web__MCP__web_fetch', category: 'MCP' }]
  reads[1].resolve(tools)
  await refresh
  reads[0].resolve([])
  await initial
  assert.equal(availableTools.value[0].id, tools[0].id)
  const stale = fetchCatalog()
  const latest = fetchCatalog()
  reads[3].resolve(tools)
  await latest
  reads[2].reject(new Error('old startup failure'))
  await stale
  assert.equal(error.value, null)
  assert.equal(loading.value, false)
  const failed = fetchCatalog()
  const rejected = assert.rejects(failed, /catalog offline/)
  reads[4].reject(new Error('catalog offline'))
  await rejected
  assert.equal(availableTools.value.length, 0, 'failed catalog reads cannot keep stale effective capabilities')
})

const createMcpProjection = () => {
  const id = 'server__MCP__search'
  const config = { available: [id], autoApprove: [id], autoExpand: [id] }
  const props = reactive({ currentWorkflowId: 'session', currentWorkflow: { agentConfig: { mcpTools: config, availableTools: [] } } })
  const agentStore = reactive({ availableTools: [] })
  const capabilityStore = reactive({ mcpViews: {} })
  const projection = runInNewContext(
    sourceSection(inputSource, 'const getMcpToolState =', 'const autoApprovedTools = computed') +
      '\n;({ workflowMcpTools, workflowAvailableToolIds, mcpToolStatePresentation })',
    { props, agentStore, capabilityStore, computed, agentAvailableTools: computed(() => []),
      getMcpToolServerName: ({ id }) => id.split('__MCP__')[0] }
  )
  return { id, props, agentStore, capabilityStore, ...projection }
}

test('configured missing MCP rows retain permissions but never become effective until late reads recover', () => {
  const p = createMcpProjection()
  const before = JSON.stringify(p.props.currentWorkflow.agentConfig)
  assert.equal(p.workflowMcpTools.value.length, 1)
  assert.equal(p.workflowMcpTools.value[0].id, p.id)
  assert.equal(p.workflowMcpTools.value[0].available, true)
  assert.equal(p.workflowMcpTools.value[0].autoApprove, true)
  assert.equal(p.workflowMcpTools.value[0].autoExpand, true)
  assert.equal(p.workflowMcpTools.value[0].state, 'unknown')
  assert.equal(p.workflowAvailableToolIds.value.length, 0)
  p.capabilityStore.mcpViews = { 1: { name: 'server', desired: { enabled: true }, runtime: { observed: true, state: 'running' }, tools: { freshness: 'observed' } } }
  assert.equal(p.workflowMcpTools.value[0].state, 'unavailable')
  p.agentStore.availableTools = [{ id: p.id, category: 'MCP', name: 'search' }]
  assert.equal(p.workflowMcpTools.value[0].effective, true)
  assert.equal(p.workflowAvailableToolIds.value[0], p.id)
  for (const [state, observed, enabled, expected] of [
    ['error', true, true, 'error'], ['starting', true, true, 'starting'],
    ['connected', true, true, 'connected'], ['stopped', true, true, 'unavailable'],
    ['running', false, true, 'unknown'], ['running', true, false, 'disabled'],
    ['unexpected', true, true, 'unknown']
  ]) {
    p.capabilityStore.mcpViews[1].runtime = { state, observed }
    p.capabilityStore.mcpViews[1].desired.enabled = enabled
    assert.equal(p.workflowMcpTools.value[0].state, expected)
    assert.equal(p.workflowAvailableToolIds.value.length, 0)
  }
  p.capabilityStore.mcpViews = {}
  p.agentStore.availableTools = []
  assert.equal(JSON.stringify(p.props.currentWorkflow.agentConfig), before, 'read projections never persist empty permissions')
})

test('missing MCP capability rejects grants but allows explicit revocation without clearing other IDs', async () => {
  const p = createMcpProjection()
  const other = 'other__MCP__keep'
  for (const key of ['available', 'autoApprove', 'autoExpand']) p.props.currentWorkflow.agentConfig.mcpTools[key].push(other)
  const saves = []
  const toggle = runInNewContext(
    sourceSection(inputSource, 'const toggleWorkflowMcpConfig =', 'const toggleAutoApprovedTool =') + '\ntoggleWorkflowMcpConfig',
    { props: p.props, workflowMcpTools: p.workflowMcpTools,
      persistAgentConfig: async config => { saves.push(config); p.props.currentWorkflow.agentConfig.mcpTools = config.mcpTools },
      workflowStore: { setAutoApprovedTools: () => {} }, console }
  )
  for (const key of ['available', 'autoApprove', 'autoExpand']) await toggle(p.id, key, true)
  assert.equal(saves.length, 0)
  await toggle(p.id, 'autoApprove', false)
  assert.equal(saves.length, 1)
  assert.equal(saves[0].mcpTools.available.includes(p.id), true)
  assert.equal(saves[0].mcpTools.autoApprove.includes(p.id), false)
  await toggle(p.id, 'available', false)
  for (const key of ['available', 'autoApprove', 'autoExpand']) {
    assert.equal(saves[1].mcpTools[key].includes(p.id), false)
    assert.equal(saves[1].mcpTools[key].includes(other), true)
  }
})

test('native tool toggles retain configured MCP preferences during an outage', async () => {
  const p = createMcpProjection()
  const saves = []
  const toggle = runInNewContext(
    sourceSection(inputSource, 'const toggleWorkflowAvailableTool =', 'const toggleWorkflowMcpConfig =') + '\ntoggleWorkflowAvailableTool',
    { props: p.props, workflowMcpTools: p.workflowMcpTools,
      workflowAvailableToolIds: p.workflowAvailableToolIds, autoApprovedTools: computed(() => []),
      persistAgentConfig: async config => saves.push(config),
      workflowStore: { setAutoApprovedTools: () => {} }, console }
  )
  assert.equal(p.workflowAvailableToolIds.value.length, 0)
  await toggle('read_file', true)
  assert.deepEqual(Array.from(saves[0].availableTools), [p.id, 'read_file'])
  await toggle('read_file', false)
  assert.deepEqual(Array.from(saves[1].availableTools), [p.id])
  assert.equal(p.props.currentWorkflow.agentConfig.mcpTools.available[0], p.id)
})

test('canonical MCP reads reject stale success/failure and latest failure becomes unknown', async () => {
  const source = readFileSync(new URL('../../stores/capability.js', import.meta.url), 'utf8')
  const reads = []
  const mcpViews = ref({})
  const lastError = ref(null)
  const load = runInNewContext(
    sourceSection(source, '  let mcpReadRevision =', '  async function loadDoctor') + '\nloadMcpServers',
    { mcpViews, lastError, mcpViewIndex: rows => Object.fromEntries(rows.map(row => [row.id, row])),
      invokeCapability: () => new Promise((resolve, reject) => reads.push({ resolve, reject })) }
  )
  const old = load()
  const current = load()
  reads[1].resolve([{ id: 1, name: 'latest' }])
  await current
  reads[0].resolve([])
  await old
  assert.equal(mcpViews.value[1].name, 'latest')
  const staleFailure = load()
  const staleResult = assert.rejects(staleFailure, /stale/)
  const latest = load()
  reads[3].resolve([{ id: 1, name: 'newest' }])
  await latest
  reads[2].reject(new Error('stale'))
  await staleResult
  assert.equal(lastError.value, null)
  assert.equal(mcpViews.value[1].name, 'newest')
  const failure = load()
  const rejected = assert.rejects(failure, /offline/)
  reads[4].reject(new Error('offline'))
  await rejected
  assert.equal(Object.keys(mcpViews.value).length, 0)
})

console.log('toolClassification tests passed')
