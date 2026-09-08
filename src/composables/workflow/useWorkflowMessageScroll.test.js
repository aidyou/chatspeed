import assert from 'node:assert/strict'
import test from 'node:test'
import { ref } from 'vue'
import { useWorkflowMessageScroll } from './useWorkflowMessageScroll.js'

const waitForReconcile = () => new Promise(resolve => setTimeout(resolve, 15))

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
test('does not let a delayed internal scroll event switch reading mode', async () => {
  const container = createContainer({ scrollTop: 100, scrollHeight: 1000 })
  const containerRef = ref(container)
  const controller = useWorkflowMessageScroll({ containerRef })

  controller.scrollToBottom(true)
  await waitForReconcile()
  assert.equal(container.scrollTop, 600)

  controller.onScroll()
  assert.equal(controller.mode.value, 'following')
  controller.dispose()
})
