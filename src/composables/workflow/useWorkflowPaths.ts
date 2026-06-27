import { ref, computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'
import { useWorkflowAutomationStore } from '@/stores/workflowAutomation'

/**
 * Composable for managing authorized paths
 * Handles adding/removing paths for workflows and pending paths for new workflows
 */
export function useWorkflowPaths({
  currentWorkflowId,
  selectedAgent,
  activeTab,
  selectedAutomation,
  historyItemCount,
  automationItemCount
}) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()
  const workflowAutomationStore = useWorkflowAutomationStore()

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

  const isAutomationContext = computed(() => {
    return activeTab?.value === 'automation' && !!selectedAutomation?.value?.id
  })

  const automationAllowedPaths = computed(() => {
    if (!isAutomationContext.value) return []
    return Array.isArray(selectedAutomation.value?.allowedPaths) ? selectedAutomation.value.allowedPaths : []
  })

  const hasHistoryItems = computed(() => Number(historyItemCount?.value || 0) > 0)
  const hasAutomationItems = computed(() => Number(automationItemCount?.value || 0) > 0)

  // Current paths: use workflow paths if available, pending paths for new workflow, or agent paths as default
  const currentPaths = computed(() => {
    if (activeTab?.value === 'automation' && !selectedAutomation.value?.id) {
      return []
    }
    if (isAutomationContext.value) {
      return automationAllowedPaths.value
    }
    if (currentWorkflowId.value) {
      return allowedPaths.value
    }
    if (activeTab?.value === 'history' && !hasHistoryItems.value) {
      return []
    }
    if (activeTab?.value === 'automation' && !hasAutomationItems.value) {
      return []
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
    if (isAutomationContext.value) {
      return true
    }
    if (activeTab?.value === 'automation' && !selectedAutomation.value?.id) {
      return false
    }
    if (activeTab?.value === 'history' && !currentWorkflowId.value && !hasHistoryItems.value) {
      return false
    }
    return !!currentWorkflowId.value || !!selectedAgent.value
  })

  const updateSelectedAutomationPaths = async updater => {
    const automationId = selectedAutomation.value?.id
    if (!automationId) return
    const nextPaths = updater([...automationAllowedPaths.value])
    await workflowAutomationStore.updateAutomationAllowedPaths(automationId, nextPaths)
  }

  // Display first path name
  const displayAllowedPath = computed(() => {
    const paths = currentPaths.value
    if (!paths || paths.length === 0) return t('settings.agent.workingDirectory')
    return paths
      .map(p => {
        const parts = p.split(/[/\\]/).filter(p => p !== '')
        return parts[parts.length - 1] ?? ''
      })
      .filter(p => p.trim() !== '')
      .join(', ')
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
        if (isAutomationContext.value) {
          await updateSelectedAutomationPaths(paths => {
            if (!paths.includes(selected)) {
              paths.push(selected)
            }
            return paths
          })
          return
        }
        if (currentWorkflowId.value) {
          // Editing existing workflow
          const newPaths = [...allowedPaths.value]
          if (!newPaths.includes(selected)) {
            newPaths.push(selected)
            await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
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

  const onRemovePath = async index => {
    if (isAutomationContext.value) {
      await updateSelectedAutomationPaths(paths => {
        paths.splice(index, 1)
        return paths
      })
      return
    }
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = [...allowedPaths.value]
      newPaths.splice(index, 1)
      await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
    } else {
      // No workflow yet - remove from pendingPaths
      pendingPaths.value.splice(index, 1)
    }
  }

  // Handle add path from FileTree component
  const onAddPathFromTree = async selected => {
    if (!selected) return
    if (isAutomationContext.value) {
      await updateSelectedAutomationPaths(paths => {
        if (!paths.includes(selected)) {
          paths.push(selected)
        }
        return paths
      })
      return
    }
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = [...allowedPaths.value]
      if (!newPaths.includes(selected)) {
        newPaths.push(selected)
        await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
      }
    } else {
      // No workflow yet - cache in pendingPaths
      if (!pendingPaths.value.includes(selected)) {
        pendingPaths.value.push(selected)
      }
    }
  }

  // Handle remove path from FileTree component
  const onRemovePathFromTree = async path => {
    if (!path) return
    if (isAutomationContext.value) {
      await updateSelectedAutomationPaths(paths => paths.filter(item => item !== path))
      return
    }
    if (currentWorkflowId.value) {
      // Editing existing workflow
      const newPaths = allowedPaths.value.filter(p => p !== path)
      await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
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
