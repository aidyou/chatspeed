import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { ref, nextTick, watch } from 'vue'

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
  assert.match(mcpProjection, /\.filter\(id => currentlyAvailableMcpTools\.has\(id\)\)/)
  assert.match(mcpProjection, /configuredMcpTools = new Set\(\[\.\.\.available, \.\.\.autoApprove, \.\.\.autoExpand\]\)/)

  const agentSource = readFileSync(new URL('../../components/setting/Agent.vue', import.meta.url), 'utf8')
  assert.match(agentSource, /!isWorkflowMcpTool\(t\.id, t\.category\)/)
  assert.match(agentSource, /const mcpToolOptions = computed\(\(\) => \{[\s\S]*?tool\.category === 'MCP'/)
})

test('MCP panel refreshes late discovery, avoids overlapping reads and stops polling on close', async () => {
  const visible = ref(false)
  let refreshes = 0
  let tick
  let cleared = false
  let release
  const gate = new Promise(resolve => { release = resolve })
  const stop = runInNewContext(
    sourceSection(inputSource, 'watch(mcpConfigPopoverVisible,', "const approvalToolsTab = ref('available')"),
    {
      watch: (...args) => watch(...args), mcpConfigPopoverVisible: visible,
      agentStore: { fetchAvailableTools: async () => { refreshes++; await gate } },
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
    release()
    await new Promise(resolve => setImmediate(resolve))
    await tick()
    assert.equal(refreshes, 2)
    visible.value = false
    await nextTick()
    assert.equal(cleared, true)
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
})

console.log('toolClassification tests passed')
