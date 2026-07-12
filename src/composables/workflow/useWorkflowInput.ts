import { ref, computed, watch, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { invokeWrapper } from '@/libs/tauri'
import { useSettingStore } from '@/stores/setting'
import { getFileExtension } from '@/libs/fs'

const IMAGE_FILE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg'])

/**
 * Composable for managing input handling
 * Handles slash commands, file @mentions, and keyboard navigation
 */
export function useWorkflowInput({
    inputRef,
    onSendMessage: onSendMessageCallback,
    currentPaths,
    systemSkills = ref([]),
    builtinCommands = ref([]),
    onBuiltinCommandSelect = null,
    onImageFileSelect = null
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

    const filteredSystemSkills = computed(() => {
        // Only search if starts with /
        if (!inputMessage.value.startsWith('/')) return []
        const query = inputMessage.value.substring(1).toLowerCase()

        const skills = systemSkills.value.map((s) => ({
            name: s.name,
            description: s.description,
            type: 'skill',
            source: s.source,
            group: s.source === 'user' ? 'installed' : 'chatspeed'
        }))
        const commands = builtinCommands.value

        const matchedItems = [...commands, ...skills]
            .map((item) => {
                const name = item.name.toLowerCase()
                const description = (item.description || '').toLowerCase()
                return {
                    ...item,
                    matchMeta: {
                        nameExact: query.length > 0 && name === query,
                        nameStarts: query.length > 0 && name.startsWith(query),
                        nameIncludes: query.length === 0 || name.includes(query),
                        descStarts: query.length > 0 && description.startsWith(query),
                        descIncludes: query.length > 0 && description.includes(query)
                    }
                }
            })
            .filter(item => item.matchMeta.nameIncludes || item.matchMeta.descIncludes)

        const hasNameMatch = matchedItems.some(
            item => item.matchMeta.nameExact || item.matchMeta.nameStarts || item.matchMeta.nameIncludes
        )

        const filteredItems = matchedItems.filter(item => {
            if (!hasNameMatch) return item.matchMeta.descIncludes
            return item.matchMeta.nameExact || item.matchMeta.nameStarts || item.matchMeta.nameIncludes
        })

        return filteredItems
            .sort((a, b) => {
                const aName = a.name.toLowerCase()
                const bName = b.name.toLowerCase()

                // 1. Prioritize exact name match
                if (a.matchMeta.nameExact && !b.matchMeta.nameExact) return -1
                if (!a.matchMeta.nameExact && b.matchMeta.nameExact) return 1

                // 2. Prioritize "starts with" name match
                const aStarts = a.matchMeta.nameStarts
                const bStarts = b.matchMeta.nameStarts
                if (aStarts && !bStarts) return -1
                if (!aStarts && bStarts) return 1

                // 3. Prioritize "includes" name match
                const aIncludes = a.matchMeta.nameIncludes
                const bIncludes = b.matchMeta.nameIncludes
                if (aIncludes && !bIncludes) return -1
                if (!aIncludes && bIncludes) return 1

                // 4. When falling back to description matches, keep prefix hits ahead of generic includes.
                const aDescStarts = a.matchMeta.descStarts
                const bDescStarts = b.matchMeta.descStarts
                if (aDescStarts && !bDescStarts) return -1
                if (!aDescStarts && bDescStarts) return 1

                // 5. Prioritize installed skills over ChatSpeed commands/builtin skills
                if (a.group !== b.group) {
                    if (a.group === 'installed') return -1
                    if (b.group === 'installed') return 1
                }

                // 6. Fallback to alphabetical order
                return aName.localeCompare(bName)
            })
            .map(({ matchMeta, ...item }) => item)
    })

    const groupedSkillSuggestions = computed(() => {
        const groups = []
        const installedItems = filteredSystemSkills.value
            .map((item, index) => ({ ...item, originalIndex: index }))
            .filter(item => item.group === 'installed')
        const chatspeedItems = filteredSystemSkills.value
            .map((item, index) => ({ ...item, originalIndex: index }))
            .filter(item => item.group === 'chatspeed')

        if (installedItems.length > 0) {
            groups.push({
                key: 'installed',
                title: t('workflow.installedSkillsGroup'),
                items: installedItems
            })
        }

        if (chatspeedItems.length > 0) {
            groups.push({
                key: 'chatspeed',
                title: t('workflow.chatspeedSkillsGroup'),
                items: chatspeedItems
            })
        }

        return groups
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

    const onFileSelect = async (file) => {
        ignoreNextSearch.value = true
        const cursorPosition = inputRef.value?.$el.querySelector('textarea').selectionStart || 0
        const textBeforeCursor = inputMessage.value.slice(0, cursorPosition)
        const textAfterCursor = inputMessage.value.slice(cursorPosition)
        const isImageFile =
            !file.is_directory && IMAGE_FILE_EXTENSIONS.has(getFileExtension(file.path || file.relative_path || ''))

        if (isImageFile && typeof onImageFileSelect === 'function') {
            const handled = await onImageFileSelect(file)
            if (handled === true || handled === 'handled') {
                const newTextBefore = textBeforeCursor.replace(/@([^\s]*)$/, '')
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
                    setTimeout(() => {
                        ignoreNextSearch.value = false
                    }, 100)
                })
                return
            }
            if (handled === 'blocked') {
                showFileSuggestions.value = false
                selectedFileIndex.value = 0
                setTimeout(() => {
                    ignoreNextSearch.value = false
                }, 100)
                return
            }
        }

        // Smart path display: primary directory uses relative path, others use absolute path
        let displayPath: string
        const primaryRoot = currentPaths.value?.[0] // First authorized path is the primary directory

        if (file.root_path && file.root_path !== primaryRoot) {
            // Not from primary directory: show absolute path for backend to expand
            displayPath = file.path
        } else {
            // From primary directory: use simple relative path
            displayPath = file.relative_path
        }

        // Replace the @query part with @path
        const newTextBefore = textBeforeCursor.replace(/@([^\s]*)$/, `@${displayPath} `)
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
        if (skill.type === 'command' && typeof onBuiltinCommandSelect === 'function') {
            inputMessage.value = ''
            showSkillSuggestions.value = false
            selectedSkillIndex.value = 0
            onBuiltinCommandSelect(skill)
            nextTick(() => {
                if (inputRef.value) {
                    inputRef.value.focus()
                }
            })
            return
        }

        // Replace the slash command with the full skill command
        inputMessage.value = '/' + skill.name + (skill.type === 'command' ? '' : ' ')
        showSkillSuggestions.value = false
        selectedSkillIndex.value = 0

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
            if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') {
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
        if (newVal.startsWith('/')) {
            showSkillSuggestions.value = filteredSystemSkills.value.length > 0
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
        groupedSkillSuggestions,
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
