import assert from 'node:assert/strict'
import test from 'node:test'

import { hydrateWorkflowSession } from './workflowSessionHydration.js'

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
