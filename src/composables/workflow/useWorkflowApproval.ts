import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useWorkflowStore } from '@/stores/workflow'
import { SIGNAL_TYPES } from '@/composables/workflow/signalTypes'

/**
 * Composable for managing workflow approval actions.
 * Approval UI is rendered inline in the message list.
 */
export function useWorkflowApproval({
  currentWorkflowId,
  getPendingApprovalEntry,
  clearPendingApprovalEntry,
  upsertPendingApprovalEntry
}) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  const approvalLoading = ref(false)
  const activeApprovalId = ref('')

  const isApprovalSubmitting = computed(
    () => (sessionId, toolCallId) => workflowStore.isApprovalSubmitted(sessionId, toolCallId)
  )

  const submitApproval = async ({
    toolCallId,
    approved,
    approveAll = false,
    rejectionMessage = '',
    sessionId = currentWorkflowId.value
  }) => {
    if (!toolCallId || !sessionId) {
      return
    }
    approvalLoading.value = true
    activeApprovalId.value = toolCallId
    const pendingEntry = getPendingApprovalEntry?.(sessionId, toolCallId) || null
    workflowStore.markApprovalSubmitted(sessionId, toolCallId)

    try {
      const signal = JSON.stringify({
        type: SIGNAL_TYPES.APPROVAL,
        approved,
        approve_all: approveAll,
        id: toolCallId,
        rejection_message: rejectionMessage?.trim() || undefined
      })

      await invokeWrapper('workflow_signal', {
        sessionId,
        signal
      })

      if (!approved) {
        workflowStore.markToolRejected(toolCallId, rejectionMessage)
      }
    } catch (error) {
      workflowStore.clearApprovalSubmission(sessionId, toolCallId)
      if (pendingEntry) {
        upsertPendingApprovalEntry?.(sessionId, {
          id: pendingEntry.id,
          kind: pendingEntry.kind,
          action: pendingEntry.action
        })
      }
      console.error('Failed to resolve approval action:', error)
      if (
        String(error).includes('No sender') ||
        String(error).includes('No active session') ||
        String(error).includes('Session interrupted')
      ) {
        showMessage(
          t('workflow.sessionLost') ||
            'Session disconnected. Please refresh the page to restore the workflow.',
          'warning'
        )
        workflowStore.setRunning(false)
      } else {
        showMessage(String(error), 'error')
      }
    } finally {
      approvalLoading.value = false
      activeApprovalId.value = ''
    }
  }

  const onApproveAction = (toolCallId, sessionId) =>
    submitApproval({
      toolCallId,
      sessionId,
      approved: true,
      approveAll: false
    })

  const onApproveAllAction = (toolCallId, sessionId) =>
    submitApproval({
      toolCallId,
      sessionId,
      approved: true,
      approveAll: true
    })

  const onRejectAction = (toolCallId, rejectionMessage, sessionId) =>
    submitApproval({
      toolCallId,
      sessionId,
      approved: false,
      approveAll: false,
      rejectionMessage
    })

  return {
    approvalLoading,
    activeApprovalId,
    isApprovalSubmitting,
    onApproveAction,
    onApproveAllAction,
    onRejectAction
  }
}
