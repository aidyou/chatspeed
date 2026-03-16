import { ref, computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'

/**
 * Composable for managing authorized paths
 * Handles adding/removing paths for workflows and pending paths for new workflows
 */
export function useWorkflowPaths({ currentWorkflowId, selectedAgent }) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  // Pending paths for new workflow (cached locally until workflow is created)
  const pendingPaths = ref([])

  // Allowed paths from current workflow
  const allowedPaths = computed(() => {
    const paths = workflowStore.currentWorkflow?.allowedPaths
    if (!paths) return []
    try {
      const parsed = typeof paths === 'string' ? JSON.parse(paths) : paths
      return parsed
    } catch (e) {
      return []
    }
  })

  // Current paths: use workflow paths if available, pending paths for new workflow, or agent paths as default
  const currentPaths = computed(() => {
    if (currentWorkflowId.value) {
      return allowedPaths.value
    }
    // No workflow - use pending paths if any, otherwise show agent's paths as reference
    if (pendingPaths.value.length > 0) {
      return pendingPaths.value
    }
    // Show agent's default paths as reference (read-only display)
    if (!selectedAgent.value) return []
    try {
      const paths = selectedAgent.value.allowedPaths
      if (!paths) return []
      return typeof paths === 'string' ? JSON.parse(paths) : paths
    } catch (e) {
      return []
    }
  })

  // Can edit paths if we have a workflow, or if we have a selected agent (for new workflow)
  const canEditPaths = computed(() => {
    return !!currentWorkflowId.value || !!selectedAgent.value
  })

  // Display first path name
  const displayAllowedPath = computed(() => {
    const paths = currentPaths.value
    if (!paths || paths.length === 0) return t('settings.agent.workingDirectory')
    const firstPath = paths[0]
    if (!firstPath) return t('settings.agent.workingDirectory')
    // Try to get last segment of path
    const parts = firstPath.split(/[/\\]/).filter((p) => p !== '')
    const result = parts[parts.length - 1] || firstPath
    return result
  })

  // Clear pending paths when agent changes
  const clearPendingPaths = () => {
    pendingPaths.value = []
  }

  const onAddPath = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t('settings.agent.selectDirectory')
      })
      if (selected) {
        if (currentWorkflowId.value) {
          // Editing existing workflow
          const newPaths = [...allowedPaths.value]
          if (!newPaths.includes(selected)) {
            newPaths.push(selected)
            await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
            // Immediately notify executor to update path_guard in memory
            await invokeWrapper('workflow_signal', {
              sessionId: currentWorkflowId.value,
              signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
            })
          }
        } else {
          // No workflow yet - cache in pendingPaths
          if (!pendingPaths.value.includes(selected)) {
            pendingPaths.value.push(selected)
          }
        }
      }
    } catch (error) {
      console.error('Failed to add path:', error)
    }
  }

  const onRemovePath = async (index) => {
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = [...allowedPaths.value]
      newPaths.splice(index, 1)
      await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
      // Immediately notify executor
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
      })
    } else {
      // No workflow yet - remove from pendingPaths
      pendingPaths.value.splice(index, 1)
    }
  }

  // Handle add path from FileTree component
  const onAddPathFromTree = async (selected) => {
    if (!selected) return
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = [...allowedPaths.value]
      if (!newPaths.includes(selected)) {
        newPaths.push(selected)
        await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
        // Immediately notify executor to update path_guard in memory
        await invokeWrapper('workflow_signal', {
          sessionId: currentWorkflowId.value,
          signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
        })
      }
    } else {
      // No workflow yet - cache in pendingPaths
      if (!pendingPaths.value.includes(selected)) {
        pendingPaths.value.push(selected)
      }
    }
  }

  // Handle remove path from FileTree component
  const onRemovePathFromTree = async (path) => {
    if (!path) return
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = allowedPaths.value.filter((p) => p !== path)
      await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
      // Immediately notify executor
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
      })
    } else {
      // No workflow yet - remove from pendingPaths
      const index = pendingPaths.value.indexOf(path)
      if (index > -1) {
        pendingPaths.value.splice(index, 1)
      }
    }
  }

  return {
    pendingPaths,
    allowedPaths,
    currentPaths,
    canEditPaths,
    displayAllowedPath,
    clearPendingPaths,
    onAddPath,
    onRemovePath,
    onAddPathFromTree,
    onRemovePathFromTree
  }
}
