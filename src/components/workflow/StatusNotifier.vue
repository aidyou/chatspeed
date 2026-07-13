<template>
  <div v-if="visible" class="status-notifier" :class="[displayState.tone, { active: visible }]">
    <div class="notifier-content">
      <cs
        :name="displayState.icon"
        size="14px"
        class="status-icon"
        :class="{ rotating: displayState.spinning }" />

      <span class="status-message">{{ displayMessage }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'

const props = defineProps({
  chatState: {
    type: Object,
    default: () => ({
      content: '',
      reasoning: '',
      reasoningStatus: 'idle'
    })
  },
  isChatting: {
    type: Boolean,
    default: false
  }
})

const { t } = useI18n()
const workflowStore = useWorkflowStore()

const workflowStatus = computed(() =>
  String(workflowStore.currentWorkflow?.status || '').toLowerCase()
)
const workflowWaitReason = computed(() =>
  String(
    workflowStore.waitReason ||
      workflowStore.currentWorkflow?.waitReason ||
      workflowStore.currentWorkflow?.wait_reason ||
      ''
  ).toLowerCase()
)
const isCompleted = computed(() => workflowStatus.value === 'completed')
const isWaitingForUser = computed(
  () => workflowWaitReason.value === 'user_input' || workflowStatus.value === 'awaiting_user'
)
const isWaitingForApproval = computed(
  () =>
    workflowWaitReason.value === 'approval' ||
    workflowStatus.value === 'awaiting_approval' ||
    workflowStatus.value === 'awaiting_auto_approval'
)
const visible = computed(
  () =>
    !isCompleted.value &&
    (workflowStore.isRunning ||
      isWaitingForUser.value ||
      isWaitingForApproval.value ||
      workflowStore.notification.message)
)
const isRunning = computed(() => workflowStore.isRunning)

const currentWorkflowMessages = computed(() => {
  const workflowId = workflowStore.currentWorkflowId
  return (workflowStore.messages || []).filter(message => {
    const messageWorkflowId = message?.sessionId || message?.session_id
    return !messageWorkflowId || messageWorkflowId === workflowId
  })
})

const currentStepMessages = computed(() => {
  const messages = currentWorkflowMessages.value
  let lastUserIndex = -1

  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === 'user') {
      lastUserIndex = index
      break
    }
  }

  return lastUserIndex >= 0 ? messages.slice(lastUserIndex + 1) : messages
})

const currentStepToolCallIds = computed(() => {
  const ids = new Set()

  for (const message of currentStepMessages.value) {
    const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
    if (toolCallId) ids.add(toolCallId)

    const toolCalls = Array.isArray(message?.metadata?.tool_calls) ? message.metadata.tool_calls : []
    for (const call of toolCalls) {
      const callId = String(call?.id || '').trim()
      if (callId) ids.add(callId)
    }
  }

  return ids
})

const sanitizePreviewText = text =>
  String(text || '')
    .replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '')
    .replace(/^\s*<(?:think|thinking)(?:\s+class="[^"]*")?>\s*/i, '')
    .replace(/\s*<\/(?:think|thinking)>\s*$/i, '')
    .replace(/\s+/g, ' ')
    .trim()

const getLastSentence = text => {
  const normalized = sanitizePreviewText(text)
  if (!normalized) return ''
  const sentences = normalized.split(/(?<=[。！？.!?])\s*/).filter(Boolean)
  return sentences[sentences.length - 1] || normalized
}

const latestToolState = computed(() => {
  const tools = Array.isArray(workflowStore.toolList) ? workflowStore.toolList : []
  const stepToolCallIds = currentStepToolCallIds.value
  return [...tools]
    .filter(tool => {
      const toolCallId = String(tool?.toolCallId || '').trim()
      if (!toolCallId || !stepToolCallIds.has(toolCallId)) return false
      return ['pending', 'approved_running'].includes(String(tool?.status || ''))
    })
    .sort((left, right) => Number(right?.updatedAt || 0) - Number(left?.updatedAt || 0))[0]
})

const latestTerminalError = computed(() => {
  for (let index = currentStepMessages.value.length - 1; index >= 0; index -= 1) {
    const message = currentStepMessages.value[index]
    if (message?.role === 'user') continue

    const isError = !!(message?.isError || message?.is_error || message?.metadata?.is_error)
    if (!isError) return ''

    const toolError =
      sanitizePreviewText(message?.toolDisplay?.summary || '') ||
      sanitizePreviewText(message?.toolDisplay?.title || '')
    if (toolError) return toolError

    return sanitizePreviewText(message?.message || message?.reasoning || '')
  }

  return ''
})

const latestAssistantPreview = computed(() => {
  if (props.isChatting) {
    return getLastSentence(props.chatState?.reasoning || props.chatState?.content || '')
  }

  for (let index = currentStepMessages.value.length - 1; index >= 0; index -= 1) {
    const message = currentStepMessages.value[index]
    if (message?.role !== 'assistant') continue
    const preview = getLastSentence(message?.reasoning || message?.message || '')
    if (preview) return preview
  }
  return ''
})

const displayState = computed(() => {
  const notification = workflowStore.notification || {}
  const notificationMessage = sanitizePreviewText(notification.message || '')
  const notificationCategory = String(notification.category || 'info')
  const latestTool = latestToolState.value

  if (notificationMessage && ['warning', 'error'].includes(notificationCategory)) {
    return {
      text: notificationMessage,
      tone: notificationCategory,
      icon: 'warning',
      spinning: false
    }
  }

  if (isWaitingForUser.value) {
    return {
      text: t('workflow.awaitingUser') || 'Awaiting user input',
      tone: 'warning',
      icon: 'warning',
      spinning: false
    }
  }

  if (latestTool) {
    const title = sanitizePreviewText(latestTool.title || '')
    const summary = sanitizePreviewText(latestTool.summary || '')

    if (latestTool.status === 'approved_running') {
      return {
        text: `${t('workflow.executing') || 'Executing...'} ${title || summary}`,
        tone: 'info',
        icon: 'loading',
        spinning: true
      }
    }

    if (latestTool.status === 'pending') {
      return {
        text: `${t('workflow.awaitingApproval') || 'Awaiting approval'}: ${title || summary}`,
        tone: 'warning',
        icon: 'warning',
        spinning: false
      }
    }
  }

  if (isWaitingForApproval.value) {
    return {
      text: t('workflow.awaitingApproval') || 'Awaiting approval',
      tone: 'warning',
      icon: 'warning',
      spinning: false
    }
  }

  if (latestTerminalError.value) {
    return {
      text: `${t('common.error') || 'Error'}: ${latestTerminalError.value}`,
      tone: 'error',
      icon: 'warning',
      spinning: false
    }
  }

  if (latestAssistantPreview.value) {
    return {
      text: latestAssistantPreview.value,
      tone: 'info',
      icon: 'reasoning',
      spinning: false
    }
  }

  if (notificationMessage) {
    return {
      text: notificationMessage,
      tone: notificationCategory === 'error' ? 'error' : notificationCategory === 'warning' ? 'warning' : 'info',
      icon: notificationCategory === 'warning' || notificationCategory === 'error' ? 'warning' : 'info',
      spinning: false
    }
  }

  return {
    text: t('workflow.thinking') || 'Thinking...',
    tone: 'info',
    icon: 'reasoning',
    spinning: true
  }
})

const displayMessage = computed(() => displayState.value.text)

// Reset notification after 10 seconds if it's not a persistent one (like compression or retrying)
watch(() => workflowStore.notification.timestamp, () => {
  if (workflowStore.notification.message && !workflowStore.isRunning) {
    setTimeout(() => {
      workflowStore.setNotification('', 'info')
    }, 10000)
  }
})

// Clear notification when workflow state changes (except for special categories)
watch(() => workflowStore.currentWorkflow?.status, (newStatus, oldStatus) => {
  if (oldStatus && newStatus !== oldStatus) {
    if (String(newStatus || '').toLowerCase() === 'completed') {
      workflowStore.setNotification('', 'info')
      return
    }

    // State changed - clear notification unless it's a special category
    // We use category as identifier instead of message content for i18n safety
    const specialCategories = ['warning', 'error'];
    const shouldKeep = specialCategories.includes(workflowStore.notification.category)

    if (!shouldKeep) {
      workflowStore.setNotification('', 'info')
    }
  }
})
</script>

<style lang="scss" scoped>
.status-notifier {
  padding: 0 0 var(--cs-space-xs);
  font-size: 12px;
  color: var(--cs-text-color-secondary);
  min-height: 24px;
  display: flex;
  align-items: center;
  overflow: hidden;
  transition: all 0.3s ease;
  opacity: 0;
  transform: translateY(-100%);

  &.active {
    opacity: 1;
    transform: translateY(0);
  }

  &.warning {
    color: var(--el-color-warning);
  }

  &.error {
    color: var(--el-color-danger);
  }

  .notifier-content {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }

  .status-icon {
    flex-shrink: 0;
  }

  .status-message {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }
}

.rotating {
  animation: rotate 2s linear infinite;
}

@keyframes rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

</style>
