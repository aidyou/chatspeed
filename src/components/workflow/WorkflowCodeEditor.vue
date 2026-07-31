<template>
  <section class="workflow-code-editor" data-workflow-code-editor>
    <header class="code-editor-header">
      <div class="code-editor-tabs" role="tablist">
        <div
          v-for="tab in editor.tabs.value"
          :key="tab.path"
          class="code-editor-tab"
          :class="{ active: tab.path === editor.activePath.value, dirty: tab.dirty }"
        >
          <button
            class="tab-select"
            type="button"
            role="tab"
            :aria-selected="tab.path === editor.activePath.value"
            :title="tab.path"
            @click="editor.setActive(tab.path)"
          >
            <span class="dirty-dot" aria-hidden="true" />
            <span class="tab-name">{{ tab.name }}</span>
          </button>
          <button
            class="tab-close"
            type="button"
            :aria-label="t('workflow.codeEditor.closeTab', { name: tab.name })"
            @click="editor.closeTab(tab.path)"
          >
            ×
          </button>
        </div>
      </div>
      <div class="code-editor-actions" v-if="activeTab">
        <el-tooltip :content="t('workflow.codeEditor.saveShortcut', { shortcut: editor.saveShortcutLabel.value })">
          <el-button size="small" type="primary" :loading="activeTab.saving" :disabled="!activeTab.dirty" @click="editor.saveTab(activeTab.path)">
            {{ t('workflow.codeEditor.save') }}
          </el-button>
        </el-tooltip>
        <el-button size="small" :loading="activeTab.loading" @click="editor.reloadTab(activeTab.path)">
          {{ t('workflow.codeEditor.reload') }}
        </el-button>
      </div>
    </header>

    <div v-if="activeTab" class="code-editor-meta">
      <span class="code-editor-path" :title="activeTab.path">{{ activeTab.path }}</span>
      <span class="code-editor-language">{{ activeTab.languageLabel }}</span>
      <span class="code-editor-size">{{ formatBytes(activeTab.size) }}</span>
      <el-tooltip :content="t('workflow.codeEditor.tabEscapeHint')">
        <span class="code-editor-tab-hint">{{ t('workflow.codeEditor.tabHint') }}</span>
      </el-tooltip>
    </div>

    <el-alert
      v-if="activeTab?.conflict"
      class="code-editor-alert"
      type="warning"
      :closable="false"
      :title="t('workflow.codeEditor.externalChangeTitle')"
      :description="t('workflow.codeEditor.externalChangeMessage', { name: activeTab.name })"
      show-icon
    />

    <div v-if="activeTab" ref="editorHost" class="code-editor-host" />
    <div v-else class="code-editor-empty">
      {{ t('workflow.codeEditor.empty') }}
    </div>
  </section>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { EditorState } from '@codemirror/state'
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { bracketMatching, defaultHighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language'
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete'
import { lintKeymap } from '@codemirror/lint'

import { resolveEditorLanguage } from '@/composables/workflow/codeEditorLanguages.js'

const props = defineProps({
  editor: {
    type: Object,
    required: true
  }
})

const { t } = useI18n()
const editorHost = ref(null)
let view = null
let applyingExternalDocument = false

const editor = computed(() => props.editor)
const activeTab = computed(() => editor.value.activeTab.value)

const editorTheme = EditorView.theme({
  '&': {
    height: '100%',
    color: 'var(--cs-text-primary)',
    backgroundColor: 'var(--cs-bg-color)'
  },
  '.cm-scroller': {
    fontFamily: 'var(--cs-font-mono, "JetBrains Mono", "Fira Code", Consolas, monospace)',
    fontSize: '13px',
    lineHeight: '1.6'
  },
  '.cm-content': {
    overflowWrap: 'anywhere'
  },
  '.cm-gutters': {
    backgroundColor: 'var(--cs-bg-color-secondary)',
    color: 'var(--cs-text-secondary)',
    borderRight: '1px solid var(--cs-border-color)',
    position: 'sticky',
    left: '0',
    zIndex: '3'
  },
  '.cm-gutter': {
    backgroundColor: 'var(--cs-bg-color-secondary)'
  },
  '.cm-lineNumbers .cm-gutterElement': {
    minWidth: '3.2em',
    padding: '0 10px 0 8px',
    textAlign: 'right'
  },
  '.cm-activeLine, .cm-activeLineGutter': {
    backgroundColor: 'var(--cs-hover-bg)'
  },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': {
    backgroundColor: 'var(--cs-primary-light, rgba(64, 158, 255, 0.24))'
  },
  '.cm-panels': {
    borderTop: '1px solid var(--cs-border-color)',
    borderBottom: '0',
    backgroundColor: 'var(--cs-bg-color-secondary)',
    color: 'var(--cs-text-primary)'
  },
  '.cm-panels-bottom': {
    borderTop: '1px solid var(--cs-border-color)'
  },
  '.cm-search': {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '6px',
    padding: '8px 10px'
  },
  '.cm-search input': {
    height: '24px',
    border: '1px solid var(--cs-border-color)',
    borderRadius: '6px',
    padding: '0 8px',
    color: 'var(--cs-text-primary)',
    backgroundColor: 'var(--cs-bg-color)'
  },
  '.cm-search button': {
    height: '24px',
    border: '1px solid var(--cs-border-color)',
    borderRadius: '6px',
    padding: '0 8px',
    color: 'var(--cs-text-primary)',
    backgroundColor: 'var(--cs-bg-color)',
    cursor: 'pointer'
  },
  '.cm-search button:hover': {
    borderColor: 'var(--cs-primary-color)',
    color: 'var(--cs-primary-color)'
  }
})

function formatBytes(bytes = 0) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / 1024 / 1024).toFixed(2)} MiB`
}

function createEditorState(tab) {
  const language = resolveEditorLanguage(tab.path)
  const languageExtension = language.load()

  return EditorState.create({
    doc: tab.content,
    extensions: [
      lineNumbers(),
      EditorView.lineWrapping,
      highlightActiveLineGutter(),
      highlightActiveLine(),
      history(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      autocompletion(),
      highlightSelectionMatches(),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      Array.isArray(languageExtension) ? languageExtension : [languageExtension],
      keymap.of([
        {
          key: 'Mod-s',
          preventDefault: true,
          run() {
            editor.value.saveTab(tab.path).catch(() => {})
            return true
          }
        },
        indentWithTab,
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        ...lintKeymap
      ]),
      EditorView.updateListener.of(update => {
        if (!update.docChanged || applyingExternalDocument) return
        editor.value.updateContent(tab.path, update.state.doc.toString())
      }),
      editorTheme
    ]
  })
}

async function mountEditor(tab) {
  await nextTick()
  if (!editorHost.value || !tab) return

  view?.destroy()
  view = new EditorView({
    state: createEditorState(tab),
    parent: editorHost.value
  })
  view.focus()
}

watch(
  () => activeTab.value?.path,
  () => mountEditor(activeTab.value),
  { immediate: true }
)

watch(
  () => activeTab.value?.content,
  content => {
    if (!view || !activeTab.value || content === undefined) return
    const current = view.state.doc.toString()
    if (current === content) return

    applyingExternalDocument = true
    view.dispatch({ changes: { from: 0, to: current.length, insert: content } })
    applyingExternalDocument = false
  }
)

onBeforeUnmount(() => {
  view?.destroy()
  view = null
})
</script>

<style scoped lang="scss">
.workflow-code-editor {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  height: 100%;
  border-right: 1px solid var(--cs-border-color);
  background: var(--cs-bg-color);
}

.code-editor-header {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 40px;
  padding: 6px 8px;
  border-bottom: 1px solid var(--cs-border-color);
  background: var(--cs-bg-color-secondary);
}

.code-editor-tabs {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  flex: 1;
  overflow-x: auto;
}

.code-editor-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 180px;
  height: 28px;
  padding: 0 6px;
  border: 1px solid var(--cs-border-color);
  border-radius: 6px;
  background: var(--cs-bg-color);
  color: var(--cs-text-secondary);

  &.active {
    color: var(--cs-text-primary);
    border-color: var(--cs-primary-color);
  }

  &.dirty .dirty-dot {
    background: var(--cs-warning-color, #e6a23c);
  }
}

.tab-select {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.dirty-dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background: transparent;
  flex: none;
}

.tab-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tab-close {
  border: 0;
  background: transparent;
  color: var(--cs-text-secondary);
  cursor: pointer;
  padding: 0 2px;
  line-height: 1;
}

.code-editor-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.code-editor-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 30px;
  padding: 4px 10px;
  color: var(--cs-text-secondary);
  font-size: 12px;
  border-bottom: 1px solid var(--cs-border-color);
}

.code-editor-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.code-editor-language,
.code-editor-size,
.code-editor-tab-hint {
  flex: none;
}

.code-editor-alert {
  margin: 8px;
  width: auto;
}

.code-editor-host {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.code-editor-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--cs-text-secondary);
}
</style>
