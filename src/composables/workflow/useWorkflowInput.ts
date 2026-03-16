import { ref, computed, watch, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { invokeWrapper } from '@/libs/tauri'
import { useSettingStore } from '@/stores/setting'

/**
 * Composable for managing input handling
 * Handles slash commands, file @mentions, and keyboard navigation
 */
export function useWorkflowInput({
  inputRef,
  onSendMessage: onSendMessageCallback,
  currentPaths,
  systemSkills = ref([])
}) {
  const { t } = useI18n()
  const settingStore = useSettingStore()

  const inputMessage = ref('')
  const composing = ref(false)
  const compositionJustEnded = ref(false)
  const showSkillSuggestions = ref(false)
  const showFileSuggestions = ref(false)
  const selectedSkillIndex = ref(0)
  const selectedFileIndex = ref(0)
  const fileSuggestions = ref([])
  const fileQuery = ref('')
  const ignoreNextSearch = ref(false)

  // Make onSendMessage mutable so it can be updated after composable creation
  const onSendMessage = ref(onSendMessageCallback)

  const builtinCommands = [
    { name: 'settings', description: 'Open settings window' },
    { name: 'models', description: 'Open model selection window' },
    { name: 'mcp', description: 'Open MCP settings' },
    { name: 'proxy', description: 'Open proxy settings' },
    { name: 'agent', description: 'Open agent settings' },
    { name: 'about', description: 'Open about page' }
  ]

  const filteredSystemSkills = computed(() => {
    // Only search if starts with /
    if (!inputMessage.value.startsWith('/')) return []
    const query = inputMessage.value.substring(1).toLowerCase()

    const skills = systemSkills.value.map((s) => ({
      name: s.name,
      description: s.description,
      type: 'skill'
    }))
    const commands = builtinCommands.map((c) => ({
      name: c.name,
      description: c.description,
      type: 'command'
    }))

    return [...commands, ...skills]
      .filter(
        (item) =>
          item.name.toLowerCase().includes(query) ||
          (item.description && item.description.toLowerCase().includes(query))
      )
      .sort((a, b) => {
        const aName = a.name.toLowerCase()
        const bName = b.name.toLowerCase()

        // 1. Prioritize exact name match
        if (aName === query && bName !== query) return -1
        if (aName !== query && bName === query) return 1

        // 2. Prioritize "starts with" name match
        const aStarts = aName.startsWith(query)
        const bStarts = bName.startsWith(query)
        if (aStarts && !bStarts) return -1
        if (!aStarts && bStarts) return 1

        // 3. Prioritize "includes" name match
        const aIncludes = aName.includes(query)
        const bIncludes = bName.includes(query)
        if (aIncludes && !bIncludes) return -1
        if (!aIncludes && bIncludes) return 1

        // 4. Fallback to alphabetical order
        return aName.localeCompare(bName)
      })
  })

  const searchFiles = async (query) => {
    if (ignoreNextSearch.value) return
    if (!currentPaths.value || currentPaths.value.length === 0) return
    try {
      const results = await invokeWrapper('search_workspace_files', {
        paths: currentPaths.value,
        query: query
      })
      fileSuggestions.value = results || []
      showFileSuggestions.value = fileSuggestions.value.length > 0
      selectedFileIndex.value = 0
    } catch (error) {
      console.error('Failed to search files:', error)
    }
  }

  const onFileSelect = (file) => {
    ignoreNextSearch.value = true
    const cursorPosition = inputRef.value?.$el.querySelector('textarea').selectionStart || 0
    const textBeforeCursor = inputMessage.value.slice(0, cursorPosition)
    const textAfterCursor = inputMessage.value.slice(cursorPosition)

    // Replace the @query part with @path
    const newTextBefore = textBeforeCursor.replace(/@([^\s]*)$/, `@${file.relative_path} `)
    inputMessage.value = newTextBefore + textAfterCursor

    showFileSuggestions.value = false
    selectedFileIndex.value = 0

    nextTick(() => {
      if (inputRef.value) {
        inputRef.value.focus()
        const newPos = newTextBefore.length
        const textarea = inputRef.value?.$el.querySelector('textarea')
        if (textarea) {
          textarea.setSelectionRange(newPos, newPos)
        }
      }
      // Allow search again after UI has updated
      setTimeout(() => {
        ignoreNextSearch.value = false
      }, 100)
    })
  }

  const onSkillSelect = (skill) => {
    // Replace the slash command with the full skill command
    inputMessage.value = '/' + skill.name + (skill.type === 'command' ? '' : ' ')
    showSkillSuggestions.value = false
    selectedSkillIndex.value = 0

    // For commands (UI action), we focus immediately and let the caller decide to send
    // For skills (AI logic), we focus and let user add more details
    nextTick(() => {
      if (inputRef.value) {
        inputRef.value.focus()
      }
    })
  }

  const onCompositionStart = () => {
    composing.value = true
  }

  const onCompositionEnd = () => {
    composing.value = false
    compositionJustEnded.value = true
    setTimeout(() => {
      compositionJustEnded.value = false
    }, 100)
  }

  const onInputKeyDown = (event) => {
    if (composing.value || compositionJustEnded.value) return

    // Handle Slash Command Suggestions
    if (showSkillSuggestions.value) {
      if (event.key === 'Enter' || event.key === 'Tab') {
        event.preventDefault()
        if (filteredSystemSkills.value.length > 0) {
          onSkillSelect(filteredSystemSkills.value[selectedSkillIndex.value])
        } else {
          showSkillSuggestions.value = false
        }
        return
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        selectedSkillIndex.value =
          (selectedSkillIndex.value - 1 + filteredSystemSkills.value.length) %
          filteredSystemSkills.value.length
        return
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault()
        selectedSkillIndex.value =
          (selectedSkillIndex.value + 1) % filteredSystemSkills.value.length
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        showSkillSuggestions.value = false
        return
      }
    }

    // Handle File At-mention Suggestions
    if (showFileSuggestions.value) {
      if (event.key === 'Enter' || event.key === 'Tab') {
        event.preventDefault()
        if (fileSuggestions.value.length > 0) {
          onFileSelect(fileSuggestions.value[selectedFileIndex.value])
        } else {
          showFileSuggestions.value = false
        }
        return
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        selectedFileIndex.value =
          (selectedFileIndex.value - 1 + fileSuggestions.value.length) %
          fileSuggestions.value.length
        return
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault()
        selectedFileIndex.value = (selectedFileIndex.value + 1) % fileSuggestions.value.length
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        showFileSuggestions.value = false
        return
      }
    }

    // Handle Enter key for sending message based on user settings
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && onSendMessage.value) {
      const shouldSend =
        settingStore.settings.sendMessageKey === 'Enter'
          ? !event.shiftKey // Enter to send, Shift+Enter for line break
          : event.shiftKey // Shift+Enter to send, Enter for line break

      if (shouldSend) {
        event.preventDefault()
        onSendMessage.value()
        return
      }
    }
  }

  // Watch for input changes to trigger suggestions
  watch(inputMessage, (newVal) => {
    // TRIGGERS ONLY if '/' is the very first character of the whole input
    if (newVal === '/') {
      showSkillSuggestions.value = systemSkills.value.length > 0
      selectedSkillIndex.value = 0
    } else if (!newVal.startsWith('/') || newVal === '') {
      showSkillSuggestions.value = false
    }

    // At-mention detection
    if (inputRef.value) {
      const cursorPosition = inputRef.value?.$el.querySelector('textarea').selectionStart || 0
      const textBeforeCursor = newVal.slice(0, cursorPosition)
      const match = textBeforeCursor.match(/@([^\s]*)$/)
      if (match) {
        searchFiles(match[1])
      } else {
        showFileSuggestions.value = false
      }
    }
  })

  const clearInput = () => {
    inputMessage.value = ''
    showSkillSuggestions.value = false
    showFileSuggestions.value = false
  }

  return {
    inputMessage,
    composing,
    showSkillSuggestions,
    showFileSuggestions,
    selectedSkillIndex,
    selectedFileIndex,
    fileSuggestions,
    filteredSystemSkills,
    onInputKeyDown,
    onCompositionStart,
    onCompositionEnd,
    onSkillSelect,
    onFileSelect,
    searchFiles,
    clearInput,
    onSendMessage
  }
}
