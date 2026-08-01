import { computed, ref } from 'vue'

import { invokeWrapper } from '../../libs/tauri.js'
import { resolveEditorLanguage } from './codeEditorLanguages.js'

export const WORKFLOW_EDITOR_MAX_BYTES = 5 * 1024 * 1024

const noopTranslate = (key, params = {}) => {
  if (params.error) return `${key}: ${params.error}`
  return key
}

function getFileName(path = '') {
  return path.split(/[\\/]/).pop() || path
}

function toErrorText(error) {
  if (!error) return ''
  if (typeof error.toFormattedString === 'function') return error.toFormattedString()
  return error.message || String(error)
}

function createTabFromReadResult(path, result) {
  const language = resolveEditorLanguage(path)
  const content = result?.content ?? ''

  return {
    id: path,
    path,
    name: result?.name || getFileName(path),
    content,
    savedContent: content,
    dirty: false,
    loading: false,
    saving: false,
    size: Number(result?.size || 0),
    modifiedAtMs: result?.modified_at_ms ?? result?.modifiedAtMs ?? 0,
    languageId: language.id,
    languageLabel: language.label,
    error: '',
    conflict: false
  }
}

async function defaultConfirm(message, title, confirmOptions) {
  const { ElMessageBox } = await import('element-plus')
  return ElMessageBox.confirm(message, title, confirmOptions)
}

async function defaultNotify(message, type) {
  const { showMessage } = await import('../../libs/util.js')
  return showMessage(message, type)
}

async function defaultSaveError(message, title, alertOptions) {
  const { ElMessageBox } = await import('element-plus')
  return ElMessageBox.alert(message, title, alertOptions)
}

export function isEditorConflictError(error) {
  const text = toErrorText(error).toLowerCase()
  return text.includes('changed on disk')
    || text.includes('磁盘')
    || text.includes('磁碟')
    || text.includes('reload before saving')
}

export function useWorkflowCodeEditor(options = {}) {
  const t = options.t || noopTranslate
  const invoke = options.invoke || invokeWrapper
  const notify = options.notify || defaultNotify
  const confirm = options.confirm || defaultConfirm
  const saveError = options.saveError || defaultSaveError
  const usesCommandKey = options.usesCommandKey || ref(false)

  const tabs = ref([])
  const activePath = ref('')

  const hasTabs = computed(() => tabs.value.length > 0)
  const activeTab = computed(() => tabs.value.find(tab => tab.path === activePath.value) || null)
  const saveShortcutLabel = computed(() => usesCommandKey.value ? 'Cmd+S' : 'Ctrl+S')

  function findTab(path) {
    return tabs.value.find(tab => tab.path === path) || null
  }

  function setActive(path) {
    if (findTab(path)) {
      activePath.value = path
    }
  }

  async function openFile(path) {
    const existing = findTab(path)
    if (existing) {
      activePath.value = path
      return existing
    }

    try {
      const result = await invoke('read_text_file_for_editor', {
        filePath: path,
        maxBytes: WORKFLOW_EDITOR_MAX_BYTES
      })
      const tab = createTabFromReadResult(path, result)
      tabs.value.push(tab)
      activePath.value = path
      return tab
    } catch (error) {
      const message = toErrorText(error)
      const isTooLarge = message.toLowerCase().includes('too large') || message.includes('太大')
      notify(
        isTooLarge
          ? t('workflow.codeEditor.fileTooLarge', { max: '5 MiB' })
          : t('workflow.codeEditor.openFailed', { error: message }),
        isTooLarge ? 'warning' : 'error'
      )
      throw error
    }
  }

  function updateContent(path, content) {
    const tab = findTab(path)
    if (!tab) return
    tab.content = content
    tab.dirty = tab.content !== tab.savedContent
    if (tab.dirty) {
      tab.conflict = false
      tab.error = ''
    }
  }

  async function saveTab(path = activePath.value) {
    const tab = findTab(path)
    if (!tab || tab.saving) return null

    tab.saving = true
    tab.error = ''

    try {
      const result = await invoke('write_text_file_for_editor', {
        filePath: tab.path,
        content: tab.content,
        expectedModifiedAtMs: tab.modifiedAtMs,
        expectedSize: tab.size
      })

      tab.savedContent = tab.content
      tab.dirty = false
      tab.conflict = false
      tab.size = Number(result?.size || tab.content.length)
      tab.modifiedAtMs = result?.modified_at_ms ?? result?.modifiedAtMs ?? tab.modifiedAtMs
      notify(t('workflow.codeEditor.saveSuccess', { name: tab.name }), 'success')
      return tab
    } catch (error) {
      const message = toErrorText(error)
      tab.error = message
      if (isEditorConflictError(error)) {
        tab.conflict = true
        void Promise.resolve(saveError(
          t('workflow.codeEditor.externalChangeMessage', { name: tab.name }),
          t('common.warning'),
          {
            confirmButtonText: t('common.confirm'),
            type: 'warning'
          }
        )).catch(() => {})
      } else {
        void Promise.resolve(saveError(
          t('workflow.codeEditor.saveFailed', { error: message }),
          t('common.error'),
          {
            confirmButtonText: t('common.confirm'),
            type: 'error'
          }
        )).catch(() => {})
      }
      throw error
    } finally {
      tab.saving = false
    }
  }

  async function reloadTab(path = activePath.value, { force = false } = {}) {
    const tab = findTab(path)
    if (!tab) return null

    if (tab.dirty && !force) {
      await confirm(
        t('workflow.codeEditor.reloadConfirmMessage', { name: tab.name }),
        t('workflow.codeEditor.reloadConfirmTitle'),
        {
          confirmButtonText: t('common.confirm'),
          cancelButtonText: t('common.cancel'),
          type: 'warning'
        }
      )
    }

    tab.loading = true
    try {
      const result = await invoke('read_text_file_for_editor', {
        filePath: tab.path,
        maxBytes: WORKFLOW_EDITOR_MAX_BYTES
      })
      const next = createTabFromReadResult(tab.path, result)
      Object.assign(tab, next)
      activePath.value = tab.path
      return tab
    } catch (error) {
      const message = toErrorText(error)
      tab.error = message
      notify(t('workflow.codeEditor.openFailed', { error: message }), 'error')
      throw error
    } finally {
      tab.loading = false
    }
  }

  async function closeTab(path) {
    const index = tabs.value.findIndex(tab => tab.path === path)
    if (index === -1) return false

    const tab = tabs.value[index]
    if (tab.dirty) {
      await confirm(
        t('workflow.codeEditor.closeConfirmMessage', { name: tab.name }),
        t('workflow.codeEditor.closeConfirmTitle'),
        {
          confirmButtonText: t('workflow.codeEditor.closeWithoutSaving'),
          cancelButtonText: t('common.cancel'),
          type: 'warning'
        }
      )
    }

    tabs.value.splice(index, 1)
    if (activePath.value === path) {
      activePath.value = tabs.value[index]?.path || tabs.value[index - 1]?.path || ''
    }
    return true
  }

  return {
    tabs,
    activePath,
    activeTab,
    hasTabs,
    saveShortcutLabel,
    openFile,
    setActive,
    updateContent,
    saveTab,
    reloadTab,
    closeTab
  }
}
