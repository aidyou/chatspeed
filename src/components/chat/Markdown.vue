<template>
  <div :class="className">
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
        <span>{{ $t('chat.reasoningProcess') }}</span>
      </div>
      <div
        v-if="showThink"
        class="think-content"
        v-highlight
        v-katex
        v-link
        v-table
        v-html="parseMarkdown(reasoning)"></div>
    </div>
    <div
      v-highlight
      v-katex
      v-link
      v-mermaid
      v-table
      v-think
      v-html="parseMarkdown(content, reference)" />
  </div>
</template>
<script setup>
import { ref } from 'vue'
import { parseMarkdown } from '@/libs/chat'

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
  }
})

const showReference = ref(false)
const showThink = ref(true)
</script>
