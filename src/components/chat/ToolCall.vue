<template>
  <div class="chat-tool-calls" v-for="(call, index) in toolParsed" :key="index">
    <div
      class="tool-name"
      :class="{ expanded: expandedState[index], [toolStatus(call.result)]: true }"
      @click="expandedState[index] = !expandedState[index]">
      <template v-if="call.function?.mcpName">
        <span>{{
          toolName(
            call.function.mcpName + '::' + call.function.name,
            args(call?.function?.arguments)
          )
        }}</span>
      </template>
      <template v-else>
        <span>{{ toolName(call.function.name, args(call?.function?.arguments)) }}</span>
      </template>
    </div>
    <div class="tool-codes" v-show="expandedState[index]">
      <div class="tool-code">
        <h3>📝 {{ i18n.global.t('chat.toolArgs') }}</h3>
        <pre><code class="language-json" disable-titlebar>{{ call?.function?.arguments }}</code></pre>
      </div>
      <div class="tool-code">
        <h3>🎯 {{ i18n.global.t('chat.toolResult') }}</h3>
        <pre><code class="tool-results">{{ call.result }}</code></pre>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import i18n from '@/i18n/index.js'
import { toolName } from '@/libs/chat.js'

const props = defineProps({
  toolCalls: {
    type: Array,
    default: () => []
  }
})

const expandedState = ref({})

const toolParsed = computed(() => {
  if (!props.toolCalls || !props.toolCalls.length) {
    return []
  }
  return props.toolCalls.map(originalCall => {
    const call = JSON.parse(JSON.stringify(originalCall))

    if (call?.function?.name?.includes('__MCP__')) {
      const names = call.function.name.split('__MCP__')
      if (names.length === 2) {
        call.function.mcpName = names[0]
        call.function.name = names[1]
      }
    }

    if (typeof call.result === 'string') {
      try {
        const parsedResult = JSON.parse(call.result)
        call.result = JSON.stringify(parsedResult, null, 2)
      } catch (e) {
        // Keep original string if not valid JSON
      }
    }

    return call
  })
})
const toolStatus = result => {
  if (!result) {
    return 'calling'
  } else {
    try {
      const parsedResult = typeof result === 'string' ? JSON.parse(result) : result
      return parsedResult?.error ? 'error' : 'success'
    } catch {
      return typeof result === 'string' && result.startsWith('Error: ') ? 'error' : 'success'
    }
  }
}

const args = argString => {
  if (!argString) {
    return []
  }
  try {
    return JSON.parse(argString)
  } catch {
    return argString
  }
}
</script>
