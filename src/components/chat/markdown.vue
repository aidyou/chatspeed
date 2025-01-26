<template>
  <div
    ref="contentRef"
    :class="props.className"
    v-html="parseMarkdown(props.content)"
    v-highlight
    v-link
    v-katex
    v-mermaid
    v-think />
</template>
<script setup>
import { ref } from 'vue'
import { parseMarkdown } from '@/libs/chat'

const contentRef = ref(null)

const props = defineProps({
  content: {
    type: String,
    required: true,
  },
  className: {
    type: String,
    default: 'content',
  },
})
</script>
