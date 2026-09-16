<template>
  <div
    class="chat-hub-splitter"
    :style="{ right: `${right}px` }"
    role="separator"
    aria-orientation="vertical"
    @pointerdown="onPointerDown" />
</template>

<script setup>
import { onBeforeUnmount } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'

/**
 * Splitter between the workflow UI and the docked ChatHub page.
 *
 * The page is a native webview, so it cannot be resized by CSS: dragging this handle
 * reports the width the page should use and the carrier applies it. The clamp mirrors
 * the backend one (same limits, same window width) so this side never asks for a width
 * the page cannot have.
 *
 * The handle sits just inside the workflow UI, because the page webview is a native
 * sibling: pointer events that land on the right of the boundary go to the page instead
 * of this webview.
 */
const props = defineProps({
  /** Space the workflow UI reserves on its right, in logical pixels. */
  right: {
    type: Number,
    default: 0
  },
  /** Width the page uses now, in logical pixels. */
  width: {
    type: Number,
    required: true
  },
  /** Narrowest page width the carrier accepts. */
  minWidth: {
    type: Number,
    required: true
  },
  /** Width the workflow UI keeps next to the page. */
  minHostWidth: {
    type: Number,
    required: true
  }
})

const emit = defineEmits(['resize'])

let frame = 0
let dragging = false
let startX = 0
let startWidth = 0
let maxWidth = 0

/** Logical width of the window client area, which is what the carrier clamps against. */
const windowInnerWidth = async () => {
  const appWindow = getCurrentWindow()
  const [size, scaleFactor] = await Promise.all([
    appWindow.innerSize(),
    appWindow.scaleFactor()
  ])

  return size.width / (scaleFactor || 1)
}

const apply = event => {
  cancelAnimationFrame(frame)
  const clientX = event.clientX
  frame = requestAnimationFrame(() => {
    const width = Math.min(maxWidth, Math.max(props.minWidth, startWidth - (clientX - startX)))
    emit('resize', width)
  })
}

const stop = () => {
  dragging = false
  cancelAnimationFrame(frame)
  window.removeEventListener('pointermove', apply)
  window.removeEventListener('pointerup', stop)
  window.removeEventListener('pointercancel', stop)
}

const onPointerDown = async event => {
  if (event.button !== 0 || dragging) {
    return
  }

  dragging = true
  startX = event.clientX
  startWidth = props.width
  maxWidth = Math.max(props.minWidth, (await windowInnerWidth()) - props.minHostWidth)

  event.preventDefault()
  window.addEventListener('pointermove', apply)
  window.addEventListener('pointerup', stop)
  window.addEventListener('pointercancel', stop)
}

onBeforeUnmount(stop)
</script>

<style lang="scss">
.chat-hub-splitter {
  position: fixed;
  top: var(--cs-titlebar-height);
  bottom: 0;
  z-index: 5;
  width: 6px;
  cursor: col-resize;
  background-color: transparent;
  transition: background-color 0.2s ease;

  &:hover {
    background-color: var(--cs-border-color);
  }
}
</style>
