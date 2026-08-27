export async function hydrateWorkflowSession({
  registerListener,
  fetchSnapshot,
  applySnapshot,
  applyEvent,
  isCurrent,
  onListenerRegistered
}) {
  const queuedEvents = []
  let snapshotApplied = false
  const queueOrApplyEvent = payload => {
    if (!snapshotApplied) {
      queuedEvents.push(payload)
      return
    }
    applyEvent(payload)
  }

  const stop = await registerListener(queueOrApplyEvent)
  onListenerRegistered?.(stop)
  try {
    const snapshot = await fetchSnapshot()
    if (!isCurrent()) return { stop, applied: false }
    applySnapshot(snapshot)
    snapshotApplied = true
    queuedEvents.splice(0).forEach(applyEvent)
    return { stop, applied: true }
  } catch (error) {
    if (isCurrent()) {
      snapshotApplied = true
      queuedEvents.splice(0).forEach(applyEvent)
    }
    throw error
  }
}
