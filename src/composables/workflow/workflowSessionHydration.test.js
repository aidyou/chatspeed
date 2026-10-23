import assert from 'node:assert/strict'
import test from 'node:test'
import { readFile } from 'node:fs/promises'
import { runInNewContext } from 'node:vm'

import { hydrateWorkflowSession } from './workflowSessionHydration.js'
import {
  buildStructuredPendingToolMetadata,
  deriveInlinePendingApprovals,
  upsertExecutionContextPendingTool
} from '../../stores/workflowApprovalRecovery.js'
import { isWorkflowMessagePendingApproval } from './messageProjectionRules.js'

test('queues a child approval received after listener registration until the snapshot is applied', async () => {
  let listener = null
  let resolveSnapshot
  const snapshotPromise = new Promise(resolve => {
    resolveSnapshot = resolve
  })
  const pendingToolIds = []
  const approvalSignals = []

  const hydration = hydrateWorkflowSession({
    registerListener: async handleEvent => {
      listener = handleEvent
      return () => {}
    },
    fetchSnapshot: () => snapshotPromise,
    applySnapshot: snapshot => {
      pendingToolIds.splice(0, pendingToolIds.length, ...snapshot.pendingToolIds)
    },
    applyEvent: payload => {
      if (payload.type !== 'confirm') return
      pendingToolIds.push(payload.id)
      approvalSignals.push({ sessionId: payload.sessionId, toolCallId: payload.id })
    },
    isCurrent: () => true
  })

  await new Promise(resolve => setImmediate(resolve))
  listener({ type: 'confirm', id: 'child-tool-call', sessionId: 'child-session' })
  resolveSnapshot({ pendingToolIds: [] })
  await hydration

  assert.deepEqual(pendingToolIds, ['child-tool-call'])
  assert.deepEqual(approvalSignals, [{ sessionId: 'child-session', toolCallId: 'child-tool-call' }])
})

test('primary reconnect subscribes before the snapshot and restores approvals received during hydration', async () => {
  const source = await readFile(new URL('./useWorkflowCore.ts', import.meta.url), 'utf8')
  const setupSource = source.slice(
    source.indexOf('const setupWorkflowEvents = async'),
    source.indexOf('const reconcilePendingSubAgentApprovals = async')
  )
  const calls = []
  let listener
  let resolveSnapshot
  const snapshotReady = new Promise(resolve => { resolveSnapshot = resolve })
  const workflow = { status: 'executing', executionContext: null }
  const messages = []
  const sessionId = 'reopened-primary'
  const command = 'command -v cargo rustc; cargo --version 2>&1 | head -2'
  const store = {
    currentWorkflowId: sessionId,
    currentWorkflow: workflow,
    setHasLiveSession: () => {},
    clearApprovalSubmission: () => {},
    upsertPendingTool: (_, pendingTool) => {
      workflow.executionContext = upsertExecutionContextPendingTool(workflow.executionContext, pendingTool)
    }
  }
  const setup = runInNewContext(`${setupSource}; setupWorkflowEvents`, {
    workflowEventSetupRevision: 0,
    currentSessionId: { value: sessionId },
    unlistenWorkflowEvents: { value: null },
    workflowStore: store,
    hydrateWorkflowSession,
    listen: async (_, callback) => {
      calls.push('listen')
      listener = callback
      return () => {}
    },
    invokeWrapper: async command => { calls.push(command) },
    safeExecute: callback => callback(),
    TERMINAL_STATUSES: ['completed', 'failed', 'cancelled'],
    normalizeConfirmPendingTool: payload => ({
      toolCallId: payload.id,
      toolName: payload.tool_name,
      arguments: payload.arguments,
      details: payload.details,
      displayType: payload.display_type
    }),
    upsertPendingApprovalEntry: () => {},
    upsertPendingApprovalMessage: (_, payload) => {
      messages.push({
        role: 'tool',
        metadata: buildStructuredPendingToolMetadata({
          toolCallId: payload.id,
          toolName: payload.tool_name,
          arguments: payload.arguments,
          details: payload.details,
          displayType: payload.display_type
        })
      })
    },
    playApprovalNotificationSound: () => {},
    syncBackgroundStateListeners: async () => {}
  })

  const hydration = setup(sessionId, async () => {
    calls.push('snapshot')
    await snapshotReady
    workflow.status = 'awaiting_approval'
    workflow.executionContext = { wait_reason: 'approval', pending_tools: [] }
  })
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(calls, ['listen', 'workflow_subscribe', 'snapshot'])
  listener({ payload: {
    type: 'confirm', id: 'cargo-approval', tool_name: 'bash',
    arguments: { command }, details: { command }, display_type: 'text'
  } })
  assert.equal(messages.length, 0, 'events must wait for snapshot hydration')
  resolveSnapshot()
  assert.equal(await hydration, true)

  const approvals = deriveInlinePendingApprovals({
    currentWorkflowId: sessionId,
    status: workflow.status,
    waitReason: 'approval',
    executionContext: workflow.executionContext,
    messages
  })
  assert.equal(approvals.length, 1)
  assert.equal(approvals[0].arguments.command, command)
  assert.equal(isWorkflowMessagePendingApproval(messages[0], approvals.map(entry => entry.id)), true)
  assert.match(source, /setupWorkflowEvents\(id, \(\) => workflowStore\.selectWorkflow\(id\)\)/)
})
