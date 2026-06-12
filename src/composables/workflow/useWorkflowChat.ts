import { ref, computed, onUnmounted } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
import { MarkdownStreamParser } from '@/libs/markdown-stream-parser'
import { useI18n } from 'vue-i18n'

/**
 * Composable for managing chat/streaming state
 * Handles real-time message streaming, retry countdown, and compression status
 */
export function useWorkflowChat({ currentWorkflowId }) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  const chattingParser = new MarkdownStreamParser()
  const THINK_BLOCK_REGEX = /<(think|thinking)(?:\s+class="[^"]*")?>([\s\S]*?)<\/\1>/gi
  const THINK_OPEN_REGEX = /<(?:think|thinking)(?:\s+class="[^"]*")?>/i
  const chatState = ref({
    rawContent: '',
    content: '',
    reasoning: '',
    explicitReasoning: '',
    reasoningStatus: 'idle',
    blocks: [],
    retryInfo: null
  })
  const compressionStates = ref({})

  let retryCountdownTimer = null

  const isChatting = computed(() => workflowStore.isRunning)

  const clearRetryTimer = () => {
    if (retryCountdownTimer) {
      clearInterval(retryCountdownTimer)
      retryCountdownTimer = null
    }
    chatState.value.retryInfo = null
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
    chatState.value.rawContent = ''
    chatState.value.content = ''
    chatState.value.reasoning = ''
    chatState.value.explicitReasoning = ''
    chatState.value.reasoningStatus = 'idle'
    chatState.value.blocks = []
    chatState.value.retryInfo = null
  }

  const extractInlineReasoning = rawContent => {
    const reasoningParts = []
    let visibleContent = String(rawContent || '').replace(THINK_BLOCK_REGEX, (_match, _tagName, innerContent) => {
      const normalized = String(innerContent || '').trim()
      if (normalized) reasoningParts.push(normalized)
      return ''
    })

    const trailingThinkIndex = visibleContent.search(THINK_OPEN_REGEX)
    if (trailingThinkIndex >= 0) {
      const trailingReasoning = visibleContent
        .slice(trailingThinkIndex)
        .replace(THINK_OPEN_REGEX, '')
        .trim()
      if (trailingReasoning) reasoningParts.push(trailingReasoning)
      visibleContent = visibleContent.slice(0, trailingThinkIndex)
    }

    return {
      content: visibleContent,
      reasoning: reasoningParts.join('\n\n').trim(),
      hasOpenThink: trailingThinkIndex >= 0
    }
  }

  const refreshDerivedChatState = (source = 'chunk') => {
    const { content, reasoning, hasOpenThink } = extractInlineReasoning(chatState.value.rawContent)
    const combinedReasoning = [reasoning, chatState.value.explicitReasoning]
      .map(part => String(part || '').trim())
      .filter(Boolean)
      .join('\n\n')

    const hadStreamingReasoning = chatState.value.reasoningStatus === 'streaming'
    let reasoningStatus = 'idle'

    if (combinedReasoning) {
      if (source === 'reasoning' || hasOpenThink) {
        reasoningStatus = 'streaming'
      } else if (content.trim()) {
        reasoningStatus = 'done'
      } else if (hadStreamingReasoning) {
        reasoningStatus = 'streaming'
      } else {
        reasoningStatus = 'done'
      }
    }

    chattingParser.reset()
    chatState.value.content = content
    chatState.value.reasoning = combinedReasoning
    chatState.value.reasoningStatus = reasoningStatus
    chatState.value.blocks = content ? chattingParser.process(content) : []
  }

  // Handle retry status with countdown
  const setRetryStatus = (payload) => {
    chatState.value.retryInfo = null
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
    clearRetryTimer()
    workflowStore.setNotification('', 'info')
    chatState.value.rawContent += content
    refreshDerivedChatState('chunk')
  }

  // Handle reasoning chunk
  const processReasoningChunk = (content) => {
    clearRetryTimer()
    workflowStore.setNotification('', 'info')
    chatState.value.explicitReasoning += content
    refreshDerivedChatState('reasoning')
  }

  // Set compression status
  const isCompressing = computed(() => {
    const sessionId = currentWorkflowId?.value
    if (!sessionId) return false
    return !!compressionStates.value[sessionId]?.isCompressing
  })

  const compressionMessage = computed(() => {
    const sessionId = currentWorkflowId?.value
    if (!sessionId) return ''
    return compressionStates.value[sessionId]?.message || ''
  })

  const setCompressionStatus = (sessionId, isCompressingValue, message) => {
    if (!sessionId) return

    const nextStates = { ...compressionStates.value }
    if (isCompressingValue) {
      nextStates[sessionId] = {
        isCompressing: true,
        message: message || ''
      }
    } else {
      delete nextStates[sessionId]
    }
    compressionStates.value = nextStates
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
