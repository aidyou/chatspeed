<template>
  <div ref="containerRef" :class="className"></div>
</template>

<script setup>
import { ref, watch, onMounted, nextTick } from 'vue'
import hljs from 'highlight.js'
import { parseMarkdown } from '@/libs/chat'

const props = defineProps({
  content: {
    type: String,
    required: true
  },
  className: {
    type: String,
    default: 'content'
  }
})

const containerRef = ref(null)

// Simple highlight function - no title bar, no copy button
const highlightCode = () => {
  if (!containerRef.value) return

  containerRef.value.querySelectorAll('pre code').forEach(block => {
    // Skip if already highlighted
    if (block.getAttribute('data-highlighted') === 'yes') return

    hljs.highlightElement(block)
    // Mark as highlighted to avoid re-processing
    block.setAttribute('data-highlighted', 'yes')
  })
}

// Watch for content changes and re-highlight
watch(
  () => props.content,
  async () => {
    if (!containerRef.value) return

    // Parse markdown and set innerHTML
    containerRef.value.innerHTML = parseMarkdown(props.content)

    // Wait for DOM to update then highlight
    await nextTick()
    highlightCode()
  }
)

onMounted(async () => {
  if (!containerRef.value) return

  // Initial render
  containerRef.value.innerHTML = parseMarkdown(props.content)

  // Wait for DOM to update then highlight
  await nextTick()
  highlightCode()
})
</script>

<style lang="scss">
// Enhanced styling for markdown content with better code blocks
.content {

  // Inline code - subtle highlight
  code {
    font-family: var(--cs-font-family-mono, monospace);
    font-size: 0.9em;
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--cs-code-text-color);
  }

  // Code blocks - enhanced visual design
  pre {
    code.hljs {
      border-radius: var(--cs-border-radius-md) !important;
      line-height: 1.5;
      font-size: var(--cs-font-size);
    }

  }

  // Tables - enhanced styling
  table {
    max-width: 100%;
    min-width: unset !important;
    border-collapse: collapse;
    margin: var(--cs-space) 0;
    font-size: 14px;
    border-radius: var(--cs-border-radius-sm);
    overflow: hidden;
    border: 1px solid var(--cs-border-color);

    th,
    td {
      padding: 10px 14px;
      text-align: left;
    }

    th {
      background: var(--cs-bg-color-light);
      font-weight: 600;
    }

    tr:nth-child(even) {
      background: var(--cs-bg-color-light);
    }
  }

  // Blockquotes
  blockquote {
    margin: 16px 0;
    padding: 12px 20px;
    border-left: 4px solid var(--cs-color-primary);
    background: var(--cs-bg-color-light);
    color: var(--cs-text-color-secondary);
    border-radius: 0 var(--cs-border-radius-sm) var(--cs-border-radius-sm) 0;
    font-style: italic;

    p {
      margin: 0;
    }
  }

  // Lists
  ul,
  ol {
    margin: 12px 0;
    padding-left: 28px;

    li {
      margin: 6px 0;
      line-height: 1.6;
    }
  }

  // Images
  img {
    max-width: 100%;
    height: auto;
    border-radius: var(--cs-border-radius-sm);
    box-shadow: 0 2px 8px var(--cs-shadow-color);
  }

  // Headings
  h1,
  h2,
  h3,
  h4,
  h5,
  h6 {
    margin: 24px 0 12px;
    font-weight: 600;
    line-height: 1.4;
    color: var(--cs-text-color-primary);
  }

  h1 {
    font-size: 1.8em;
  }

  h2 {
    font-size: 1.5em;
  }

  h3 {
    font-size: 1.25em;
  }

  h4 {
    font-size: 1.1em;
  }

  // Paragraphs
  p {
    margin: 12px 0;
    line-height: 1.7;
  }

  // Horizontal rule
  hr {
    border: none;
    border-top: 1px solid var(--cs-border-color);
    margin: 24px 0;
  }

  // Mermaid diagrams
  .mermaid,
  .markmap {
    margin: 16px 0;
    padding: 16px;
    background: var(--cs-bg-color-light);
    border-radius: var(--cs-border-radius-sm);
    text-align: center;

    &:empty::before {
      content: 'Diagram';
      color: var(--cs-text-color-placeholder);
    }
  }
}
</style>