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
import { resolveWorkflowToolIcon } from '@/composables/workflow/toolIcons'
import { normalizeShellCommandForDisplay, formatDisplayPath } from '@/composables/workflow/toolDisplay'
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

const getLastSentence = text => {
  const normalized = sanitizePreviewText(text)
  if (!normalized) return ''
  const sentences = normalized.split(/(?<=[。！？.!?])\s*/).filter(Boolean)
  return sentences[sentences.length - 1] || normalized
}

const latestStreamingPreview = computed(() => {
  if (!props.isChatting) return ''
  return truncateText(getLastSentence(props.chatState?.reasoning || props.chatState?.content || ''), 72)
})

const chatHasStreamingOutput = computed(() => Boolean(latestStreamingPreview.value))

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

const buildToolText = (key, params, fallback) => {
  const translated = t(key, params)
  return typeof translated === 'string' && translated !== key ? translated : fallback
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
        text: buildToolText(
          'workflow.statusNotifier.awaitingEditApproval',
          { path },
          `等待修改审批: ${path}`
        ),
        tone: 'warning',
        icon: getToolIcon(toolName, 'edit'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: buildToolText(
          'workflow.statusNotifier.awaitingCreateApproval',
          { path },
          `等待创建审批: ${path}`
        ),
        tone: 'warning',
        icon: getToolIcon(toolName, 'write_file'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: buildToolText(
          'workflow.statusNotifier.awaitingBashApproval',
          { command: command || label },
          `等待命令执行审批: ${command || label}`
        ),
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
        text: buildToolText('workflow.statusNotifier.editingFile', { path }, `正在编辑文件: ${path}`),
        tone: 'info',
        icon: getToolIcon(toolName, 'edit'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: buildToolText('workflow.statusNotifier.creatingFile', { path }, `正在创建文件: ${path}`),
        tone: 'info',
        icon: getToolIcon(toolName, 'write_file'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: buildToolText(
          'workflow.statusNotifier.runningCommand',
          { command: command || label },
          `正在执行命令: ${command || label}`
        ),
        tone: 'info',
        icon: getToolIcon(toolName, 'bash'),
        spinning: false
      }
    }

    return {
      text: buildToolText('workflow.statusNotifier.runningTool', { tool: label }, `正在执行工具: ${label}`),
      tone: 'info',
      icon: getToolIcon(toolName, 'tool'),
      spinning: false
    }
  }

  if (tool.status === 'final_success') {
    if (toolName === 'edit_file' && path) {
      return {
        text: buildToolText(
          'workflow.statusNotifier.fileEditedDone',
          { path },
          `文件编辑完成: ${path}`
        ),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    if (toolName === 'write_file' && path) {
      return {
        text: buildToolText(
          'workflow.statusNotifier.fileCreatedDone',
          { path },
          `文件创建完成: ${path}`
        ),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    if (toolName === 'bash') {
      return {
        text: buildToolText(
          'workflow.statusNotifier.toolCompleted',
          { tool: command || label },
          `工具执行完成: ${command || label}`
        ),
        tone: 'info',
        icon: getToolIcon(toolName, 'check-circle'),
        spinning: false
      }
    }

    return {
      text: buildToolText('workflow.statusNotifier.toolCompleted', { tool: label }, `工具执行完成: ${label}`),
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
      text:
        buildToolText('workflow.statusNotifier.awaitingPlanApproval', {}, '等待用户确认计划') ||
        t('workflow.awaitingApproval') ||
        'Awaiting approval',
      tone: 'warning',
      icon: 'skill-plan',
      spinning: false
    }
  }

  if (isWaitingForUser.value) {
    return {
      text: buildToolText('workflow.statusNotifier.awaitingUserReply', {}, '等待用户回复'),
      tone: 'warning',
      icon: 'ask_user',
      spinning: false
    }
  }

  if (latestToolDisplay) {
    return latestToolDisplay
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

  if (chatHasStreamingOutput.value) {
    return {
      text: latestStreamingPreview.value,
      tone: 'info',
      icon: 'reasoning',
      spinning: true
    }
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
    if (String(newStatus || '').toLowerCase() === 'completed') {
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
