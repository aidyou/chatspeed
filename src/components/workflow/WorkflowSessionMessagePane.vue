<template>
  <section class="workflow-session-message-pane" :data-agent-role="agentRole">
    <header v-if="agentRole === 'child'" class="workflow-session-message-pane__header">
      <div class="workflow-session-message-pane__title">
        <cs name="task" />
        <span>{{ $t('workflow.messageList') }}</span>
        <span v-if="childStatus" class="workflow-session-message-pane__status">{{ childStatus }}</span>
      </div>
      <el-tooltip :content="$t('common.close')" :hide-after="0" :enterable="false">
        <button type="button" class="workflow-session-message-pane__close" @click="$emit('close')">
          <cs name="fullscreen" />
        </button>
      </el-tooltip>
    </header>

    <WorkflowMessageList
      ref="messageListRef"
      :key="`${agentRole}:${sessionId || 'empty'}`"
      :messages="messageProjection.enhancedMessages.value"
      :is-loading="isLoadingMessages"
      :hidden-earlier-message-count="messageProjection.hiddenEarlierMessageCount.value"
      :is-running="isRunning"
      :queued-messages="agentRole === 'primary' ? workflowStore.messageQueue : []"
      :is-chatting="isChatting"
      :chat-state="resolvedChatState"
      :is-compressing="isCompressing"
      :compression-message="compressionMessage"
      :last-assistant-message="messageProjection.lastAssistantMessage.value"
      :approval-loading="approval.approvalLoading.value"
      :active-approval-id="approval.activeApprovalId.value"
      :is-batch-approval-submitting="false"
      :ask-user-submitting="agentRole === 'primary' ? askUserSubmitting : false"
      :is-message-expanded="messageProjection.isMessageExpanded"
      :is-reasoning-expanded="messageProjection.isReasoningExpanded"
      :remove-system-reminder="messageProjection.removeSystemReminder"
      :get-diff-markdown="messageProjection.getDiffMarkdown"
      :parse-choice-content="messageProjection.parseChoiceContent"
      :get-parsed-message="messageProjection.getParsedMessage"
      :should-show-tool-raw-content="messageProjection.shouldShowToolRawContent"
      :pending-count="resolvedPendingApprovalIds.length"
      :pending-approvals="resolvedPendingApprovals"
      :pending-approval-ids="resolvedPendingApprovalIds"
      :current-workflow-id="sessionId"
      :wait-reason="resolvedWaitReason"
      :is-approval-submitting="approval.isApprovalSubmitting.value"
      :get-tool-stream="getToolStream"
      @message-window-anchor-change="messageProjection.setMessageWindowAnchor"
      @toggle-expand="messageProjection.toggleMessageExpand"
      @toggle-reasoning="messageProjection.toggleReasoningExpand"
      @reveal-earlier-messages="loadEarlierMessagePage"
      @submit-ask-user="$emit('submit-ask-user', $event)"
      @approve-tool="approveTool"
      @approve-all-tool="approveAllTool"
      @approve-all-pending="approveAllPending"
      @remove-queued-message="$emit('remove-queued-message', $event)"
      @open-sub-agent="$emit('open-sub-agent', $event)"
      @reject-tool="rejectTool" />
  </section>
</template>

<script setup>
import { computed, ref } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
import { useWorkflowMessages } from '@/composables/workflow/useWorkflowMessages'
import { useWorkflowApproval } from '@/composables/workflow/useWorkflowApproval'
import { useWorkflowSessionMessages } from '@/composables/workflow/useWorkflowSessionMessages'
import WorkflowMessageList from './WorkflowMessageList.vue'

const props = defineProps({
  sessionId: { type: String, default: '' },
  agentRole: { type: String, default: 'primary', validator: value => ['primary', 'child'].includes(value) },
  primaryChatState: { type: Object, default: null },
  primaryIsChatting: { type: Boolean, default: false },
  primaryIsCompressing: { type: Boolean, default: false },
  primaryCompressionMessage: { type: String, default: '' },
  primaryWaitReason: { type: String, default: '' },
  askUserSubmitting: { type: Boolean, default: false }
})

const emit = defineEmits(['close', 'submit-ask-user', 'remove-queued-message', 'open-sub-agent'])
const workflowStore = useWorkflowStore()
const messageListRef = ref(null)
const sessionIdRef = computed(() => props.sessionId)
const agentRoleRef = computed(() => props.agentRole)
const childSession = useWorkflowSessionMessages({ sessionId: sessionIdRef, agentRole: agentRoleRef })
const source = props.agentRole === 'child' ? childSession.source : workflowStore
const messageProjection = useWorkflowMessages(source)

const resolvedChatState = computed(() => props.agentRole === 'child' ? childSession.chatState.value : props.primaryChatState)
const isChatting = computed(() => props.agentRole === 'child' ? childSession.isRunning.value : props.primaryIsChatting)
const isCompressing = computed(() => props.agentRole === 'child' ? childSession.isCompressing.value : props.primaryIsCompressing)
const compressionMessage = computed(() => props.agentRole === 'child' ? childSession.compressionMessage.value : props.primaryCompressionMessage)
const isLoadingMessages = computed(() => props.agentRole === 'child' ? childSession.isLoadingMessages.value : workflowStore.isLoadingMessages)
const isRunning = computed(() => props.agentRole === 'child' ? childSession.isRunning.value : workflowStore.isRunning)
const resolvedWaitReason = computed(() => props.agentRole === 'child' ? childSession.waitReason.value : props.primaryWaitReason)
const resolvedPendingApprovals = computed(() => props.agentRole === 'child' ? childSession.pendingApprovals.value : workflowStore.currentInlinePendingApprovals)
const resolvedPendingApprovalIds = computed(() => props.agentRole === 'child' ? childSession.pendingApprovalIds.value : workflowStore.currentInlinePendingApprovalIds)
const childStatus = computed(() => childSession.workflow.value?.status || '')
const getToolStream = toolCallId => props.agentRole === 'child' ? childSession.source.getToolStream(toolCallId) : workflowStore.getToolStream(toolCallId)

const approval = useWorkflowApproval({
  currentWorkflowId: sessionIdRef,
  getPendingApprovalEntry: () => null,
  clearPendingApprovalEntry: () => {},
  upsertPendingApprovalEntry: () => {},
  trackToolState: props.agentRole === 'primary'
})

const approveTool = toolCallId => approval.onApproveAction(toolCallId, props.sessionId)
const approveAllTool = toolCallId => approval.onApproveAllAction(toolCallId, props.sessionId)
const rejectTool = (toolCallId, message) => approval.onRejectAction(toolCallId, message, props.sessionId)
const approveAllPending = async payload => {
  for (const toolCallId of payload?.orderedToolCallIds || []) {
    await approval.onApproveAction(toolCallId, props.sessionId)
  }
}

const loadEarlierMessagePage = done => {
  messageProjection.revealEarlierMessages()
  done?.()
}

defineExpose({
  scrollToBottom: force => messageListRef.value?.scrollToBottom(force)
})
</script>

<style scoped lang="scss">
.workflow-session-message-pane {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
}

.workflow-session-message-pane :deep(.messages-container) {
  flex: 1;
}

.workflow-session-message-pane__header {
  display: flex;
  min-height: 40px;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--cs-border-color);
  padding: 0 var(--cs-space);
  background: var(--cs-bg-color);
}

.workflow-session-message-pane__title {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: var(--cs-space-sm);
  color: var(--cs-text-color-primary);
  font-weight: 600;
}

.workflow-session-message-pane__status {
  overflow: hidden;
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-sm);
  font-weight: 400;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workflow-session-message-pane__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: var(--cs-border-radius-sm);
  padding: var(--cs-space-xs);
  color: var(--cs-text-color-primary);
  background: transparent;
  cursor: pointer;
}

.workflow-session-message-pane__close:hover {
  background: var(--cs-hover-bg-color);
}
</style>
