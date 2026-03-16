import { ref, computed, onUnmounted } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
import { MarkdownStreamParser } from '@/libs/markdown-stream-parser'
import { useI18n } from 'vue-i18n'

/**
 * Composable for managing chat/streaming state
 * Handles real-time message streaming, retry countdown, and compression status
 */
export function useWorkflowChat() {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  const chattingParser = new MarkdownStreamParser()
  const chatState = ref({
    content: '',
    reasoning: '',
    blocks: [],
    retryInfo: null
  })
  const isCompressing = ref(false)
  const compressionMessage = ref('')

  let retryCountdownTimer = null

  const isChatting = computed(() => workflowStore.isRunning)

  const clearRetryTimer = () => {
    if (retryCountdownTimer) {
      clearInterval(retryCountdownTimer)
      retryCountdownTimer = null
    }
  }

  // Get last sentence from text (split by punctuation)
  const getLastSentence = (text) => {
    if (!text) return ''
    const sentences = text.split(/(?<=[。！？.!?])\s*/).filter((s) => s.trim())
    return sentences[sentences.length - 1] || text.slice(-50)
  }

  // Get preview text for reasoning (last sentence with max length)
  const getReasoningPreview = (text, maxLen = 50) => {
    if (!text) return t('workflow.thinking') || 'Thinking...'
    const last = getLastSentence(text)
    if (last.length <= maxLen) return last
    return last.slice(0, maxLen) + '...'
  }

  // Reset chat state
  const resetChatState = () => {
    chattingParser.reset()
    chatState.value.content = ''
    chatState.value.reasoning = ''
    chatState.value.blocks = []
    chatState.value.retryInfo = null
  }

  // Handle retry status with countdown
  const setRetryStatus = (payload) => {
    chatState.value.retryInfo = {
      attempt: payload.attempt,
      total: payload.total_attempts,
      nextRetryIn: payload.next_retry_in_seconds
    }

    // Auto-decrement timer
    clearRetryTimer()
    retryCountdownTimer = setInterval(() => {
      if (chatState.value.retryInfo && chatState.value.retryInfo.nextRetryIn > 0) {
        chatState.value.retryInfo.nextRetryIn--
      } else {
        clearRetryTimer()
      }
    }, 1000)
  }

  // Handle chunk for streaming
  const processChunk = (content) => {
    chatState.value.content += content
    chatState.value.blocks = chattingParser.process(content)
  }

  // Handle reasoning chunk
  const processReasoningChunk = (content) => {
    chatState.value.reasoning += content
  }

  // Set compression status
  const setCompressionStatus = (isCompressingValue, message) => {
    isCompressing.value = isCompressingValue
    compressionMessage.value = message
  }

  onUnmounted(() => {
    clearRetryTimer()
  })

  return {
    chattingParser,
    chatState,
    isChatting,
    isCompressing,
    compressionMessage,
    clearRetryTimer,
    getLastSentence,
    getReasoningPreview,
    resetChatState,
    setRetryStatus,
    processChunk,
    processReasoningChunk,
    setCompressionStatus
  }
}
