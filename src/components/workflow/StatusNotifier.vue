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
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { resolveWorkflowToolIcon } from '@/composables/workflow/toolIcons'
import { normalizeShellCommandForDisplay, formatDisplayPath } from '@/composables/workflow/toolDisplay'
import { TERMINAL_STATUSES } from '@/composables/workflow/signalTypes'
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

const APPROVAL_REQUIRED_TOOLS = new Set(['edit_file', 'write_file', 'bash'])

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
const isTerminal = computed(() => TERMINAL_STATUSES.includes(workflowStatus.value))
const isWaitingForUser = computed(
  () => workflowWaitReason.value === 'user_input' || workflowStatus.value === 'awaiting_user'
)
const isWaitingForApproval = computed(
  () =>
    workflowWaitReason.value === 'approval' ||
    workflowStatus.value === 'awaiting_approval' ||
    workflowStatus.value === 'awaiting_auto_approval'
)
const retryInfo = computed(() => props.chatState?.retryInfo || null)
const hasActiveRetry = computed(() => Number(retryInfo.value?.nextRetryIn || 0) > 0)
const visible = computed(
  () =>
    !isTerminal.value &&
    (workflowStore.isRunning ||
      isWaitingForUser.value ||
      isWaitingForApproval.value ||
      hasActiveRetry.value ||
      workflowStore.notification.message)
)

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

const truncateText = (text, maxLength = 60) => {
  const normalized = sanitizePreviewText(text)
  if (!normalized || normalized.length <= maxLength) return normalized
  return `${normalized.slice(0, maxLength - 3)}...`
}

const getPreviewSegment = text => {
  const normalized = sanitizePreviewText(text)
  if (!normalized) return ''

  const completedSentences = normalized.match(/[^。！？.!?]+[。！？.!?]+(?=\s|$|[^\w])/g) || []
  const latestCompleted = completedSentences[completedSentences.length - 1]
  if (latestCompleted) return latestCompleted.trim()

  const segments = normalized.split(/[\n。！？.!?]+/).map(segment => segment.trim()).filter(Boolean)
  return segments[segments.length - 1] || normalized
}

const getMessageTimestamp = (message, fallback = 0) => {
  const candidates = [
    message?.createdAt,
    message?.created_at,
    message?.updatedAt,
    message?.updated_at,
    message?.metadata?.created_at,
    message?.metadata?.createdAt,
    message?.metadata?.updated_at,
    message?.metadata?.updatedAt
  ]

  for (const candidate of candidates) {
    const numeric = Number(candidate)
    if (Number.isFinite(numeric) && numeric > 0) return numeric
  }

  return fallback
}

const getToolTimestamp = tool =>
  Math.max(
    Number(tool?.updatedAt || 0),
    Number(tool?.createdAt || 0),
    Number(tool?.startedAt || 0)
  )

const streamActivity = ref({ contentAt: 0, reasoningAt: 0 })

watch(
  () => props.chatState?.content || '',
  (content, previousContent) => {
    if (content && content !== previousContent) {
      streamActivity.value = { ...streamActivity.value, contentAt: Date.now() }
    }
  },
  { immediate: true }
)

watch(
  () => props.chatState?.reasoning || '',
  (reasoning, previousReasoning) => {
    if (reasoning && reasoning !== previousReasoning) {
      streamActivity.value = { ...streamActivity.value, reasoningAt: Date.now() }
    }
  },
  { immediate: true }
)

const latestStreamingActivity = computed(() => {
  if (!props.isChatting) return null

  const content = props.chatState?.content || ''
  const reasoning = props.chatState?.reasoning || ''
  const candidates = [
    {
      text: truncateText(getPreviewSegment(content), 72),
      updatedAt: streamActivity.value.contentAt,
      kind: 'content'
    },
    {
      text: truncateText(getPreviewSegment(reasoning), 72),
      updatedAt: streamActivity.value.reasoningAt,
      kind: 'reasoning'
    }
  ].filter(candidate => candidate.text)

  return candidates.sort((left, right) => right.updatedAt - left.updatedAt)[0] || null
})

const latestTranscriptActivity = computed(() => {
  const candidates = []

  currentStepMessages.value.forEach((message, index) => {
    if (message?.role !== 'assistant') return

    const updatedAt = getMessageTimestamp(message, index + 1)
    const contentText = truncateText(getPreviewSegment(message?.message || ''), 72)
    if (contentText) {
      candidates.push({ text: contentText, updatedAt, kind: 'content' })
    }

    const reasoningText = truncateText(getPreviewSegment(message?.reasoning || ''), 72)
    if (reasoningText) {
      candidates.push({ text: reasoningText, updatedAt, kind: 'reasoning' })
    }
  })

  return candidates.sort((left, right) => right.updatedAt - left.updatedAt)[0] || null
})

const latestTextActivity = computed(() => {
  const candidates = [latestStreamingActivity.value, latestTranscriptActivity.value].filter(Boolean)
  return candidates.sort((left, right) => right.updatedAt - left.updatedAt)[0] || null
})

const pendingApprovalRequest = computed(() => workflowStore.pendingApprovalRequest || null)
const isWaitingForPlanApproval = computed(
  () =>
    !!workflowStore.canApprovePlan &&
    String(pendingApprovalRequest.value?.toolName || '').toLowerCase() === 'submit_plan'
)

const latestToolState = computed(() => {
  const tools = Array.isArray(workflowStore.toolList) ? workflowStore.toolList : []
  const stepToolCallIds = currentStepToolCallIds.value
  return [...tools]
    .filter(tool => {
      const toolCallId = String(tool?.toolCallId || '').trim()
      if (!toolCallId || !stepToolCallIds.has(toolCallId)) return false
      return ['pending', 'approved_running', 'final_success', 'final_error', 'rejected'].includes(
        String(tool?.status || '')
      )
    })
    .sort((left, right) => getToolTimestamp(right) - getToolTimestamp(left))[0]
})

const elapsedNow = ref(Date.now())
let elapsedTimer = null

const stopElapsedTimer = () => {
  if (elapsedTimer) {
    clearInterval(elapsedTimer)
    elapsedTimer = null
  }
}

watch(
  () => !isTerminal.value && (workflowStore.isRunning || latestToolState.value?.status === 'approved_running'),
  shouldTick => {
    stopElapsedTimer()
    elapsedNow.value = Date.now()
    if (shouldTick) {
      elapsedTimer = setInterval(() => {
        elapsedNow.value = Date.now()
      }, 1000)
    }
  },
  { immediate: true }
)

onBeforeUnmount(stopElapsedTimer)

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

const getToolName = tool => String(tool?.toolName || '').toLowerCase()

const getToolIcon = (toolName, fallback = 'tool') =>
  resolveWorkflowToolIcon(toolName, fallback)

const getToolPath = tool => {
  const args = tool?.arguments || {}
  const rawPath = args.file_path || args.path || ''
  return rawPath ? formatDisplayPath(String(rawPath)) : ''
}

const getWorkflowDisplayRoots = () => {
  const workflow = workflowStore.currentWorkflow
  const roots = [
    ...(Array.isArray(workflow?.allowedPaths) ? workflow.allowedPaths : []),
    ...(Array.isArray(workflow?.agentConfig?.allowedPaths) ? workflow.agentConfig.allowedPaths : [])
  ]
  return [...new Set(roots.filter(Boolean))]
}

const getToolCommand = tool => {
  const args = tool?.arguments || {}
  const command = args.command || ''
  return command
    ? `Run ${normalizeShellCommandForDisplay(String(command), getWorkflowDisplayRoots())}`
    : ''
}

const getToolLabel = tool => truncateText(tool?.title || tool?.summary || '', 72)

const buildToolText = (key, params) => t(key, params)

const withToolElapsed = (text, tool) => {
  const startedAt = Number(tool?.startedAt || tool?.updatedAt || tool?.createdAt || elapsedNow.value)
  const seconds = Math.max(0, Math.floor((elapsedNow.value - startedAt) / 1000))
  return buildToolText('workflow.statusNotifier.runningElapsed', { text, seconds })
}

const buildToolState = tool => {
  if (!tool) return null

  const toolName = getToolName(tool)
  const path = truncateText(getToolPath(tool), 72)
  const command = truncateText(getToolCommand(tool), 72)
  const label = getToolLabel(tool)

  if (tool.status === 'pending') {
    if (toolName === 'edit_file' && path) {
      return {
        text: buildToolText('workflow.statusNotifier.awaitingEditApproval', { path }),
        tone: 'warning',
        icon: getToolIcon(toolName, 'edit'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: buildToolText('workflow.statusNotifier.awaitingCreateApproval', { path }),
        tone: 'warning',
        icon: getToolIcon(toolName, 'write_file'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: buildToolText('workflow.statusNotifier.awaitingBashApproval', {
          command: command || label
        }),
        tone: 'warning',
        icon: getToolIcon(toolName, 'bash'),
        spinning: false
      }
    }

    if (!APPROVAL_REQUIRED_TOOLS.has(toolName)) return null
  }

  if (tool.status === 'approved_running') {
    if (toolName === 'edit_file' && path) {
      return {
        text: withToolElapsed(buildToolText('workflow.statusNotifier.editingFile', { path }), tool),
        tone: 'info',
        icon: getToolIcon(toolName, 'edit'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: withToolElapsed(buildToolText('workflow.statusNotifier.creatingFile', { path }), tool),
        tone: 'info',
        icon: getToolIcon(toolName, 'write_file'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: withToolElapsed(
          buildToolText('workflow.statusNotifier.runningCommand', {
            command: command || label
          }),
          tool
        ),
        tone: 'info',
        icon: getToolIcon(toolName, 'bash'),
        spinning: false
      }
    }

    return {
      text: withToolElapsed(
        buildToolText('workflow.statusNotifier.runningTool', { tool: label }),
        tool
      ),
      tone: 'info',
      icon: getToolIcon(toolName, 'tool'),
      spinning: false
    }
  }

  if (tool.status === 'final_success') {
    if (toolName === 'edit_file' && path) {
      return {
        text: buildToolText('workflow.statusNotifier.fileEditedDone', { path }),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: buildToolText('workflow.statusNotifier.fileCreatedDone', { path }),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: buildToolText('workflow.statusNotifier.toolCompleted', {
          tool: command || label
        }),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    return {
      text: buildToolText('workflow.statusNotifier.toolCompleted', { tool: label }),
      tone: 'info',
      icon: getToolIcon(toolName, 'check-circle'),
      spinning: false
    }
  }

  return null
}

const displayState = computed(() => {
  const notification = workflowStore.notification || {}
  const notificationMessage = sanitizePreviewText(notification.message || '')
  const notificationCategory = String(notification.category || 'info')
  const latestTool = latestToolState.value
  const latestToolDisplay = buildToolState(latestTool)
  const latestToolUpdatedAt = latestTool ? getToolTimestamp(latestTool) : 0
  const latestToolStatus = String(latestTool?.status || '')
  const latestText = latestTextActivity.value
  const latestTextUpdatedAt = Number(latestText?.updatedAt || 0)
  const latestToolSucceeded = latestToolStatus === 'final_success'
  const latestToolIsActive = ['pending', 'approved_running'].includes(latestToolStatus)
  const textIsLatestActivity = latestTextUpdatedAt >= latestToolUpdatedAt
  const shouldShowCompletedTool =
    latestToolSucceeded &&
    latestToolDisplay &&
    !textIsLatestActivity &&
    elapsedNow.value - latestToolUpdatedAt < 5000

  if (hasActiveRetry.value) {
    return {
      text: t('workflow.retrying', {
        attempt: retryInfo.value.attempt,
        total: retryInfo.value.total,
        seconds: retryInfo.value.nextRetryIn
      }),
      tone: 'warning',
      icon: 'loading',
      spinning: true
    }
  }

  if (notificationMessage && ['warning', 'error'].includes(notificationCategory)) {
    return {
      text: notificationMessage,
      tone: notificationCategory,
      icon: 'warning',
      spinning: false
    }
  }

  if (isWaitingForPlanApproval.value) {
    return {
      text: buildToolText('workflow.statusNotifier.awaitingPlanApproval'),
      tone: 'warning',
      icon: 'skill-plan',
      spinning: false
    }
  }

  if (isWaitingForUser.value) {
    return {
      text: buildToolText('workflow.statusNotifier.awaitingUserReply'),
      tone: 'warning',
      icon: 'ask_user',
      spinning: false
    }
  }

  if (latestToolDisplay && latestToolIsActive) {
    return latestToolDisplay
  }

  if (isWaitingForApproval.value) {
    return {
      text: t('workflow.awaitingApproval'),
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

  if (latestText?.text && textIsLatestActivity) {
    return {
      text: latestText.text,
      tone: 'info',
      icon: 'reasoning',
      spinning: true
    }
  }

  if (shouldShowCompletedTool) {
    return latestToolDisplay
  }

  if (workflowStore.isRunning) {
    return {
      text: buildToolText('workflow.statusNotifier.thinking', {}, t('workflow.thinking') || 'Thinking...'),
      tone: 'info',
      icon: 'reasoning',
      spinning: true
    }
  }

  if (notificationMessage) {
    return {
      text: notificationMessage,
      tone:
        notificationCategory === 'error'
          ? 'error'
          : notificationCategory === 'warning'
            ? 'warning'
            : 'info',
      icon:
        notificationCategory === 'warning' || notificationCategory === 'error' ? 'warning' : 'info',
      spinning: false
    }
  }

  return {
    text: buildToolText('workflow.statusNotifier.thinking', {}, t('workflow.thinking') || 'Thinking...'),
    tone: 'info',
    icon: 'reasoning',
    spinning: true
  }
})

const displayMessage = computed(() => displayState.value.text)

watch(() => workflowStore.notification.timestamp, () => {
  if (workflowStore.notification.message && !workflowStore.isRunning) {
    setTimeout(() => {
      workflowStore.setNotification('', 'info')
    }, 10000)
  }
})

watch(() => workflowStore.currentWorkflow?.status, (newStatus, oldStatus) => {
  if (oldStatus && newStatus !== oldStatus) {
    if (TERMINAL_STATUSES.includes(String(newStatus || '').toLowerCase())) {
      workflowStore.setNotification('', 'info')
      return
    }

    const specialCategories = ['warning', 'error']
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
