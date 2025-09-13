<template>
  <div class="chat-tool-calls" v-for="(call, index) in toolParsed" :key="index">
    <div class="tool-name" :class="{ expanded: showToolCalls }" @click="showToolCalls = !showToolCalls">
      <template v-if="call.function?.mcpName">
        {{ i18n.global.t('chat.mcpCall') }} {{ call.function.mcpName }}::{{
          call.function.name
        }}
      </template>
      <template v-else>
        {{ i18n.global.t('chat.toolCall') }} {{ call.function.name }}
      </template>
    </div>
    <div class="tool-codes" v-show="showToolCalls">
      <div class="tool-code">
        <h3>📝 {{ i18n.global.t('chat.toolArgs') }}</h3>
        <pre><code disable-titlebar>{{ call.function.arguments }}</code></pre>
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

const props = defineProps({
  toolCalls: {
    type: Array,
    default: () => []
  }
})

const showToolCalls = ref(false)

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
</script>