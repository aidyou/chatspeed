<template>
  <div class="content">
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
      <div
        v-if="showThink"
        class="think-content"
        v-highlight
        v-link
        v-table
        v-katex
        v-html="parseMarkdown(reasoning)"></div>
    </div>
    <div v-html="currentAssistantMessageHtml" v-highlight v-link v-table v-katex v-think />
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
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
  },
  isReasoning: {
    type: Boolean,
    default: false
  }
})

const showReference = ref(false)
const showThink = ref(true)

const cicleIndex = ref(0)
const cicle = ['◒', '◐', '◓', '◑', '☯']
const currentAssistantMessageHtml = computed(() =>
  props.content
    ? ((cicleIndex.value = (cicleIndex.value + 1) % 5),
      parseMarkdown(props.content + cicle[cicleIndex.value], props.reference))
    : '<div class="cs cs-loading cs-spin"></div>'
)
</script>
