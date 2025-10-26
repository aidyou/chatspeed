<template>
  <div class="content" ref="contentRef">
    <div class="chat-reference" v-if="reference.length > 0">
      <div
        class="chat-reference-title"
        :class="{ expanded: showReference }"
        @click="showReference = !showReference">
        <span>{{ $t('chat.reference', { count: reference.length }) }}</span>
      </div>
      <ul class="chat-reference-list" v-show="showReference" v-link>
        <li v-for="item in reference" :key="item.id">
          <a :href="item.url" :title="item.title.trim()">{{ item.title.trim() }}</a>
        </li>
      </ul>
    </div>

    <div class="chat-think" v-if="reasoning != ''">
      <div
        class="chat-think-title"
        :class="{ expanded: showThink }"
        @click="showThink = !showThink">
        <span>{{ $t(`chat.${isReasoning ? 'reasoning' : 'reasoningProcess'}`) }}</span>
      </div>
      <div v-if="showThink" class="think-content">
        <!-- Wrapper for each block, contains directives -->
        <div
          v-for="(think, index) in finalThoughts"
          :key="`thought-${index}`"
          v-think
          v-highlight
          v-link
          v-table
          v-katex>
          <div v-html="parseMarkdown(think, [], [])"></div>
        </div>
      </div>
    </div>

    <div
      v-for="(content, index) in finalContents"
      :key="`content-${index}`"
      v-highlight
      v-katex
      v-link
      v-mermaid
      v-table
      v-think
      v-tools>
      <div v-html="parseMarkdown(content, [], [])"></div>
    </div>
    <div v-if="step" class="step">{{ step }}</div>
    <span class="cs cs-spin-linear" v-if="isChatting">☯</span>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { formatReference, parseMarkdown, htmlspecialchars, toolName } from '@/libs/chat'
import { MarkdownStreamParser } from '@/libs/markdown-stream-parser.js'
import i18n from '@/i18n/index.js'
import { watch } from 'vue'

const contentRef = ref(null)

defineExpose({
  contentRef
})

const props = defineProps({
  content: {
    type: String,
    required: true
  },
  className: {
    type: String,
    default: 'content'
  },
  reference: {
    type: Array,
    default: () => []
  },
  reasoning: {
    type: String,
    default: ''
  },
  isReasoning: {
    type: Boolean,
    default: false
  },
  isChatting: {
    type: Boolean,
    default: false
  },
  toolCalls: {
    type: Array,
    default: () => []
  },
  step: {
    type: String,
    default: ''
  }
})

const showReference = ref(false)
const showThink = ref(true)

const contentParser = new MarkdownStreamParser()
const thoughtParser = new MarkdownStreamParser()

const contentBlocks = computed(() => {
  return props.content ? contentParser.process(props.content).map(b => b.content) : []
})
const thoughtBlocks = computed(() => {
  return props.reasoning ? thoughtParser.process(props.reasoning).map(b => b.content) : []
})

const finalContents = computed(() =>
  createFinalBlocks(contentBlocks.value, props.toolCalls, props.reference)
)
const finalThoughts = computed(() =>
  createFinalBlocks(thoughtBlocks.value, props.toolCalls, props.reference)
)

watch(
  () => props.toolCalls,
  () => {
    contentParser.reset()
    thoughtParser.reset()
  }
)

const createSingleToolCallHtml = call => {
  const functionName = call?.function?.name
  if (!functionName) return ''

  let status = 'calling'
  if (call.result) {
    try {
      const parsedResult = typeof call.result === 'string' ? JSON.parse(call.result) : call.result
      status = parsedResult?.error ? 'error' : 'success'
    } catch {
      status =
        typeof call.result === 'string' && call.result.startsWith('Error: ') ? 'error' : 'success'
    }
  }

  const args = call.function.arguments ? JSON.parse(call.function.arguments) : {}

  let toolDisplayName = ''
  if (functionName.includes('__MCP__')) {
    const names = functionName.split('__MCP__')
    toolDisplayName = toolName(names.length === 2 ? `${names[0]}::${names[1]}` : functionName, args)
  } else {
    toolDisplayName = toolName(functionName, args)
  }

  const result =
    typeof call.result === 'string' ? call.result : JSON.stringify(call.result, null, 2)
  const escapedArguments = htmlspecialchars(call.function.arguments || '')

  return `<div class="chat-tool-calls">
    <div class="tool-name ${status}"><span>${toolDisplayName}</span></div>
    <div class="tool-codes" style="display:none;">
      <div class="tool-code"><h3>📝 ${i18n.global.t('chat.toolArgs')}</h3>
         <pre><code class="language-json">${escapedArguments}</code></pre>
      </div>
      <div class="tool-code"><h3>🎯 ${i18n.global.t('chat.toolResult')}</h3>
        <code data-result="${encodeURIComponent(result)}" class="tool-results"></code>
      </div>
    </div>
  </div>`
}

const createFinalBlocks = (blocks, toolCalls, references) => {
  let toolCallIdx = 0
  const allToolCalls = toolCalls || []
  const allReferences = references || []

  return blocks.map(blockContent => {
    let newContent = blockContent

    while (newContent.includes('<!--[ToolCalls]-->') && toolCallIdx < allToolCalls.length) {
      const toolHtml = createSingleToolCallHtml(allToolCalls[toolCallIdx])
      newContent = newContent.replace('<!--[ToolCalls]-->', toolHtml)
      toolCallIdx++
    }

    if (allReferences.length > 0) {
      newContent = formatReference(newContent, allReferences)
    }

    return newContent
  })
}
</script>

<style scoped>
.step {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  display: block;
}
</style>
