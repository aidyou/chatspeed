<template>
  <div class="messages" ref="messagesRef">
    <div v-for="(message, index) in messages" :key="message.displayId" class="message"
      :class="[message.role, message.stepType?.toLowerCase(), { 'is-error': message.isError }]">
      <div class="avatar" v-if="message.role === 'user'">
        <cs name="talk" class="user-icon" />
      </div>
      <div class="content-container">
        <div class="content" v-if="message.role === 'user'">
          <pre class="simple-text">{{ message.message }}</pre>
        </div>
        <div v-else class="ai-content chat">
          <!-- CLI Style Tool Call (Results) -->
          <div v-if="message.role === 'tool'" class="cli-tool-call"
            :class="[message.toolDisplay.toolType || 'tool-system', message.toolDisplay.isError ? 'status-error' : 'status-success']">
            <!-- finish_task special display -->
            <template
              v-if="message.toolDisplay?.action === $t('workflow.finishTask') || message.toolDisplay?.action?.includes('Finish')">
              <div class="tool-line finish-task-display">
                <cs :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'" size="14px"
                  class="tool-type-icon finish-icon" />
                <span class="finish-text">{{ $t('workflow.finishTask') }}</span>
              </div>
            </template>

            <!-- Normal tool call display -->
            <template v-else>
              <div class="tool-line title-wrap expandable" :class="{ 'tool-rejected': message.isRejected }"
                @click="$emit('toggle-expand', message.displayId)">
                <cs :name="message.toolDisplay.icon || 'tool'" size="14px" class="tool-type-icon" />
                <span class="tool-name">{{ message.toolDisplay.action }}</span>
                <span class="tool-target">{{ message.toolDisplay.target }}</span>
                <cs v-if="message.isApproved" name="check" size="14px" class="approved-icon" />
              </div>
              <!-- Hide summary when expanded -->
              <div class="tool-line summary expandable" v-if="!isMessageExpanded(message)"
                @click="$emit('toggle-expand', message.displayId)">
                <span class="corner-icon">⎿</span>
                <span class="summary-text">{{ message.toolDisplay.summary }}</span>
                <span class="expand-hint">(click to expand)</span>
              </div>
              <div v-if="isMessageExpanded(message)" class="tool-detail">
                <!-- Tool Stream Output (for bash commands) -->
                <div v-if="message.metadata?.tool_call_id && workflowStore.getToolStream(message.metadata.tool_call_id).length > 0"
                  class="tool-stream-output">
                  <div v-for="(line, idx) in workflowStore.getToolStream(message.metadata.tool_call_id)"
                    :key="idx" class="stream-line">
                    {{ line }}
                  </div>
                </div>
                <!-- Final Result -->
                <MarkdownSimple v-if="message.toolDisplay.displayType === 'diff'"
                  :content="getDiffMarkdown(removeSystemReminder(message.message))" />
                <div v-else-if="message.toolDisplay.displayType === 'choice'" class="choice-container">
                  <div class="choice-question">{{
                    parseChoiceContent(removeSystemReminder(message.message)).question
                  }}
                  </div>
                  <div class="choice-options">
                    <el-button v-for="opt in parseChoiceContent(removeSystemReminder(message.message)).options"
                      :key="opt" size="small" plain round :disabled="isRunning" @click="$emit('send-choice', opt)">
                      {{ opt }}
                    </el-button>
                  </div>
                </div>
                <pre v-else class="raw-content">{{ removeSystemReminder(message.message) }}</pre>
              </div>
            </template>
          </div>

          <!-- Regular Assistant Content -->
          <div v-else>
            <!-- Thought/Content FIRST (Separate reasoning field has priority) -->
            <div v-if="message.reasoning || message.stepType === 'Think'" class="reasoning-container">
              <div class="reasoning-header" @click="$emit('toggle-reasoning', message.displayId)">
                <cs name="reasoning" size="14px" class="reasoning-icon" :class="{
                  rotating: isRunning && !getParsedMessage(message).content &&
                    (message.metadata?.tool_calls?.length || 0) === 0 &&
                    !isReasoningExpanded(message.displayId) &&
                    message === lastAssistantMessage
                }" />
                <span class="reasoning-text" :class="{ expanded: isReasoningExpanded(message.displayId) }">
                  <template v-if="isReasoningExpanded(message.displayId)">
                    {{ $t('workflow.thinkingExpanded') || 'Thinking Process' }}
                  </template>
                  <template v-else-if="isRunning && !getParsedMessage(message).content &&
                    (message.metadata?.tool_calls?.length || 0) === 0 &&
                    message === lastAssistantMessage">
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
            <MarkdownSimple v-if="getParsedMessage(message).content" :content="getParsedMessage(message).content" />

            <!-- Tool Call Indicators SECOND (Only pending ones) -->
            <div v-if="message.pendingToolCalls?.length > 0" class="cli-tool-calls-container">
              <div v-for="call in message.pendingToolCalls" :key="call.id" class="cli-tool-call pending"
                :class="[call.toolType || 'tool-system', 'status-running']">
                <div class="tool-line title-wrap">
                  <cs :name="call.icon || 'tool'" size="14px" class="tool-type-icon" />
                  <span class="tool-name">{{ call.action }}</span>
                  <span class="tool-target">{{ call.target }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Streaming Chat State -->
    <div v-if="isChatting && (chatState.content || chatState.reasoning)" class="message assistant chatting">
      <div class="content-container">
        <div class="ai-content chat">
          <div v-if="chatState.reasoning" class="reasoning-container">
            <div class="reasoning-header">
              <cs name="reasoning" size="14px" class="reasoning-icon" :class="{ rotating: !chatState.content }" />
              <span class="reasoning-text">
                {{ chatState.content ? ($t('workflow.thoughtCompleted') || 'Thought Complete') :
                  getReasoningPreview(chatState.reasoning) }}
              </span>
            </div>
          </div>
          <!-- Streaming Blocks (Optimized rendering) -->
          <div v-for="(block, bIdx) in chatState.blocks" :key="bIdx">
            <!-- Output all blocks from the parser (paragraph, code, math, etc.) -->
            <MarkdownSimple :content="block.content" />
          </div>

          <!-- Retry Countdown... -->
          <div v-if="chatState.retryInfo && chatState.retryInfo.nextRetryIn > 0" class="retry-status-alert">
            <el-alert type="warning" :closable="false" show-icon>
              <template #title>
                {{ $t('workflow.retrying', {
                  attempt: chatState.retryInfo.attempt,
                  total: chatState.retryInfo.total,
                  seconds: chatState.retryInfo.nextRetryIn
                }) }}
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
  </div>
</template>

<script setup>
import { ref, nextTick } from 'vue'
import MarkdownSimple from './MarkdownSimple.vue'
import { useWorkflowStore } from '@/stores/workflow'

const workflowStore = useWorkflowStore()

defineProps({
  messages: {
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
  getReasoningPreview: {
    type: Function,
    required: true
  }
})

defineEmits([
  'toggle-expand',
  'toggle-reasoning',
  'send-choice',
  'scroll-bottom'
])

const messagesRef = ref(null)

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
