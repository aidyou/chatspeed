import { nextTick, ref, watch } from 'vue'

export function useWorkflowMessageLoading({ isLoading, sessionId, onContentMounted = () => {} }) {
  const showLoadingSkeleton = ref(true)
  const renderMessages = ref(false)
  let revision = 0
  let frameId = null
  let disposed = false

  const cancelPendingFrame = () => {
    if (frameId === null) return
    if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frameId)
    else clearTimeout(frameId)
    frameId = null
  }

  const scheduleFrame = callback => {
    const run = () => {
      frameId = null
      callback()
    }
    frameId = typeof requestAnimationFrame === 'function'
      ? requestAnimationFrame(run)
      : setTimeout(run, 0)
  }

  const stop = watch(
    [isLoading, sessionId],
    ([loading]) => {
      const requestRevision = ++revision
      const isCurrent = () => !disposed && revision === requestRevision
      cancelPendingFrame()
      showLoadingSkeleton.value = true
      renderMessages.value = false
      if (loading) return

      nextTick(() => {
        if (!isCurrent()) return
        scheduleFrame(() => {
          if (!isCurrent()) return
          scheduleFrame(() => {
            if (!isCurrent()) return
            renderMessages.value = true
            nextTick(() => {
              if (!isCurrent()) return
              onContentMounted()
              scheduleFrame(() => {
                if (isCurrent()) showLoadingSkeleton.value = false
              })
            })
          })
        })
      })
    },
    { immediate: true }
  )

  const dispose = () => {
    disposed = true
    revision += 1
    stop()
    cancelPendingFrame()
  }

  return { showLoadingSkeleton, renderMessages, dispose }
}
