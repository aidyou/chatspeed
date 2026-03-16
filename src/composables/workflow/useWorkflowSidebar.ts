import { ref, computed } from 'vue'
import { useWindowStore } from '@/stores/window'

/**
 * Composable for managing workflow sidebar state
 * Handles collapse/expand, width, and resize dragging
 */
export function useWorkflowSidebar() {
  const windowStore = useWindowStore()

  const sidebarCollapsed = ref(!windowStore.workflowSidebarShow)
  const sidebarWidthValue = ref(300) // Default sidebar width
  const isDragging = ref(false)
  const maxSidebarWidth = ref(window.innerWidth * 0.5)

  const sidebarWidth = computed(() =>
    sidebarCollapsed.value ? '0px' : `${sidebarWidthValue.value}px`
  )

  const sidebarStyle = computed(() => ({
    '--sidebar-width': sidebarCollapsed.value ? '0px' : `${sidebarWidthValue.value}px`
  }))

  const onToggleSidebar = () => {
    sidebarCollapsed.value = !sidebarCollapsed.value
    windowStore.setWorkflowSidebarShow(!sidebarCollapsed.value)
  }

  const updateMaxWidth = () => {
    maxSidebarWidth.value = window.innerWidth * 0.5
  }

  const onResizeStart = (e) => {
    if (sidebarCollapsed.value) return
    isDragging.value = true
    e.preventDefault()

    const startX = e.clientX
    const startWidth = sidebarWidthValue.value

    const onMouseMove = (moveEvent) => {
      const delta = moveEvent.clientX - startX
      const newWidth = Math.max(200, Math.min(startWidth + delta, maxSidebarWidth.value))
      sidebarWidthValue.value = newWidth
    }

    const onMouseUp = () => {
      isDragging.value = false
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }

    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
  }

  return {
    sidebarCollapsed,
    sidebarWidthValue,
    sidebarWidth,
    sidebarStyle,
    isDragging,
    maxSidebarWidth,
    onToggleSidebar,
    onResizeStart,
    updateMaxWidth
  }
}
