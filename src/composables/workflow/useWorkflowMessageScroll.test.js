import assert from 'node:assert/strict'
import test from 'node:test'
import { ref } from 'vue'
import { useWorkflowMessageScroll } from './useWorkflowMessageScroll.js'

const waitForReconcile = () => new Promise(resolve => setTimeout(resolve, 15))

// Scroll work is requested from a microtask (`nextTick`) and only then handed to the
// animation frame fallback, so draining microtasks lets a request reach the frame queue.
const drainMicrotasks = async () => {
  for (let turn = 0; turn < 4; turn += 1) await Promise.resolve()
}

const createContainer = ({ scrollTop = 0, scrollHeight = 1000, clientHeight = 400 } = {}) => {
  const container = {
    scrollTop,
    scrollHeight,
    clientHeight,
    children: [],
    getBoundingClientRect: () => ({ top: 0 }),
    querySelectorAll: () => []
  }
  return container
}

test('keeps following mode at the bottom after content changes', async () => {
  const container = createContainer({ scrollTop: 600 })
  const containerRef = ref(container)
  const controller = useWorkflowMessageScroll({ containerRef })

  controller.beforeContentChange()
  container.scrollHeight = 1200
  controller.requestContentChange()
  await waitForReconcile()

  assert.equal(container.scrollTop, 800)
  assert.equal(controller.mode.value, 'following')
  controller.dispose()
})

test('keeps following a later tail change after an explicit send boundary', async () => {
  const container = createContainer({ scrollTop: 200, scrollHeight: 1000 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  controller.onWheel({ deltaY: -100 })
  controller.onScroll()
  assert.equal(controller.mode.value, 'reading')

  controller.scrollToBottom(true)
  await waitForReconcile()
  assert.equal(container.scrollTop, 600)
  assert.equal(controller.mode.value, 'following')

  controller.beforeContentChange()
  container.scrollHeight = 1250
  controller.requestContentChange()
  await waitForReconcile()

  assert.equal(container.scrollTop, 850)
  assert.equal(controller.mode.value, 'following')
  controller.dispose()
})

test('non-forced tail updates do not interrupt history reading', async () => {
  const container = createContainer({ scrollTop: 200, scrollHeight: 1000 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  controller.onWheel({ deltaY: -100 })
  controller.onScroll()
  controller.scrollToBottom()
  controller.beforeContentChange()
  container.scrollHeight = 1100
  controller.requestContentChange()
  await waitForReconcile()

  assert.equal(container.scrollTop, 300)
  assert.equal(controller.mode.value, 'reading')
  controller.dispose()
})

test('restores a physical message anchor while reading history', async () => {
  const anchor = {
    id: 'message-2',
    windowAnchorId: 'window-message-2',
    getAttribute: name =>
      ({
        'data-message-id': 'message-2',
        'data-window-anchor-id': 'window-message-2'
      })[name],
    getBoundingClientRect: () => ({ top: 80, bottom: 140 })
  }
  const container = createContainer({ scrollTop: 320, scrollHeight: 1200 })
  container.querySelectorAll = () => [anchor]
  container.getBoundingClientRect = () => ({ top: 0 })
  const containerRef = ref(container)
  const controller = useWorkflowMessageScroll({ containerRef })

  controller.onWheel({ deltaY: -100 })
  controller.onScroll()
  assert.equal(controller.mode.value, 'reading')

  controller.beforeContentChange()
  container.scrollTop = 260
  anchor.getBoundingClientRect = () => ({ top: 20, bottom: 80 })
  container.scrollHeight = 1140
  controller.requestContentChange()
  await waitForReconcile()

  assert.equal(container.scrollTop, 200)
  assert.equal(controller.mode.value, 'reading')
  controller.dispose()
})

test('scrollbar dragging cancels pending automatic scrolling', async () => {
  const container = createContainer({ scrollTop: 600 })
  const containerRef = ref(container)
  const controller = useWorkflowMessageScroll({ containerRef })

  controller.beforeContentChange()
  container.scrollHeight = 1400
  controller.requestContentChange()
  container.scrollTop = 200
  controller.onScroll()
  await waitForReconcile()

  assert.equal(container.scrollTop, 200)
  assert.equal(controller.mode.value, 'reading')
  controller.dispose()
})
test('prefers a visible message with a window anchor id', () => {
  const events = []
  const anchoredMessage = {
    getAttribute: name =>
      ({
        'data-message-id': 'message-2',
        'data-window-anchor-id': 'window-message-2'
      })[name] || null,
    getBoundingClientRect: () => ({ top: 80, bottom: 140 })
  }
  const unanchoredMessage = {
    getAttribute: name =>
      ({ 'data-message-id': 'message-1', 'data-window-anchor-id': null })[name] || null,
    getBoundingClientRect: () => ({ top: 20, bottom: 80 })
  }
  const nextAnchoredMessage = {
    getAttribute: name =>
      ({
        'data-message-id': 'message-3',
        'data-window-anchor-id': 'window-message-3'
      })[name] || null,
    getBoundingClientRect: () => ({ top: 80, bottom: 140 })
  }
  const messages = [anchoredMessage]
  const container = createContainer({ scrollTop: 100 })
  container.querySelectorAll = () => messages
  const controller = useWorkflowMessageScroll({
    containerRef: ref(container),
    onWindowAnchorChange: anchorId => events.push(anchorId)
  })

  controller.onWheel({ deltaY: -100 })
  messages.splice(0, messages.length, unanchoredMessage)
  controller.onScroll()
  assert.deepEqual(events, ['window-message-2'])

  messages.splice(0, messages.length, unanchoredMessage, nextAnchoredMessage)
  controller.onScroll()

  assert.deepEqual(events, ['window-message-2', 'window-message-3'])
  controller.dispose()
})

test('cancelling scheduled scrolling clears its internal target', async () => {
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame
  const originalCancelAnimationFrame = globalThis.cancelAnimationFrame
  const pendingFrames = new Map()
  let nextFrameId = 0

  globalThis.requestAnimationFrame = callback => {
    const frameId = ++nextFrameId
    pendingFrames.set(frameId, callback)
    return frameId
  }
  globalThis.cancelAnimationFrame = frameId => pendingFrames.delete(frameId)

  try {
    const container = createContainer({ scrollTop: 100 })
    const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

    controller.scrollToBottom(true)
    await new Promise(resolve => setTimeout(resolve, 0))
    assert.equal(pendingFrames.size, 1)

    const reconcileFrame = pendingFrames.values().next().value
    pendingFrames.delete(1)
    reconcileFrame()
    assert.equal(container.scrollTop, 600)
    assert.equal(pendingFrames.size, 1)

    controller.onWheel({ deltaY: -100 })
    container.scrollHeight = 1300
    controller.onScroll()

    assert.equal(controller.mode.value, 'reading')
    controller.dispose()
  } finally {
    if (originalRequestAnimationFrame) {
      globalThis.requestAnimationFrame = originalRequestAnimationFrame
    } else {
      delete globalThis.requestAnimationFrame
    }
    if (originalCancelAnimationFrame) {
      globalThis.cancelAnimationFrame = originalCancelAnimationFrame
    } else {
      delete globalThis.cancelAnimationFrame
    }
  }
})

test('an in-flight programmatic scroll event does not cancel an explicit bottom request', async () => {
  const container = createContainer({ scrollTop: 600 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  // Streaming output pins the list to the bottom; that write reports its own scroll
  // event in a later frame.
  controller.beforeContentChange()
  container.scrollHeight = 1200
  controller.requestContentChange()
  await waitForReconcile()
  assert.equal(container.scrollTop, 800)

  // Sending a message while the workflow runs: the send forces the bottom and the
  // queued-message block is appended in the same update.
  controller.scrollToBottom(true)
  controller.beforeContentChange()
  container.scrollHeight = 1270
  controller.requestContentChange()
  await drainMicrotasks()

  // The earlier write's scroll event arrives before that frame runs, carrying the value
  // we wrote ourselves. It must not be treated as a user scroll.
  controller.onScroll()
  await waitForReconcile()

  assert.equal(controller.mode.value, 'following')
  assert.equal(container.scrollTop, 870)
  controller.dispose()
})

test('a wheel gesture keeps scroll control while our own write event is in flight', async () => {
  const container = createContainer({ scrollTop: 600 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  controller.beforeContentChange()
  container.scrollHeight = 1200
  controller.requestContentChange()
  await waitForReconcile()
  assert.equal(container.scrollTop, 800)

  controller.onWheel({ deltaY: -100 })
  container.scrollTop = 700
  controller.onScroll()
  await waitForReconcile()

  assert.equal(controller.mode.value, 'reading')
  assert.equal(container.scrollTop, 700)
  controller.dispose()
})

test('a late content resize keeps following mode pinned to the newest content', async () => {
  const container = createContainer({ scrollTop: 600 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  controller.beforeContentChange()
  container.scrollHeight = 1200
  controller.requestContentChange()
  await waitForReconcile()
  assert.equal(container.scrollTop, 800)

  // A queued-message block or streamed markdown can grow after the first reconcile.
  container.scrollHeight = 1320
  controller.onContentResize()
  assert.equal(container.scrollTop, 920)
  assert.equal(controller.mode.value, 'following')

  controller.dispose()
})

test('a container resize re-pins the newest content only while following', async () => {
  const container = createContainer({ scrollTop: 600 })
  const controller = useWorkflowMessageScroll({ containerRef: ref(container) })

  // Following: a shorter pane leaves the list short of its newest content.
  container.clientHeight = 300
  controller.onContainerResize()
  await waitForReconcile()
  assert.equal(container.scrollTop, 700)
  assert.equal(controller.mode.value, 'following')

  // Reading history: the same resize must not move the reader.
  controller.onWheel({ deltaY: -100 })
  container.scrollTop = 200
  controller.onScroll()
  assert.equal(controller.mode.value, 'reading')

  container.clientHeight = 250
  controller.onContainerResize()
  await waitForReconcile()
  assert.equal(container.scrollTop, 200)
  controller.dispose()
})
