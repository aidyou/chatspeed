<template>
  <div class="assistant-page" @click="selectGroupVisible = false">
    <header class="header">
      <div class="input-container">
        <el-input
          class="input upperLayer"
          ref="inputRef"
          v-model="inputMessage"
          type="textarea"
          :disabled="!canChat"
          :autosize="{ minRows: 3, maxRows: 5 }"
          :placeholder="$t('assistant.chatPlaceholder')"
          @input="onInput"
          @keydown.enter="onKeyEnter"
          @compositionstart="onCompositionStart"
          @compositionend="onCompositionEnd" />

        <div class="icons upperLayer" v-if="canChat">
          <!-- model selector -->
          <ModelSelector
            v-model="currentModelProvider"
            position="top"
            :useProviderAvatar="true"
            :triggerSize="14"
            @model-select="onModelSelect"
            @sub-model-select="onSubModelSelect"
            @selection-complete="onSelectionComplete" />

          <!-- netowrk switch -->
          <el-tooltip
            :content="$t(`chat.${!networkEnabled ? 'networkEnabled' : 'networkDisabled'}`)"
            :hide-after="0"
            :enterable="false"
            placement="top">
            <cs name="connected" @click="onToggleNetwork" :class="{ active: networkEnabled }" />
          </el-tooltip>

          <!-- MCP switch -->
          <el-tooltip
            :content="$t(`chat.${!mcpEnabled ? 'mcpEnabled' : 'mcpDisabled'}`)"
            :hide-after="0"
            :enterable="false"
            placement="top"
            v-if="mcpServers.length > 0">
            <cs name="mcp" @click="onToggleMcp" :class="{ active: mcpEnabled }" />
          </el-tooltip>
        </div>
      </div>

      <div class="transaction" v-if="isTranslation">
        <el-dropdown trigger="click" class="transaction-dropdown">
          <span class="el-dropdown-link">
            {{
              fromLang ? languageDict[fromLang] || 'english' : $t('chat.transaction.autoDetection')
            }}
            <cs class="caret-right" />
          </span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="fromLang = ''">{{
                $t('chat.transaction.autoDetection')
              }}</el-dropdown-item>
              <el-dropdown-item
                v-for="lang in availableLanguages"
                :key="lang.code"
                @click="fromLang = lang.code">
                <span>{{ lang.icon }} {{ lang.name }}</span>
                <cs name="check" class="active" v-if="lang.code === fromLang" />
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <span class="separator">→</span>
        <el-dropdown trigger="click" class="transaction-dropdown">
          <span class="el-dropdown-link">
            {{ displayToLang }}
            <cs class="caret-right" />
          </span>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item @click="toLang = ''">{{
                $t('chat.transaction.autoDetection')
              }}</el-dropdown-item>
              <el-dropdown-item
                v-for="lang in availableLanguages"
                :key="lang.code"
                :checked="toLang === lang.code"
                @click="toLang = lang.code">
                <span>{{ lang.icon }} {{ lang.name }}</span>
                <cs name="check" class="active" v-if="lang.code === toLang" />
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
      <div class="quote" v-if="quoteMessage">
        <div class="data cs cs-quote">
          {{ quoteMessage }}
        </div>
        <div class="close-btn upperLayer" @click="quoteMessage = ''">
          <cs name="delete" />
        </div>
      </div>

      <!-- pin button -->
      <div class="pin-btn upperLayer" @click="onPin" :class="{ active: isAlwaysOnTop }">
        <el-tooltip
          :content="$t(`common.${isAlwaysOnTop ? 'autoHide' : 'pin'}`)"
          :hide-after="0"
          :enterable="false"
          placement="bottom">
          <cs name="pin" />
        </el-tooltip>
      </div>
    </header>

    <!-- empty message -->
    <main v-if="!canChat">
      <div class="empty-message">
        <div>{{ $t('assistant.haveNoModel') }}</div>
        <div class="add-model-btn" @click="onAddModel">
          {{ $t('settings.model.add') }}
        </div>
        <el-button type="primary" round @click="onOpenSettingWindow('model')">
          <cs name="add" class="small" />
          {{ $t('settings.model.add') }}
        </el-button>
      </div>
    </main>

    <!-- error message -->
    <main v-else-if="chatErrorMessage">
      <div class="chat">
        <div class="message error">
          <div class="content">
            {{ chatErrorMessage }}
            <div class="icons">
              <cs name="delete" @click="chatErrorMessage = ''" />
            </div>
          </div>
        </div>
      </div>
    </main>

    <!-- chat message -->
    <main class="main" v-else :class="{ 'split-view': chatState.message || isChatting }">
      <!-- Left Sidebar: Compact Skill List (only in split-view) -->
      <div class="skill-list-sidebar" v-if="chatState.message || isChatting">
        <el-tooltip
          v-for="(skill, index) in skills"
          :key="skill.id"
          :content="skill.name + ': ' + skill.metadata.description"
          placement="top"
          :hide-after="0"
          :enterable="false"
          :disabled="!skill.metadata.description"
          transition="none">
          <div
            class="skill-item-compact"
            :class="{ active: skillIndex === index }"
            @click="onSelectSkill(index)">
            <div class="icon">
              <cs :name="skill.icon" />
            </div>
            <!-- <span class="name">{{ skill.name }}</span> -->
          </div>
        </el-tooltip>
      </div>

      <!-- Main Content Area -->
      <div class="main-content-area">
        <div class="chat" v-if="chatState.message || isChatting">
          <div class="message">
            <div class="content-container">
              <chatting
                ref="chatMessagesRef"
                :key="lastChatId"
                :content="chatState.lastMessageChunk"
                :reference="chatState.reference"
                :reasoning="chatState.lastReasoningChunk"
                :toolCalls="chatState.toolCall || []"
                :is-reasoning="chatState.isReasoning"
                :is-chatting="isChatting" />
            </div>
          </div>
        </div>
        <div class="skill-list" v-else>
          <div
            class="skill-item"
            v-for="(skill, index) in skills"
            :key="skill.id"
            :class="{ active: skillIndex === index }"
            @click="onSkillItemClick(index)">
            <SkillItem :skill="skill" class="skill-item-content" :active="skillIndex === index" />
            <div class="icon">
              <cs name="enter" v-if="skillIndex === index" />
            </div>
          </div>
        </div>
      </div>
    </main>
    <footer class="footer" v-if="!isChatting && chatState.message">
      <div class="metadata">
        <div class="buttons">
          <el-tooltip
            :content="$t('chat.quoteMessage')"
            :hide-after="0"
            :enterable="false"
            placement="top"
            transition="none">
            <cs name="quote" @click="onReplyMessage()" class="icon-quote" />
          </el-tooltip>
          <el-tooltip
            :content="$t('chat.resendMessage')"
            :hide-after="0"
            :enterable="false"
            placement="top"
            transition="none"
            v-if="userMessage">
            <cs name="resend" @click="onReAsk()" class="icon-resend" />
          </el-tooltip>
          <el-tooltip
            :content="$t('chat.goToChat')"
            :hide-after="0"
            :enterable="false"
            placement="top"
            transition="none"
            v-if="userMessage">
            <cs name="skill-chat-square" @click="onGoToChat()" class="icon-chat" />
          </el-tooltip>
          <cs name="copy" @click="onCopyMessage()" class="icon-copy" />
        </div>
      </div>
    </footer>
  </div>
</template>

<script setup>
import { computed, nextTick, ref, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import SkillItem from '@/components/chat/SkillItem.vue'
import ChatToolCalls from '@/components/chat/ToolCall.vue'
import ModelSelector from '@/components/chat/ModelSelector.vue'

import {
  buildUserMessage,
  chatPreProcess,
  parseMarkdown,
  handleChatMessage as handleChatMessageCommon
} from '@/libs/chat'
import { csSetStorage, csGetStorage, isEmpty, showMessage, Uuid } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'
import { csStorageKey } from '@/config/config'

import { useChatStore } from '@/stores/chat'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'
import { useSkillStore } from '@/stores/skill'

import { getAvailableLanguages } from '@/i18n/langUtils'
import { languageConfig } from '@/i18n'
import { useWindowStore } from '@/stores/window'
import { useMcpStore } from '@/stores/mcp'
const { t } = useI18n()

const chatStore = useChatStore()
const modelStore = useModelStore()
const skillStore = useSkillStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()
const mcpStore = useMcpStore()

// network connection and deep search
// When enabled, it will automatically crawl the URLs in user queries
const networkEnabled = ref(csGetStorage(csStorageKey.assistNetworkEnabled, false))
// MCP enabled state
const mcpEnabled = ref(csGetStorage(csStorageKey.assistMcpEnabled, false))

const isAlwaysOnTop = computed(() => windowStore.assistantAlwaysOnTop)

const chatMessagesRef = ref(null)
const inputRef = ref(null)
const inputMessage = ref('')
const quoteMessage = ref('')
const composing = ref(false)
const compositionJustEnded = ref(false)
const userMessage = ref('')
const chatErrorMessage = ref('')
const isChatting = ref(false)
const lastChatId = ref()
const getDefaultChatState = () => ({
  message: '',
  lastMessageChunk: '',
  reference: [],
  reasoning: '',
  lastReasoningChunk: '',
  isReasoning: false,
  toolCall: []
})
const chatState = ref(getDefaultChatState())
const showThink = ref(true)
const showReference = ref(false)

// language config
const languageDict = languageConfig.languages
const availableLanguages = getAvailableLanguages()
const fromLang = ref('')
const toLang = ref('')

const displayToLang = computed(() => {
  if (!toLang.value) {
    return t('chat.transaction.autoDetection')
  }
  const lang = availableLanguages.find(l => l.code === toLang.value)
  return lang ? lang.name : t('chat.transaction.autoDetection')
})

let unlistenChunkResponse = ref(null)
let unlistenPasteResponse = ref(null)

// Do not remove this, it's useful when user does not set default model at assistant dialog
const currentModelProvider = ref({ ...modelStore.defaultModelProvider })

watch(
  () => chatErrorMessage.value,
  nv => {
    if (nv && userMessage.value) {
      inputMessage.value = userMessage.value
    }
  }
)

// Watch for changes in modelStore.providers to keep currentModelProvider in sync
watch(
  () => modelStore.providers,
  newProviders => {
    const currentId = currentModelProvider.value?.id
    if (currentId) {
      const updatedProvider = newProviders.find(p => p.id === currentId)
      if (updatedProvider) {
        // If the provider still exists, update the local ref with the latest data from the store.
        // This ensures that the `models` array within the provider is also updated.
        currentModelProvider.value = { ...updatedProvider }
      } else {
        // The selected provider was deleted. Fallback to default.
        const mid = csGetStorage(csStorageKey.defaultModelIdAtDialog)
        let model = modelStore.getModelProviderById(mid)
        if (!model && newProviders.length > 0) {
          model = modelStore.defaultModelProvider
        }
        currentModelProvider.value = model ? { ...model } : {}
      }
    } else if (newProviders.length > 0) {
      // No provider was selected, but now there are providers. Select default.
      currentModelProvider.value = { ...modelStore.defaultModelProvider }
    }
  },
  { deep: true }
)

const canChat = computed(() => modelStore.getAvailableProviders.length > 0)

const skillIndex = ref(0)
const skills = computed(() => {
  const ask = {
    id: 0,
    name: t('assistant.ask'),
    icon: 'skill-chat-square',
    metadata: { description: t('assistant.askDescription') },
    prompt: ''
  }
  return [ask, ...skillStore.availableSkills]
})
const currentSkill = computed(() => (skillIndex.value > 0 ? skills.value[skillIndex.value] : null))
/**
 * Check if the current skill is a translation skill
 */
const isTranslation = computed(() => {
  return currentSkill.value?.metadata?.type === 'translation'
})

/**
 * Check if the current skill has tools enabled
 */
const toolsEnabled = computed(() => {
  // 1. If no skill is selected, tools can be enabled (global tools or default behavior)
  if (!currentSkill.value) {
    return true // Or based on a global setting if you have one for non-skill scenarios
  }
  // 2. If a skill is selected, it must not be a translation skill AND its metadata must allow tools
  return !isTranslation.value && !!currentSkill.value.metadata?.toolsEnabled
})

// MCP servers for visibility control
const mcpServers = computed(() => mcpStore.servers)

/**
 * Setup scroll listener only when chatting
 */
const setupScrollListener = () => {
  chatMessagesRef.value?.contentRef?.addEventListener('scroll', onScroll)
}

/**
 * Remove scroll listener
 */
const removeScrollListener = () => {
  chatMessagesRef.value?.contentRef?.removeEventListener('scroll', onScroll)
}

watch(
  () => isChatting.value,
  (newVal) => {
    if (newVal) {
      // ensure scroll listener is set after the element is rendered
      nextTick(() => {
        setupScrollListener()
      })
    } else {
      removeScrollListener()
    }
  }
)

/**
 * Scroll to bottom when assistant message changes
 */
watch([() => chatState.value.message, () => chatState.value.reasoning], () => {
  nextTick(() => {
    scrollToBottomIfNeeded()
  })
})

watch(
  () => currentModelProvider.value,
  () => {
    // get default sub model from local storage
    const defaultSubModel = csGetStorage(csStorageKey.defaultModelAtDialog)
    if (defaultSubModel) {
      const model = currentModelProvider.value.models.find((m) => m.id === defaultSubModel)
      if (model) {
        currentModelProvider.value.defaultModel = defaultSubModel
      }
    }
  }
)

// =================================================
// lifecycle
// =================================================
// Window visibility state
// const focusListener = ref(null)

onMounted(async () => {
  inputRef.value?.focus()

  // set default model from local storage
  const mid = csGetStorage(csStorageKey.defaultModelIdAtDialog)
  if (mid) {
    const model = modelStore.getModelProviderById(mid)
    if (model) {
      // IMPORTANT: Do not simplify this logic!
      // If model is not found, we should keep the system default model (modelStore.defaultModelProvider)
      // instead of setting an empty object or null.
      // This ensures fallback to system default when user-defined model has been deleted.
      currentModelProvider.value = { ...model }
    }
  }

  // listen chat_stream event
  unlistenChunkResponse.value = await listen('chat_stream', async (event) => {
    // we don't want to process messages from other windows
    if (event.payload?.metadata?.windowLabel !== settingStore.windowLabel) {
      return
    }
    // console.log('chat_stream', event)
    handleChatMessage(event.payload)
  })

  unlistenPasteResponse.value = await listen('cs://assistant-paste', async (event) => {
    // we don't want to process messages from other windows
    if (event.payload?.windowLabel !== settingStore.windowLabel) {
      return
    }
    if (event.payload?.content) {
      if (isChatting.value) {
        return
      }
      inputMessage.value = event.payload.content

      nextTick(() => {
        const textarea = inputRef.value?.textarea;
        if (textarea) {
          textarea.scrollTop = textarea.scrollHeight;
        }
      });

      if (isTranslation.value) {
        setTimeout(() => {
          dispatchChatCompletion()
        }, 300)
      }
    }
  })

  windowStore.initAssistantAlwaysOnTop()
  document.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  // unlisten chunk response event
  if (unlistenChunkResponse.value) {
    unlistenChunkResponse.value()
  }

  // unlisten paste response event
  if (unlistenPasteResponse.value) {
    unlistenPasteResponse.value()
  }
  // if (focusListener.value) {
  //   focusListener.value()
  // }

  if (chatMessagesRef.value?.contentRef) {
    chatMessagesRef.value.contentRef.removeEventListener('scroll', onScroll)
  }

  document.removeEventListener('keydown', onKeydown)
  // remove scroll listener
  removeScrollListener()
})

// =================================================
// functions
// =================================================
const proxyType = computed(() => {
  return currentModelProvider.value?.metadata?.proxyType === 'bySetting'
    ? settingStore.settings.proxyType || 'none'
    : 'bySetting'
})

/**
 * Dispatch chat completion event to the backend
 */
const dispatchChatCompletion = async () => {
  if (!inputMessage.value?.trim() || !canChat.value) {
    return
  }

  const metadata = {}
  if (isTranslation.value) {
    metadata.sourceLang = fromLang.value
    metadata.targetLang = toLang.value
  }
  let quotedMessage = buildUserMessage(inputMessage.value, quoteMessage.value)
  quoteMessage.value = ''

  const messages = await chatPreProcess(quotedMessage, [], currentSkill.value, metadata)
  if (isEmpty(messages)) {
    console.log('chat messages is empty')
    return
  }

  userMessage.value = inputMessage.value
  chatErrorMessage.value = ''
  quoteMessage.value = ''
  inputMessage.value = ''
  payloadMetadata.value = {}
  isChatting.value = true
  chatState.value = getDefaultChatState()
  lastChatId.value = Uuid()

  // reset scroll behavior
  resetScrollBehavior()
  nextTick(scrollToBottomIfNeeded)

  try {
    await invoke('chat_completion', {
      providerId: currentModelProvider.value.id,
      model: currentModelProvider.value.defaultModel,
      chatId: lastChatId.value,
      messages: messages,
      networkEnabled: networkEnabled.value,
      mcpEnabled: mcpEnabled.value,
      metadata: {
        windowLabel: settingStore.windowLabel,
        toolsEnabled: toolsEnabled.value
      }
    })
  } catch (error) {
    chatErrorMessage.value = t('chat.errorOnSendMessage', { error })
    console.error('error on sendMessage:', error)
    isChatting.value = false
  }
}

const payloadMetadata = ref({})
/**
 * Handle chat message event
 */
const handleChatMessage = async (payload) => {
  // Use the common handler for shared logic
  handleChatMessageCommon(
    payload,
    chatState,
    {
      chatErrorMessage,
      isChatting
    },
    async (payload, chatStateValue) => {
      // Custom completion handler for Assistant.vue

      // If this is the end of a tool call round, clear the message
      // to prepare for the final answer, but keep the tool call data.
      if (payload.finishReason === 'toolCalls') {
        chatState.value.message = ''
      } else {
        // This is the final end of the entire turn.
        payloadMetadata.value = {
          tokens: payload?.metadata?.tokens?.total || 0,
          prompt: payload?.metadata?.tokens?.prompt || 0,
          completion: payload?.metadata?.tokens?.completion || 0,
          provider: currentModelProvider.value.defaultModel || '',
          reference: chatStateValue?.reference || [],
          reasoning: chatStateValue?.reasoning || '',
          toolCall: chatStateValue?.toolCall || []
        }
      }
      nextTick(scrollToBottomIfNeeded)
    }
  )

  // Handle scroll behavior
  nextTick(() => {
    scrollToBottomIfNeeded()
  })
}

// =================================================
// handle scroll
// =================================================
const userHasScrolled = ref(false)
const isScrolledToBottom = ref(true)
/**
 * Scroll to the bottom of the chat messages if conditions are met
 */
const scrollToBottomIfNeeded = () => {
  if (!chatMessagesRef.value?.contentRef) return

  // 确保内容已经渲染
  nextTick(() => {
    if (!userHasScrolled.value || isScrolledToBottom.value) {
      const el = chatMessagesRef.value.contentRef
      el.scrollTop = el.scrollHeight - el.clientHeight
    }
  })
}

/**
 * Handle scroll event of chat messages container
 */
const onScroll = () => {
  if (!chatMessagesRef.value?.contentRef) {
    return
  }

  const element = chatMessagesRef.value.contentRef
  const { scrollTop, scrollHeight, clientHeight } = element

  isScrolledToBottom.value = scrollTop + clientHeight >= scrollHeight - 10

  if (!isScrolledToBottom.value) {
    userHasScrolled.value = true
  }
}

/**
 * Reset scroll behavior when starting a new chat or sending a message
 */
const resetScrollBehavior = () => {
  userHasScrolled.value = false
  isScrolledToBottom.value = true
}

// =================================================
// handle events
// =================================================
/**
 * Handle pin event
 */
const onPin = async () => {
  await windowStore.toggleAssistantAlwaysOnTop()
}

/**
 * Handle model select event
 * @param {Object} model model config
 */
const onModelSelect = model => {
  // ModelSelector组件会通过v-model自动更新currentModelProvider
  // 这里只需要处理localStorage存储
  csSetStorage(csStorageKey.defaultModelIdAtDialog, model.id)
}

/**
 * Handle sub model select event
 * @param {Object} model model config
 * @param {String} modelId sub model id
 */
const onSubModelSelect = (model, modelId) => {
  // ModelSelector组件会通过v-model自动更新currentModelProvider
  // 这里只需要处理localStorage存储
  csSetStorage(csStorageKey.defaultModelAtDialog, modelId)
}

/**
 * Handle selection complete event
 */
const onSelectionComplete = () => {
  inputRef.value?.focus()
}

/**
 * Handle skill item click event
 */
const onSkillItemClick = index => {
  skillIndex.value = index
  dispatchChatCompletion()
}

/**
 * Select skill when in conversation view
 * @param {Number} index skill index
 */
const onSelectSkill = index => {
  skillIndex.value = index
  inputRef.value?.focus()
}

/**
 * Reply message
 * @param {Number} id message id
 */
const onReplyMessage = id => {
  if (!chatState.value.message?.trim()) {
    return
  }
  quoteMessage.value = chatState.value.message.trim()
  inputRef.value.focus()
}

/**
 * Re ask, copy previous user message to input
 */
const onReAsk = () => {
  if (!userMessage.value?.trim()) {
    return
  }
  inputMessage.value = userMessage.value
  inputRef.value?.focus()
}

/**
 * Add current chat to conversation and go to main window
 */
const onGoToChat = async () => {
  await chatStore.createConversation().then(async () => {
    try {
      await chatStore.addChatMessage(chatStore.currentConversationId, 'user', userMessage.value)
      await chatStore.addChatMessage(
        chatStore.currentConversationId,
        'assistant',
        chatState.value.message,
        payloadMetadata.value
      )

      // send sync state to main window
      sendSyncState('conversation_switch', 'main', {
        conversationId: chatStore.currentConversationId
      })
      // show main window
      invoke('show_window', { windowLabel: 'main' })
    } catch (error) {
      console.error('error on go to chat:', error)
      showMessage(t('chat.errorOnGoToChat', { error }), 'error', 3000)
    }
  })
}

/**
 * Copy message content
 */
const onCopyMessage = () => {
  if (!chatState.value?.message?.trim()) {
    return
  }
  try {
    navigator.clipboard.writeText(chatState.value.message.trim())
    showMessage(t('chat.messageCopied'), 'success', 1000)
  } catch (error) {
    showMessage(t('chat.errorOnCopyMessage', { error }), 'error', 3000)
  }
}

/**
 * Toggle network connection
 */
const onToggleNetwork = () => {
  networkEnabled.value = !networkEnabled.value
  csSetStorage(csStorageKey.assistNetworkEnabled, networkEnabled.value)
}

/**
 * Toggle the MCP enabled state
 */
const onToggleMcp = () => {
  mcpEnabled.value = !mcpEnabled.value
  csSetStorage(csStorageKey.mcpEnabled, mcpEnabled.value)
}

/**
 * Handle composition start event
 */
const onCompositionStart = () => {
  composing.value = true
}

/**
 * Handle composition end event
 */
const onCompositionEnd = () => {
  composing.value = false
  compositionJustEnded.value = true
  setTimeout(() => {
    compositionJustEnded.value = false
  }, 100)
}

const onInput = () => {
  // When in split-view mode (indicated by a previous userMessage),
  // typing should not reset the view to the initial skill list.
  // if (inputMessage.value.trim() && !userMessage.value) {
  //   currentAssistantMessage.value = ''
  // }
 }

/**
 * Handle enter key event
 */
const onKeyEnter = event => {
  // Determine send behavior based on user setting
  const shouldSend =
    settingStore.settings.sendMessageKey === 'Enter'
      ? !event.shiftKey // Enter to send, Shift+Enter for line break
      : event.shiftKey // Shift+Enter to send, Enter for line break

  if (shouldSend && !composing.value && !compositionJustEnded.value) {
    event.preventDefault()
    dispatchChatCompletion()
  }
}

/**
 * Handle keydown event
 */
const onKeydown = event => {
  if (event.altKey) {
    switch (event.code) {
      case 'ArrowDown':
        event.preventDefault()
        event.stopPropagation()
        skillIndex.value = (skillIndex.value + 1) % skills.value.length
        break
      case 'ArrowUp':
        event.preventDefault()
        event.stopPropagation()
        skillIndex.value = (skillIndex.value - 1 + skills.value.length) % skills.value.length
        break
      case 'KeyC':
        event.preventDefault()
        event.stopPropagation()
        onCopyMessage()
        break
      case 'KeyQ':
        event.preventDefault()
        event.stopPropagation()
        if (chatState.value.message.trim()) {
          quoteMessage.value = chatState.value.message.trim()
          inputRef.value.focus()
        }
    }
  }
}

const onAddModel = () => {
  invoke('open_setting_window', { settingType: 'model' })
}
</script>

<style lang="scss">
.app-container {
  background-color: transparent !important;
}

.el-dropdown-menu__item {
  align-items: flex-start !important;
}

.assistant-page {
  width: 100vw;
  height: 100vh;
  background-color: var(--cs-bg-color);
  border-radius: var(--cs-border-radius-md);
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;

  header {
    padding: var(--cs-space-sm) var(--cs-space-sm) var(--cs-space-sm) var(--cs-space);
    box-sizing: border-box;
    background-color: var(--cs-titlebar-bg-color);
    border-bottom: 0.5px solid var(--cs-border-color);
    box-shadow: 0 1px 2px var(--cs-shadow-color);

    .transaction {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: var(--cs-space-sm);
      color: var(--cs-text-color-secondary);
      margin-top: var(--cs-space-xs);

      .el-dropdown {
        .el-dropdown-link {
          cursor: pointer;
          background-color: var(--cs-active-bg-color);
          padding: var(--cs-space-xs) var(--cs-space-sm);
          border-radius: var(--cs-border-radius-md);
          color: var(--cs-text-color-secondary);
          font-size: var(--cs-font-size-sm);

          &:hover {
            background-color: var(--cs-hover-bg-color);
            color: var(--cs-color-primary);
          }
        }
      }
    }

    .input-container {
      display: flex;
      justify-content: space-between;
      align-items: center;
      flex-direction: row-reverse;

      .icons {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
        width: 24px;
        height: 32px;
        cursor: pointer;
        border-radius: var(--cs-border-radius);
        gap: var(--cs-space-xs);

        .cs {
          color: var(--cs-text-color-secondary);

          &:hover {
            color: var(--cs-color-primary);
          }
        }
      }

      .input > .el-textarea__inner {
        box-shadow: none !important;
        border: none !important;
        border-radius: var(--cs-border-radius-lg);
        resize: none !important;
        background-color: transparent;
      }
    }

    .quote {
      width: calc(100% - var(--cs-space-xs) * 2);
      margin: var(--cs-space-xs) var(--cs-space-xs) 0;
      padding: var(--cs-space-xxs) var(--cs-space-xs);
      background-color: var(--cs-bg-color-deep);
      border-radius: var(--cs-border-radius-md);
      display: flex;
      align-items: center;
      justify-content: space-between;
      box-sizing: border-box;

      .data {
        padding: var(--cs-space-xs) var(--cs-space-sm) var(--cs-space-xs) var(--cs-space);
        position: relative;
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
        color: var(--cs-text-color-secondary);
        font-size: var(--cs-font-size-sm);

        &::before {
          position: absolute;
          top: -1px;
          left: 0;
        }
      }

      .close-btn {
        flex-shrink: 0;
        width: 24px;
        height: 24px;
        cursor: pointer;
        color: var(--cs-text-color-secondary);

        &:hover {
          color: var(--cs-color-primary);
        }
      }
    }
  }

  main {
    flex: 1;
    min-height: 0;
    box-sizing: border-box;
    overflow: hidden;
    padding: 0;

    &.split-view {
      display: flex;
      flex-direction: row;
    }

    .skill-list-sidebar {
      width: 45px;
      flex-shrink: 0;
      border-right: 0.5px solid var(--cs-border-color);
      padding: var(--cs-space-sm) var(--cs-space-xs);
      box-sizing: border-box;
      overflow-y: auto;
      /* box-shadow: 1px 0 2px var(--cs-shadow-color); */

      .skill-item-compact {
        display: flex;
        align-items: center;
        gap: var(--cs-space-sm);
        padding: var(--cs-space-xs) var(--cs-space-sm);
        border-radius: var(--cs-border-radius);
        cursor: pointer;
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;

        .name {
          font-size: var(--cs-font-size-sm);
        }

        .icon {
          flex-shrink: 0;
        }

        &:hover,
        &.active {
          color: var(--cs-color-primary) !important;
          background-color: var(--cs-bg-color-deep);
        }
      }
    }

    .main-content-area {
      flex: 1;
      min-width: 0;
      height: 100%;
    }

    .skill-list {
      display: flex;
      flex-direction: column;
      gap: var(--cs-space-xxs);
      padding: var(--cs-space-sm) var(--cs-space-xs);
      box-sizing: border-box;
      height: 100%;

      .skill-item {
        cursor: pointer;
        padding: var(--cs-space-xs) var(--cs-space-sm);
        display: flex;
        align-items: center;
        justify-content: space-between;

        &:hover,
        &.active {
          color: var(--cs-color-primary) !important;
          background-color: var(--cs-bg-color-deep);
          border-radius: var(--cs-border-radius-md);
        }

        .skill-item-content {
          max-width: calc(100% - 24px);
        }

        .icon {
          flex-shrink: 0;
          width: 24px;
          height: 24px;
        }
      }
    }

    .chat {
      height: 100%;
      display: flex;
      flex-direction: column;

      .message {
        flex: 1;
        min-height: 0;
        overflow: hidden;

        .content-container {
          height: 100%;
          overflow: hidden;

          .content {
            height: 100%;
            overflow-y: auto;
            background-color: transparent;
          }
        }

        &.error {
          display: unset;

          .content {
            margin: var(--cs-space-sm);
            width: calc(100% - var(--cs-space-sm) * 2);
            height: unset;
            color: var(--cs-error-color);
            background-color: var(--cs-error-bg-color);
            position: relative;
            padding-top: var(--cs-space-lg);

            .icons {
              position: absolute;
              top: var(--cs-space-xs);
              right: var(--cs-space-sm);
              cursor: pointer;
            }
          }
        }
      }
    }

    .empty-message {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100%;
      width: 100%;
      padding: var(--cs-space-lg);
      box-sizing: border-box;
      gap: var(--cs-space-sm);

      .add-model-btn {
        color: var(--cs-color-primary);
        cursor: pointer;
      }
    }
  }

  footer {
    padding: var(--cs-space-xs);
    border-top: 0.5px solid var(--cs-border-color);

    .metadata {
      display: flex;
      justify-content: center;

      .buttons {
        display: flex;
        align-items: center;
        gap: var(--cs-space-sm);
      }

      .cs {
        font-size: var(--cs-font-size-lg) !important;
        color: var(--cs-text-color-secondary);
        cursor: pointer;

        &:hover {
          color: var(--cs-color-primary);
        }
      }
    }
  }
}

.pin-btn {
  cursor: pointer;
  padding: var(--cs-space-xss);
  border-radius: var(--cs-border-radius-xs);
  color: var(--cs-text-color-secondary);
  position: absolute;
  top: 0;
  right: var(--cs-space-sm);

  &:hover .cs {
    color: var(--cs-color-primary) !important;
  }

  .cs {
    font-size: var(--cs-font-size-md) !important;
    transform: rotate(45deg);
    transition: all 0.3s ease-in-out;
  }

  &.active {
    color: var(--cs-color-primary);

    .cs {
      transform: rotate(0deg);
    }
  }
}

.el-dropdown__popper.el-popper {
  .el-dropdown-menu .el-dropdown-menu__item {
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    gap: var(--cs-space-sm);
  }
}

.el-dropdown__popper .el-dropdown__list {
  max-height: 300px;
}
</style>
