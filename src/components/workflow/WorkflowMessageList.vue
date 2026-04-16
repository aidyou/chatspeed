<template>
  <div class="messages" ref="messagesRef">
    <div
      v-for="(message, index) in messages"
      :key="message.displayId"
      class="message"
      :class="[message.role, message.stepType?.toLowerCase(), { 'is-error': message.isError }]">
      <div class="avatar" v-if="message.role === 'user'">
        <cs name="talk" class="user-icon" />
      </div>
      <div class="content-container">
        <div class="content" v-if="message.role === 'user'">
          <div v-if="getAskUserResponseItems(message).length > 0" class="ask-user-response-card">
            <div class="ask-user-response-title">{{ $t('workflow.askUser.responseTitle') }}</div>
            <div
              v-for="(item, itemIndex) in getAskUserResponseItems(message)"
              :key="`${item.title}-${itemIndex}`"
              class="ask-user-response-item">
              <div class="ask-user-response-question">{{ item.title }}</div>
              <div class="ask-user-response-answer">
                <span class="answer-label">{{ $t('workflow.askUser.answerLabel') }}</span>
                <span>{{ formatAskUserAnswer(item) }}</span>
              </div>
              <pre
                v-if="item.source === 'custom' && item.choice"
                class="ask-user-response-custom"
                >{{ item.choice }}</pre
              >
            </div>
          </div>
          <pre v-else class="simple-text">{{ getVisibleUserContent(message) }}</pre>
        </div>
        <div v-else class="ai-content chat">
          <!-- CLI Style Tool Call (Results) -->
          <div
            v-if="message.role === 'tool'"
            class="cli-tool-call"
            :class="[
              message.toolDisplay.toolType || 'tool-system',
              message.toolDisplay.isError ? 'status-error' : 'status-success'
            ]">
            <!-- finish_task special display -->
            <template
              v-if="
                message.toolDisplay?.action === $t('workflow.finishTask') ||
                message.toolDisplay?.action?.includes('Finish')
              ">
              <div class="tool-line finish-task-display">
                <cs
                  :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'"
                  size="14px"
                  class="tool-type-icon finish-icon" />
                <span class="finish-text">{{ $t('workflow.finishTask') }}</span>
              </div>
            </template>

            <!-- Normal tool call display -->
            <template v-else>
              <div
                class="tool-line title-wrap expandable"
                :class="{ 'tool-rejected': message.isRejected }"
                @click="$emit('toggle-expand', message.displayId)">
                <cs :name="message.toolDisplay.icon || 'tool'" size="15px" class="tool-type-icon" />
                <span class="tool-name">{{ message.toolDisplay.action }}</span>
                <span class="tool-target">{{ message.toolDisplay.target }}</span>
                <cs v-if="message.isApproved" name="check" size="14px" class="approved-icon" />
              </div>
              <!-- Hide summary when expanded -->
              <div
                class="tool-line summary expandable"
                v-if="!isMessageExpanded(message)"
                @click="$emit('toggle-expand', message.displayId)">
                <span class="corner-icon">⎿</span>
                <span class="summary-text">{{ message.toolDisplay.summary }}</span>
                <span class="expand-hint">(click to expand)</span>
              </div>
              <div v-if="isMessageExpanded(message)" class="tool-detail">
                <!-- Tool Stream Output (for bash commands) -->
                <div
                  v-if="
                    message.metadata?.tool_call_id &&
                    workflowStore.getToolStream(message.metadata.tool_call_id).length > 0
                  "
                  class="tool-stream-output">
                  <div
                    v-for="(line, idx) in workflowStore.getToolStream(
                      message.metadata.tool_call_id
                    )"
                    :key="idx"
                    class="stream-line">
                    {{ line }}
                  </div>
                </div>
                <!-- Final Result -->
                <MarkdownSimple
                  v-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'diff'
                  "
                  :content="getDiffMarkdown(removeSystemReminder(message.message))" />
                <div
                  v-else-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'choice'
                  "
                  class="choice-container">
                  <div
                    v-for="group in getChoiceGroups(message)"
                    :key="group.title"
                    class="choice-group">
                    <div class="choice-question">
                      {{ group.title }}
                    </div>
                    <el-radio-group
                      :model-value="getAskUserSelection(message, group.title)"
                      class="choice-options vertical numbered"
                      @update:model-value="
                        value => setAskUserSelection(message, group.title, value)
                      ">
                      <el-radio
                        v-for="(opt, optIndex) in group.options"
                        :key="`${group.title}-${opt}`"
                        :value="opt"
                        :disabled="!canAnswerAskUser(message, index) || askUserSubmitting">
                        <span class="choice-option-label">{{ optIndex + 1 }}. {{ opt }}</span>
                      </el-radio>
                      <div class="choice-custom-row">
                        <el-radio
                          :value="CUSTOM_ASK_USER_VALUE"
                          :disabled="!canAnswerAskUser(message, index) || askUserSubmitting">
                          <span class="choice-option-label">{{ group.options.length + 1 }}.</span>
                        </el-radio>
                        <el-input
                          :model-value="getAskUserCustomInput(message, group.title)"
                          class="choice-custom-input"
                          type="textarea"
                          :autosize="{ minRows: 1, maxRows: 6 }"
                          :placeholder="$t('workflow.askUser.customPlaceholder')"
                          :disabled="!canAnswerAskUser(message, index) || askUserSubmitting"
                          @focus="setAskUserSelection(message, group.title, CUSTOM_ASK_USER_VALUE)"
                          @update:model-value="
                            value => setAskUserCustomInput(message, group.title, value)
                          " />
                      </div>
                    </el-radio-group>
                  </div>
                  <div v-if="canAnswerAskUser(message, index)" class="choice-submit-row">
                    <el-button
                      size="small"
                      type="primary"
                      :loading="askUserSubmitting"
                      @click="submitAskUserResponse(message)">
                      {{ $t('workflow.askUser.submit') }}
                    </el-button>
                  </div>
                </div>
                <pre
                  v-else-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message)
                  "
                  class="raw-content"
                  >{{ removeSystemReminder(message.message) }}</pre
                >
                <ApprovalDialog
                  v-if="message.metadata?.approval_status === 'pending'"
                  inline
                  :action="message.metadata?.tool_name || message.toolDisplay.action"
                  :details="removeSystemReminder(message.message)"
                  :display-type="message.metadata?.display_type || message.toolDisplay.displayType"
                  :rejection-message="getApprovalDraft(message.metadata?.tool_call_id)"
                  :loading="approvalLoading && activeApprovalId === message.metadata?.tool_call_id"
                  @update:rejection-message="
                    value => setApprovalDraft(message.metadata?.tool_call_id, value)
                  "
                  @approve="$emit('approve-tool', message.metadata?.tool_call_id)"
                  @approve-all="$emit('approve-all-tool', message.metadata?.tool_call_id)"
                  @reject="
                    $emit(
                      'reject-tool',
                      message.metadata?.tool_call_id,
                      getApprovalDraft(message.metadata?.tool_call_id)
                    )
                  " />
              </div>
            </template>
          </div>

          <!-- Regular Assistant Content -->
          <div v-else>
            <!-- Thought/Content FIRST (Separate reasoning field has priority) -->
            <div
              v-if="message.reasoning || message.stepType === 'Think'"
              class="reasoning-container">
              <div class="reasoning-header" @click="$emit('toggle-reasoning', message.displayId)">
                <cs
                  name="reasoning"
                  size="14px"
                  class="reasoning-icon"
                  :class="{
                    rotating:
                      isRunning &&
                      !getParsedMessage(message).content &&
                      (message.metadata?.tool_calls?.length || 0) === 0 &&
                      !isReasoningExpanded(message.displayId) &&
                      message === lastAssistantMessage
                  }" />
                <span
                  class="reasoning-text"
                  :class="{ expanded: isReasoningExpanded(message.displayId) }">
                  <template v-if="isReasoningExpanded(message.displayId)">
                    {{ $t('workflow.thinkingExpanded') || 'Thinking Process' }}
                  </template>
                  <template
                    v-else-if="
                      isRunning &&
                      !getParsedMessage(message).content &&
                      (message.metadata?.tool_calls?.length || 0) === 0 &&
                      message === lastAssistantMessage
                    ">
                    {{ getReasoningPreview(message.reasoning || message.message) }}
                  </template>
                  <template v-else>
                    {{ $t('workflow.thoughtCompleted') || 'Thought Complete' }}
                  </template>
                </span>
                <span class="reasoning-toggle">
                  {{ isReasoningExpanded(message.displayId) ? '▲' : '▼' }}
                </span>
              </div>
              <div v-if="isReasoningExpanded(message.displayId)" class="reasoning-content">
                {{ message.reasoning || message.message }}
              </div>
            </div>
            <MarkdownSimple
              v-if="getParsedMessage(message).content"
              :content="getParsedMessage(message).content" />

            <!-- Tool Call Indicators SECOND (Only pending ones) -->
            <div v-if="message.pendingToolCalls?.length > 0" class="cli-tool-calls-container">
              <div
                v-for="call in message.pendingToolCalls"
                :key="call.id"
                class="cli-tool-call pending"
                :class="[
                  call.toolType || 'tool-system',
                  call.isRejected ? 'status-error' : 'status-running'
                ]">
                <div class="tool-line title-wrap" :class="{ 'tool-rejected': call.isRejected }">
                  <cs :name="call.icon || 'tool'" size="14px" class="tool-type-icon" />
                  <span class="tool-name">{{ call.action }}</span>
                  <span class="tool-target">{{ call.target }}</span>
                </div>
                <div class="tool-line summary">
                  <span class="corner-icon">⎿</span>
                  <span class="summary-text">{{ call.summary }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Streaming Chat State -->
    <div
      v-if="isChatting && (chatState.content || chatState.reasoning)"
      class="message assistant chatting">
      <div class="content-container">
        <div class="ai-content chat">
          <div v-if="chatState.reasoning" class="reasoning-container">
            <div class="reasoning-header">
              <cs
                name="reasoning"
                size="14px"
                class="reasoning-icon"
                :class="{ rotating: !chatState.content }" />
              <span class="reasoning-text">
                {{
                  chatState.content
                    ? $t('workflow.thoughtCompleted') || 'Thought Complete'
                    : getReasoningPreview(chatState.reasoning)
                }}
              </span>
            </div>
          </div>
          <!-- Streaming Blocks (Optimized rendering) -->
          <div v-for="(block, bIdx) in chatState.blocks" :key="bIdx">
            <!-- Output all blocks from the parser (paragraph, code, math, etc.) -->
            <MarkdownSimple :content="block.content" />
          </div>

          <!-- Retry Countdown... -->
          <div
            v-if="chatState.retryInfo && chatState.retryInfo.nextRetryIn > 0"
            class="retry-status-alert">
            <el-alert type="warning" :closable="false" show-icon>
              <template #title>
                {{
                  $t('workflow.retrying', {
                    attempt: chatState.retryInfo.attempt,
                    total: chatState.retryInfo.total,
                    seconds: chatState.retryInfo.nextRetryIn
                  })
                }}
              </template>
            </el-alert>
          </div>
        </div>
      </div>
    </div>

    <!-- Context Compression Status -->
    <div v-if="isCompressing" class="compression-status">
      <div class="compression-indicator">
        <cs name="loading" size="14px" class="rotating" />
        <span class="compression-text">{{ compressionMessage }}</span>
      </div>
    </div>

    <!-- Frontend queued user messages -->
    <div v-if="queuedMessages.length > 0" class="queued-list">
      <div v-for="item in queuedMessages" :key="item.id" class="queued-item">
        <cs name="clock" size="12px" class="queued-icon" />
        <span class="queued-text">{{ item.content }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { showMessage } from '@/libs/util'
import ApprovalDialog from './ApprovalDialog.vue'
import MarkdownSimple from './MarkdownSimple.vue'
import { useWorkflowStore } from '@/stores/workflow'

const workflowStore = useWorkflowStore()
const { t } = useI18n()
const CUSTOM_ASK_USER_VALUE = '__custom__'

const props = defineProps({
  messages: {
    type: Array,
    default: () => []
  },
  queuedMessages: {
    type: Array,
    default: () => []
  },
  isRunning: {
    type: Boolean,
    default: false
  },
  isChatting: {
    type: Boolean,
    default: false
  },
  chatState: {
    type: Object,
    default: () => ({
      content: '',
      reasoning: '',
      blocks: [],
      retryInfo: null
    })
  },
  isCompressing: {
    type: Boolean,
    default: false
  },
  compressionMessage: {
    type: String,
    default: ''
  },
  lastAssistantMessage: {
    type: Object,
    default: null
  },
  approvalLoading: {
    type: Boolean,
    default: false
  },
  activeApprovalId: {
    type: String,
    default: ''
  },
  isMessageExpanded: {
    type: Function,
    required: true
  },
  isReasoningExpanded: {
    type: Function,
    required: true
  },
  removeSystemReminder: {
    type: Function,
    required: true
  },
  getDiffMarkdown: {
    type: Function,
    required: true
  },
  parseChoiceContent: {
    type: Function,
    required: true
  },
  getParsedMessage: {
    type: Function,
    required: true
  },
  shouldShowToolRawContent: {
    type: Function,
    required: true
  },
  getReasoningPreview: {
    type: Function,
    required: true
  },
  askUserSubmitting: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits([
  'toggle-expand',
  'toggle-reasoning',
  'scroll-bottom',
  'approve-tool',
  'approve-all-tool',
  'reject-tool',
  'submit-ask-user'
])

const messagesRef = ref(null)
const approvalDrafts = ref({})
const askUserDrafts = ref({})

const getApprovalDraft = toolCallId => {
  if (!toolCallId) return ''
  return approvalDrafts.value[toolCallId] || ''
}

const setApprovalDraft = (toolCallId, value) => {
  if (!toolCallId) return
  approvalDrafts.value = {
    ...approvalDrafts.value,
    [toolCallId]: value
  }
}

const getChoiceKey = message =>
  message.metadata?.tool_call_id || message.displayId || message.id || ''

const getAskUserResponseItems = message => {
  const content = message?.message || ''
  const match = content.match(/<ask_user_response>\s*([\s\S]*?)\s*<\/ask_user_response>/i)
  if (!match) return []

  try {
    const parsed = JSON.parse(match[1])
    return Array.isArray(parsed) ? parsed : []
  } catch (error) {
    return []
  }
}

const formatAskUserAnswer = item => {
  if (!item) return ''
  if (item.source === 'custom') {
    return `${t('workflow.askUser.customLabel')} (${item.choice_index})`
  }
  return item.choice_index ? `${item.choice_index}. ${item.choice}` : item.choice || ''
}

const getVisibleUserContent = message => props.removeSystemReminder(message?.message || '')

const getChoiceGroups = message =>
  props.parseChoiceContent(props.removeSystemReminder(message.message || '')).groups || []

const ensureAskUserDraft = message => {
  const key = getChoiceKey(message)
  if (!key) return {}
  if (askUserDrafts.value[key]) return askUserDrafts.value[key]

  const groups = getChoiceGroups(message)
  const nextDraft = groups.reduce((acc, group) => {
    acc[group.title] = {
      selection: '',
      customInput: ''
    }
    return acc
  }, {})

  askUserDrafts.value = {
    ...askUserDrafts.value,
    [key]: nextDraft
  }

  return nextDraft
}

const updateAskUserDraft = (message, updater) => {
  const key = getChoiceKey(message)
  if (!key) return
  const current = ensureAskUserDraft(message)
  askUserDrafts.value = {
    ...askUserDrafts.value,
    [key]: updater(current)
  }
}

const getAskUserSelection = (message, title) => ensureAskUserDraft(message)[title]?.selection || ''

const setAskUserSelection = (message, title, value) => {
  updateAskUserDraft(message, current => ({
    ...current,
    [title]: {
      ...current[title],
      selection: value
    }
  }))
}

const getAskUserCustomInput = (message, title) =>
  ensureAskUserDraft(message)[title]?.customInput || ''

const setAskUserCustomInput = (message, title, value) => {
  updateAskUserDraft(message, current => ({
    ...current,
    [title]: {
      ...current[title],
      selection: value?.trim() ? CUSTOM_ASK_USER_VALUE : current[title]?.selection,
      customInput: value
    }
  }))
}

const hasRealUserResponseAfter = fromIndex => {
  for (let i = fromIndex + 1; i < props.messages.length; i++) {
    const msg = props.messages[i]
    if (msg?.role !== 'user') continue
    if (msg?.metadata?.queue_status === 'queued') continue
    const content = props.removeSystemReminder(msg.message || '').trim()
    if (!content) continue
    return true
  }
  return false
}

const canAnswerAskUser = (message, index) => {
  if (props.isRunning) return false
  if (!getChoiceGroups(message).length) return false
  return !hasRealUserResponseAfter(index)
}

const buildAskUserResponse = message => {
  const groups = getChoiceGroups(message)
  const draft = ensureAskUserDraft(message)
  const selections = []

  for (const group of groups) {
    const groupDraft = draft[group.title] || {}
    const selection = groupDraft.selection || ''
    const customInput = (groupDraft.customInput || '').trim()

    if (!selection) {
      return {
        ok: false,
        error: 'workflow.askUser.validationRequired'
      }
    }

    if (selection === CUSTOM_ASK_USER_VALUE) {
      if (!customInput) {
        return {
          ok: false,
          error: 'workflow.askUser.validationCustomRequired'
        }
      }

      selections.push({
        title: group.title,
        choice_index: group.options.length + 1,
        choice: customInput,
        source: 'custom'
      })
      continue
    }

    const optionIndex = group.options.findIndex(option => option === selection)
    if (optionIndex === -1) {
      return {
        ok: false,
        error: 'workflow.askUser.validationRequired'
      }
    }

    selections.push({
      title: group.title,
      choice_index: optionIndex + 1,
      choice: selection,
      source: 'option'
    })
  }

  return {
    ok: true,
    content: `<ask_user_response>\n${JSON.stringify(selections, null, 2)}\n</ask_user_response>`
  }
}

const submitAskUserResponse = message => {
  const result = buildAskUserResponse(message)
  if (!result.ok) {
    showMessage(t(result.error), 'warning')
    return
  }

  emit('submit-ask-user', result.content)
}

const scrollToBottom = (force = false) => {
  if (messagesRef.value) {
    const el = messagesRef.value
    // Increase threshold to 300px to handle tall tool call blocks
    // If 'force' is true, we scroll regardless of current position
    const isAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 300

    if (force || isAtBottom) {
      nextTick(() => {
        el.scrollTop = el.scrollHeight
      })
    }
  }
}

defineExpose({
  scrollToBottom,
  messagesRef
})
</script>
