import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useWorkflowStore } from '@/stores/workflow'

/**
 * Composable for managing approval dialog logic
 * Handles approve, reject, and approve all actions
 */
export function useWorkflowApproval({ currentWorkflowId }) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  const approvalVisible = ref(false)
  const approvalAction = ref('')
  const approvalDetails = ref('')
  const approvalRequestId = ref('')
  const approvalLoading = ref(false)

  // Show approval dialog
  const showApproval = (payload) => {
    approvalRequestId.value = payload.id
    approvalAction.value = payload.action
    approvalDetails.value = payload.details
    approvalVisible.value = true
  }

  // Hide approval dialog
  const hideApproval = () => {
    approvalVisible.value = false
  }

  const onApproveAction = async () => {
    approvalLoading.value = true
    try {
      const signal = JSON.stringify({
        type: 'approval',
        approved: true,
        id: approvalRequestId.value
      })
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal
      })
      approvalVisible.value = false
    } catch (error) {
      console.error('Failed to approve action:', error)
      // If session is lost, force close dialog
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
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
      } else {
        showMessage(String(error), 'error')
      }
    } finally {
      approvalLoading.value = false
    }
  }

  const onApproveAllAction = async () => {
    approvalLoading.value = true
    try {
      const signal = JSON.stringify({
        type: 'approval',
        approved: true,
        approve_all: true,
        id: approvalRequestId.value
      })
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal
      })
      approvalVisible.value = false
    } catch (error) {
      console.error('Failed to approve all actions:', error)
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
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
      } else {
        showMessage(String(error), 'error')
      }
    } finally {
      approvalLoading.value = false
    }
  }

  const onRejectAction = async () => {
    approvalLoading.value = true
    try {
      const signal = JSON.stringify({
        type: 'approval',
        approved: false,
        id: approvalRequestId.value
      })
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal
      })
      approvalVisible.value = false
    } catch (error) {
      console.error('Failed to reject action:', error)
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
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
      } else {
        showMessage(String(error), 'error')
      }
    } finally {
      approvalLoading.value = false
    }
  }

  return {
    approvalVisible,
    approvalAction,
    approvalDetails,
    approvalRequestId,
    approvalLoading,
    showApproval,
    hideApproval,
    onApproveAction,
    onApproveAllAction,
    onRejectAction
  }
}
