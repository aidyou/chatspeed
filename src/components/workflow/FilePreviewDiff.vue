<template>
  <div class="file-preview-diff">
    <div class="diff-text">
      <div
        v-for="(line, index) in diffLines"
        :key="`${index}-${line.oldLineNumber}-${line.newLineNumber}-${line.prefix}`"
        class="diff-line"
        :class="line.type">
        <span class="diff-prefix" :data-prefix="line.prefix" aria-hidden="true"></span>
        <span class="diff-line-number">{{ line.oldLineNumber }}</span>
        <span class="diff-line-number">{{ line.newLineNumber }}</span>
        <span class="diff-separator">|</span>
        <span class="diff-content" v-html="line.html"></span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import * as Diff from 'diff'
import hljs from 'highlight.js'

const props = defineProps({
  filePath: {
    type: String,
    default: ''
  },
  oldContent: {
    type: String,
    default: ''
  },
  newContent: {
    type: String,
    default: ''
  }
})

const language = computed(() => {
  const ext = props.filePath.split('.').pop()?.toLowerCase() || ''
  const mapping = {
    rs: 'rust',
    js: 'javascript',
    jsx: 'javascript',
    ts: 'typescript',
    tsx: 'typescript',
    vue: 'xml',
    py: 'python',
    go: 'go',
    java: 'java',
    kt: 'kotlin',
    swift: 'swift',
    css: 'css',
    scss: 'scss',
    html: 'xml',
    xml: 'xml',
    json: 'json',
    yaml: 'yaml',
    yml: 'yaml',
    toml: 'toml',
    md: 'markdown',
    sh: 'bash',
    zsh: 'bash'
  }
  return mapping[ext] || ''
})

const highlightLine = line => {
  if (!line) return '&nbsp;'
  if (!language.value) {
    return hljs.highlightAuto(line).value
  }
  try {
    return hljs.highlight(line, { language: language.value }).value
  } catch {
    return hljs.highlightAuto(line).value
  }
}

const createDiffLine = (prefix, oldLineNumber, newLineNumber, content, type) => ({
  prefix,
  oldLineNumber,
  newLineNumber,
  content,
  html: highlightLine(content),
  type
})

const diffLines = computed(() => {
  const oldStr = props.oldContent ?? ''
  const newStr = props.newContent ?? ''
  const lines = []

  if (oldStr === newStr) {
    const rawLines = newStr.split('\n')
    if (rawLines[rawLines.length - 1] === '') {
      rawLines.pop()
    }
    rawLines.forEach((line, index) => {
      const lineNo = (index + 1).toString()
      lines.push(createDiffLine(' ', lineNo, lineNo, line, 'context'))
    })
    return lines
  }

  const changes = Diff.diffLines(oldStr, newStr)
  let currentLineOld = 1
  let currentLineNew = 1

  changes.forEach(change => {
    const rawLines = change.value.split('\n')
    if (rawLines[rawLines.length - 1] === '') {
      rawLines.pop()
    }

    rawLines.forEach(line => {
      if (change.added) {
        lines.push(createDiffLine('+', '', currentLineNew.toString(), line, 'added'))
        currentLineNew += 1
      } else if (change.removed) {
        lines.push(createDiffLine('-', currentLineOld.toString(), '', line, 'removed'))
        currentLineOld += 1
      } else {
        const oldLine = currentLineOld.toString()
        const newLine = currentLineNew.toString()
        lines.push(createDiffLine(' ', oldLine, newLine, line, 'context'))
        currentLineOld += 1
        currentLineNew += 1
      }
    })
  })

  if (!lines.length) {
    lines.push(createDiffLine(' ', '', '', '(No visible changes)', 'context'))
  }

  return lines
})
</script>

<style lang="scss" scoped>
.file-preview-diff {
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  overflow: auto;
  background: var(--cs-bg-color);

  .diff-text {
    font-family: var(--cs-font-family-mono, monospace);
    font-size: 13px;
    line-height: 1.6;
  }

  .diff-line {
    display: grid;
    grid-template-columns: 18px 56px 56px 12px minmax(0, 1fr);
    align-items: stretch;
    min-width: 100%;

    &.added {
      background: rgba(103, 194, 58, 0.12);
    }

    &.removed {
      background: rgba(245, 108, 108, 0.12);
    }
  }

  .diff-prefix,
  .diff-line-number,
  .diff-separator {
    color: var(--cs-text-color-secondary);
    user-select: none;
    text-align: right;
    padding: 0 8px 0 0;
    border-right: 1px solid var(--cs-border-color-light, var(--cs-border-color));
  }

  .diff-prefix {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding-right: 0;
  }

  .diff-prefix::before {
    content: attr(data-prefix);
  }

  .diff-separator {
    text-align: center;
    padding-right: 0;
  }

  .diff-content {
    display: block;
    white-space: pre;
    overflow-x: auto;
    padding: 0 12px;
  }

  :deep(.hljs) {
    background: transparent;
    padding: 0;
  }
}
</style>
