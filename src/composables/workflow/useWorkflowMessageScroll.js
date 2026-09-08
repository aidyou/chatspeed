import { nextTick, ref } from 'vue'

const AUTO_SCROLL_THRESHOLD = 64
const MAX_SETTLE_FRAMES = 3

const requestFrame = callback => {
  if (typeof requestAnimationFrame === 'function') return requestAnimationFrame(callback)
  return setTimeout(callback, 0)
}

const cancelFrame = frameId => {
  if (frameId === null || frameId === undefined) return
  if (typeof cancelAnimationFrame === 'function') {
    cancelAnimationFrame(frameId)
  } else {
    clearTimeout(frameId)
  }
}

/**
 * Own all scroll decisions for a workflow message list.
 *
 * Content watchers and ResizeObserver only notify this controller. They never
 * decide whether the user should follow the bottom or keep reading history.
 */
export function useWorkflowMessageScroll({ containerRef, onWindowAnchorChange = () => {} } = {}) {
  const mode = ref('following')

  let disposed = false
  let pendingSnapshot = null
  let reconcileScheduled = false
  let reconcileFrameId = null
  let settleFrameId = null
  let settleFrameBudget = 0
  let pendingExplicitBottom = false
  let internalScrollTarget = null
  let readingAnchor = null
  let lastWindowAnchorId = ''
  let revision = 0

  const getContainer = () => containerRef?.value || null

  const isNearBottom = container => {
    if (!container) return true
    return container.scrollHeight - container.scrollTop - container.clientHeight <= AUTO_SCROLL_THRESHOLD
  }

  const emitWindowAnchorChange = windowAnchorId => {
    const normalizedId = String(windowAnchorId || '').trim()
    if (normalizedId === lastWindowAnchorId) return
    lastWindowAnchorId = normalizedId
    onWindowAnchorChange(normalizedId)
  }

  const captureAnchor = container => {
    if (!container) return null

    const containerRect = container.getBoundingClientRect()
    const anchorElement = Array.from(container.querySelectorAll('.message[data-message-id]'))
      .filter(element => element.getBoundingClientRect().bottom > containerRect.top + 1)
      .find(element => Boolean(element.getAttribute('data-window-anchor-id')))
    if (!anchorElement) return null

    const rect = anchorElement.getBoundingClientRect()
    return {
      id: anchorElement.getAttribute('data-message-id') || '',
      windowAnchorId: anchorElement.getAttribute('data-window-anchor-id') || '',
      offsetTop: rect.top - containerRect.top
    }
  }

  const updateReadingAnchor = (container, { emit = true } = {}) => {
    const anchor = captureAnchor(container)
    if (!anchor?.id) return readingAnchor

    readingAnchor = anchor
    if (emit) emitWindowAnchorChange(anchor.windowAnchorId)
    return anchor
  }

  const clearReadingAnchor = ({ emit = true } = {}) => {
    readingAnchor = null
    if (emit) emitWindowAnchorChange('')
  }

  const captureSnapshot = () => {
    if (pendingSnapshot) return pendingSnapshot

    const container = getContainer()
    if (!container) return null

    const snapshot = {
      mode: mode.value,
      scrollTop: container.scrollTop,
      scrollHeight: container.scrollHeight,
      clientHeight: container.clientHeight,
      anchor:
        mode.value === 'reading'
          ? readingAnchor || updateReadingAnchor(container, { emit: false })
          : null
    }
    pendingSnapshot = snapshot
    return snapshot
  }

  const cancelScheduledFrames = () => {
    revision += 1
    if (reconcileFrameId !== null) {
      cancelFrame(reconcileFrameId)
      reconcileFrameId = null
    }
    if (settleFrameId !== null) {
      cancelFrame(settleFrameId)
      settleFrameId = null
    }
    reconcileScheduled = false
    settleFrameBudget = 0
    internalScrollTarget = null
  }

  const clampScrollTop = (container, value) => {
    const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight)
    return Math.min(Math.max(0, value), maxScrollTop)
  }

  const writeScrollTop = (container, value) => {
    const target = clampScrollTop(container, value)
    if (Math.abs(container.scrollTop - target) <= 0.5) {
      internalScrollTarget = null
      return false
    }

    internalScrollTarget = target
    container.scrollTop = target
    return true
  }

  const restoreReadingPosition = (container, snapshot) => {
    const anchor = readingAnchor || snapshot?.anchor
    if (anchor?.id) {
      const anchorElement = Array.from(container.querySelectorAll('.message[data-message-id]')).find(
        element => element.getAttribute('data-message-id') === anchor.id
      )
      if (anchorElement) {
        const containerRect = container.getBoundingClientRect()
        const currentOffsetTop = anchorElement.getBoundingClientRect().top - containerRect.top
        const nextScrollTop = container.scrollTop + (currentOffsetTop - anchor.offsetTop)
        writeScrollTop(container, nextScrollTop)
        readingAnchor = { ...anchor, offsetTop: anchor.offsetTop }
        return true
      }
    }

    if (snapshot) {
      writeScrollTop(
        container,
        snapshot.scrollTop + (container.scrollHeight - snapshot.scrollHeight)
      )
      return true
    }

    return false
  }

  const scrollToBottomNow = container => {
    writeScrollTop(container, container.scrollHeight - container.clientHeight)
  }

  const scheduleSettle = (requestedBudget = MAX_SETTLE_FRAMES - 1) => {
    settleFrameBudget = Math.max(settleFrameBudget, requestedBudget)
    if (settleFrameBudget <= 0 || settleFrameId !== null) return

    const expectedRevision = revision
    settleFrameId = requestFrame(() => {
      settleFrameId = null
      if (disposed || expectedRevision !== revision || mode.value !== 'following') return

      const container = getContainer()
      if (!container) return
      scrollToBottomNow(container)
      settleFrameBudget -= 1
      if (settleFrameBudget > 0) scheduleSettle(settleFrameBudget)
    })
  }

  const reconcile = () => {
    reconcileScheduled = false
    reconcileFrameId = null
    if (disposed) return

    const container = getContainer()
    const snapshot = pendingSnapshot
    pendingSnapshot = null
    if (!container) return

    if (pendingExplicitBottom || mode.value === 'following') {
      pendingExplicitBottom = false
      scrollToBottomNow(container)
      scheduleSettle(MAX_SETTLE_FRAMES - 1)
      return
    }

    if (mode.value === 'reading') {
      restoreReadingPosition(container, snapshot)
    }
  }

  const requestReconcile = () => {
    if (disposed || reconcileScheduled) return
    reconcileScheduled = true
    const expectedRevision = revision

    nextTick(() => {
      if (disposed || expectedRevision !== revision) {
        reconcileScheduled = false
        return
      }
      reconcileFrameId = requestFrame(() => {
        if (disposed || expectedRevision !== revision) {
          reconcileScheduled = false
          reconcileFrameId = null
          return
        }
        reconcile()
      })
    })
  }

  const beforeContentChange = () => {
    captureSnapshot()
  }

  const requestContentChange = () => {
    if (disposed) return
    if (!pendingSnapshot) captureSnapshot()
    requestReconcile()
  }

  const onContentResize = () => {
    if (disposed) return
    if (mode.value === 'reading' && !readingAnchor) {
      updateReadingAnchor(getContainer())
    }
    requestContentChange()
  }

  const onWheel = event => {
    if (event.deltaY >= 0 || disposed) return

    cancelScheduledFrames()
    pendingSnapshot = null
    pendingExplicitBottom = false
    if (mode.value !== 'reading') {
      mode.value = 'reading'
      updateReadingAnchor(getContainer())
    }
  }

  const onScroll = () => {
    const container = getContainer()
    if (!container || disposed) return

    const currentScrollTop = container.scrollTop
    const isInternalScroll =
      internalScrollTarget !== null && Math.abs(currentScrollTop - internalScrollTarget) <= 1
    if (isInternalScroll) {
      internalScrollTarget = null
      return
    }

    cancelScheduledFrames()
    pendingSnapshot = null
    pendingExplicitBottom = false
    internalScrollTarget = null
    if (isNearBottom(container)) {
      mode.value = 'following'
      clearReadingAnchor()
      return
    }

    mode.value = 'reading'
    updateReadingAnchor(container)
  }

  const scrollToBottom = (force = false) => {
    if (disposed) return

    cancelScheduledFrames()
    pendingSnapshot = null
    if (force) {
      mode.value = 'following'
      pendingExplicitBottom = true
      clearReadingAnchor()
    }

    if (mode.value === 'following') {
      requestReconcile()
    }
  }

  const reset = () => {
    cancelScheduledFrames()
    pendingSnapshot = null
    pendingExplicitBottom = false
    internalScrollTarget = null
    mode.value = 'following'
    clearReadingAnchor()
  }

  const dispose = () => {
    disposed = true
    cancelScheduledFrames()
    pendingSnapshot = null
    pendingExplicitBottom = false
    internalScrollTarget = null
  }

  return {
    mode,
    beforeContentChange,
    requestContentChange,
    onContentResize,
    onScroll,
    onWheel,
    scrollToBottom,
    reset,
    dispose
  }
}
