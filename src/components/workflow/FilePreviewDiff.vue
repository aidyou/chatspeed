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

const LANGUAGE_MAP = {
  rs: 'rust',
  js: 'javascript',
  jsx: 'javascript',
  ts: 'typescript',
  tsx: 'typescript',
  vue: 'xml',
  py: 'python',
  php: 'php',
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

const VOID_TAGS = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'])

const language = computed(() => {
  const ext = props.filePath.split('.').pop()?.toLowerCase() || ''
  return LANGUAGE_MAP[ext] || ''
})

const highlightBlock = (content, languageName) => {
  if (!content) return ['&nbsp;']
  try {
    if (languageName) {
      return splitHighlightedHtmlByLines(hljs.highlight(content, { language: languageName }).value)
    }
    return splitHighlightedHtmlByLines(hljs.highlightAuto(content).value)
  } catch {
    return splitHighlightedHtmlByLines(hljs.highlightAuto(content).value)
  }
}

const splitHighlightedHtmlByLines = html => {
  const lines = []
  const openTags = []
  let currentLine = ''
  let index = 0

  const closeOpenTags = () => openTags.map(tag => tag.closeTag).reverse().join('')
  const reopenOpenTags = () => openTags.map(tag => tag.openTag).join('')

  while (index < html.length) {
    const char = html[index]

    if (char === '<') {
      const tagEnd = html.indexOf('>', index)
      if (tagEnd === -1) {
        currentLine += html.slice(index)
        break
      }

      const rawTag = html.slice(index, tagEnd + 1)
      currentLine += rawTag

      if (rawTag.startsWith('</')) {
        openTags.pop()
      } else {
        const tagContent = rawTag.slice(1, -1).trim()
        const tagName = tagContent.split(/\s+/)[0]?.replace(/\/$/, '').toLowerCase()
        const isSelfClosing = rawTag.endsWith('/>') || VOID_TAGS.has(tagName)
        if (!isSelfClosing && tagName) {
          openTags.push({
            openTag: rawTag,
            closeTag: `</${tagName}>`
          })
        }
      }

      index = tagEnd + 1
      continue
    }

    if (char === '\n') {
      lines.push(currentLine ? `${currentLine}${closeOpenTags()}` : '&nbsp;')
      currentLine = reopenOpenTags()
      index += 1
      continue
    }

    currentLine += char
    index += 1
  }

  lines.push(currentLine ? `${currentLine}${closeOpenTags()}` : '&nbsp;')
  return lines
}

const createDiffLine = (prefix, oldLineNumber, newLineNumber, content, html, type) => ({
  prefix,
  oldLineNumber,
  newLineNumber,
  content,
  html,
  type
})

const diffLines = computed(() => {
  const oldStr = props.oldContent ?? ''
  const newStr = props.newContent ?? ''
  const lines = []
  const highlightedOldLines = highlightBlock(oldStr, language.value)
  const highlightedNewLines = highlightBlock(newStr, language.value)

  if (oldStr === newStr) {
    const rawLines = newStr.split('\n')
    if (rawLines[rawLines.length - 1] === '') {
      rawLines.pop()
    }
    rawLines.forEach((line, index) => {
      const lineNo = (index + 1).toString()
      lines.push(createDiffLine(' ', lineNo, lineNo, line, highlightedNewLines[index] || '&nbsp;', 'context'))
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
        lines.push(
          createDiffLine(
            '+',
            '',
            currentLineNew.toString(),
            line,
            highlightedNewLines[currentLineNew - 1] || '&nbsp;',
            'added'
          )
        )
        currentLineNew += 1
      } else if (change.removed) {
        lines.push(
          createDiffLine(
            '-',
            currentLineOld.toString(),
            '',
            line,
            highlightedOldLines[currentLineOld - 1] || '&nbsp;',
            'removed'
          )
        )
        currentLineOld += 1
      } else {
        const oldLine = currentLineOld.toString()
        const newLine = currentLineNew.toString()
        lines.push(
          createDiffLine(
            ' ',
            oldLine,
            newLine,
            line,
            highlightedNewLines[currentLineNew - 1] || highlightedOldLines[currentLineOld - 1] || '&nbsp;',
            'context'
          )
        )
        currentLineOld += 1
        currentLineNew += 1
      }
    })
  })

  if (!lines.length) {
    lines.push(createDiffLine(' ', '', '', '(No visible changes)', '(No visible changes)', 'context'))
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
