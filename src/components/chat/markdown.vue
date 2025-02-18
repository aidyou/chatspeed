<template>
  <div
    ref="contentRef"
    :class="props.className"
    v-html="parseMarkdown(props.content, props.reference)"
    v-highlight
    v-link
    v-table
    v-katex
    v-mermaid
    v-think
    v-reference />
</template>
<script setup>
import { ref } from 'vue'
import { parseMarkdown } from '@/libs/chat'

const contentRef = ref(null)

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
  }
})
</script>
