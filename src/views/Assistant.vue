<template>
  <div class="assistant-page" @click="selectGroupVisible = false">
    <header class="header">
      <div class="input-container">
        <div class="icons upperLayer" v-if="canChat">
          <cs name="menu" @click.stop="selectGroupVisible = !selectGroupVisible" />
          <cs
            name="connected"
            @click="onToggleNetwork"
            :class="{ active: networkEnabled }"
            v-if="crawlerAvailable" />
        </div>
        <el-input
          class="input upperLayer"
          ref="inputRef"
          v-model="inputMessage"
          type="textarea"
          :disabled="!canChat"
          :autosize="{ minRows: 1, maxRows: 5 }"
          :placeholder="$t('assistant.chatPlaceholder')"
          @input="onInput"
          @keydown.enter="onKeyEnter"
          @compositionstart="onCompositionStart"
          @compositionend="onCompositionEnd" />
      </div>
      <div class="transaction" v-if="isTranslation">
        <el-dropdown trigger="click">
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
                {{ lang.icon }} {{ lang.name }}
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <span class="separator">→</span>
        <el-dropdown trigger="click">
          <span class="el-dropdown-link">
            {{
              toLang
                ? availableLanguages[toLang] || 'chinese'
                : $t('chat.transaction.autoDetection')
            }}
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
                {{ lang.icon }} {{ lang.name }}
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
    <main class="main" v-else>
      <div class="chat" v-if="currentAssistantMessage || isChatting">
        <div class="message">
          <div class="content-container">
            <!-- todo: use markdown component -->
            <!-- Due to the involvement of the component's scroll event, directly replacing it with the markdown component may cause bugs, so it is temporarily not handled -->
            <div class="content" ref="chatMessagesRef">
              <div class="chat-reference" v-if="chatState.reference.length > 0">
                <div
                  class="chat-reference-title"
                  :class="{ expanded: showReference }"
                  @click="showReference = !showReference">
                  <span>{{ $t('chat.reference', { count: chatState.reference.length }) }}</span>
                </div>
                <ul class="chat-reference-list" v-show="showReference" v-link>
                  <li v-for="item in chatState.reference" :key="item.id">
                    <a :href="item.url" :title="item.title.trim()">{{ item.title.trim() }}</a>
                  </li>
                </ul>
              </div>
              <div class="chat-think" v-if="chatState.reasoning != ''">
                <div
                  class="chat-think-title"
                  :class="{ expanded: showThink }"
                  @click="showThink = !showThink">
                  <span>{{
                    $t(`chat.${chatState.isReasoning ? 'reasoning' : 'reasoningProcess'}`)
                  }}</span>
                </div>
                <div
                  v-show="showThink"
                  class="think-content"
                  v-highlight
                  v-link
                  v-table
                  v-katex
                  v-html="parseMarkdown(chatState.reasoning)"></div>
              </div>
              <div
                v-html="currentAssistantMessageHtml"
                v-highlight
                v-link
                v-table
                v-katex
                v-think
                v-mermaid />
            </div>
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
    </main>
    <footer class="footer" v-if="!isChatting && currentAssistantMessage">
      <div class="metadata">
        <div class="buttons">
          <el-tooltip
            :content="$t('chat.quoteMessage')"
            :hide-after="0"
            placement="top"
            transition="none">
            <cs name="quote" @click="onReplyMessage()" class="icon-quote" />
          </el-tooltip>
          <el-tooltip
            :content="$t('chat.resendMessage')"
            :hide-after="0"
            placement="top"
            transition="none"
            v-if="userMessage">
            <cs name="resend" @click="onReAsk()" class="icon-resend" />
          </el-tooltip>
          <el-tooltip
            :content="$t('chat.goToChat')"
            :hide-after="0"
            placement="top"
            transition="none"
            v-if="userMessage">
            <cs name="skill-chat-square" @click="onGoToChat()" class="icon-chat" />
          </el-tooltip>
          <cs name="copy" @click="onCopyMessage()" class="icon-copy" />
        </div>
      </div>
    </footer>

    <!-- model selector -->
    <div class="select-group upperLayer" ref="selectGroupRef" v-if="selectGroupVisible">
      <div class="selector arrow">
        <div class="selector-content">
          <div
            class="item"
            v-for="model in providers"
            @click.stop="onModelSelect(model)"
            :key="model.id"
            :class="{ active: currentModelProvider.id === model.id }">
            <div class="name">
              <img
                :src="model.providerLogo"
                v-if="model.providerLogo !== ''"
                class="provider-logo" />
              <avatar :text="model.name" size="16" v-else />
              <span>{{ model.name }}</span>
            </div>
            <div class="icon" v-if="currentModelProvider.id === model.id">
              <cs name="check" />
            </div>
          </div>
        </div>
      </div>
      <div class="selector">
        <div class="selector-content">
          <template v-for="(models, group) in currentSubModels" :key="group">
            <div class="item group" @click.stop>
              <div class="name">
                {{ group }}
              </div>
            </div>
            <div
              class="item"
              v-for="(model, index) in models"
              @click.stop="onSubModelSelect(model.id)"
              :key="index"
              :class="{ active: currentModelProvider?.defaultModel === model.id }">
              <div class="name">
                <span>{{ model.name || model.id.split('/').pop() }}</span>
              </div>
              <div class="icon" v-if="currentModelProvider?.defaultModel === model.id">
                <cs name="check" />
              </div>
            </div>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, ref, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import SkillItem from '@/components/chat/skillItem.vue'

import { chatPreProcess, parseMarkdown } from '@/libs/chat'
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
const { t } = useI18n()

const chatStore = useChatStore()
const modelStore = useModelStore()
const skillStore = useSkillStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

// network connection and deep search
// When enabled, it will automatically crawl the URLs in user queries
const networkEnabled = ref(csGetStorage(csStorageKey.assistNetworkEnabled, true))
// When deep search is enabled, the AI will automatically plan the user's questions
// and break them down into executable steps for research.
const crawlerAvailable = computed(() => {
  return (
    settingStore.settings.chatspeedCrawler != '' &&
    settingStore.settings.chatspeedCrawler.startsWith('http')
  )
})

const selectGroupRef = ref(null)
const selectGroupVisible = ref(false)

const isAlwaysOnTop = computed(() => windowStore.assistantAlwaysOnTop)

const chatMessagesRef = ref(null)
const inputRef = ref(null)
const inputMessage = ref('')
const quoteMessage = ref('')
const composing = ref(false)
const compositionJustEnded = ref(false)
const userMessage = ref('')
const currentAssistantMessage = ref('')
const chatErrorMessage = ref('')
const isChatting = ref(false)
const lastChatId = ref()
const getDefaultChatState = () => ({
  message: '',
  reference: [],
  reasoning: '',
  isReasoning: false
})
const chatState = ref(getDefaultChatState())
const showThink = ref(true)
const showReference = ref(false)

// language config
const languageDict = languageConfig.languages
const availableLanguages = getAvailableLanguages()
const fromLang = ref('')
const toLang = ref('')

let unlistenChunkResponse = ref(null)
let unlistenPasteResponse = ref(null)

const providers = computed(() => modelStore.getAvailableProviders)
// Do not remove this, it's useful when user does not set default model at assistant dialog
const currentModelProvider = ref({ ...modelStore.defaultModelProvider })
const currentSubModels = computed(() =>
  currentModelProvider.value.models?.reduce((groups, x) => {
    if (!x.group) {
      x.group = t('settings.model.ungrouped')
    }
    groups[x.group] = groups[x.group] || []
    groups[x.group].push(x)
    return groups
  }, {})
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
  if (!currentModelProvider.value?.functionCall) {
    return false
  }
  // 1. If no skill is selected, tools can be enabled (global tools or default behavior)
  if (!selectedSkill.value) {
    return true // Or based on a global setting if you have one for non-skill scenarios
  }
  // 2. If a skill is selected, it must not be a translation skill AND its metadata must allow tools
  return !isTranslation.value && !!selectedSkill.value.metadata?.toolsEnabled
})

const cicleIndex = ref(0)
const cicle = ['◒', '◐', '◓', '◑', '☯']
const currentAssistantMessageHtml = computed(() =>
  currentAssistantMessage.value
    ? ((cicleIndex.value = (cicleIndex.value + 1) % 4),
      parseMarkdown(
        currentAssistantMessage.value + (isChatting.value ? ' ' + cicle[cicleIndex.value] : ''),
        chatState.value?.reference || []
      ))
    : isChatting.value
    ? '<div class="cs cs-loading cs-spin cs-md"></div>'
    : ''
)

/**
 * Setup scroll listener only when chatting
 */
const setupScrollListener = () => {
  chatMessagesRef.value?.addEventListener('scroll', onScroll)
}

/**
 * Remove scroll listener
 */
const removeScrollListener = () => {
  chatMessagesRef.value?.removeEventListener('scroll', onScroll)
}

watch(
  () => isChatting.value,
  newVal => {
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
watch([() => currentAssistantMessage.value, () => chatState.value.reasoning], () => {
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
      const model = currentModelProvider.value.models.find(m => m.id === defaultSubModel)
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
  unlistenChunkResponse.value = await listen('chat_stream', async event => {
    // we don't want to process messages from other windows
    if (event.payload?.metadata?.windowLabel !== settingStore.windowLabel) {
      return
    }
    // console.log('chat_stream', event)
    handleChatMessage(event.payload)
  })

  unlistenPasteResponse.value = await listen('assistant-window-paste', async event => {
    // we don't want to process messages from other windows
    if (event.payload?.windowLabel !== settingStore.windowLabel) {
      return
    }
    if (event.payload?.content) {
      inputMessage.value = event.payload.content

      await nextTick()
      const textarea = inputRef.value?.$el?.querySelector('textarea')
      if (textarea) {
        textarea.scrollTop = textarea.scrollHeight
        textarea.focus()
      }
    }
  })

  // // Listen for window show event
  // focusListener.value = await getCurrentWindow().listen('tauri://focus', async () => {
  //   await readClipboard().then(async text => {
  //     text = text.trim()
  //     if (text && !inputMessage.value) {
  //       currentAssistantMessage.value = ''
  //       inputMessage.value = text

  //       await nextTick()
  //       const textarea = inputRef.value?.$el?.querySelector('textarea')
  //       if (textarea) {
  //         textarea.scrollTop = textarea.scrollHeight
  //         textarea.focus()
  //       }
  //     }
  //   })
  // })

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

  if (chatMessagesRef.value) {
    chatMessagesRef.value.removeEventListener('scroll', onScroll)
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
  const messages = await chatPreProcess(
    inputMessage.value,
    [],
    quoteMessage.value,
    currentSkill.value,
    metadata
  )
  if (isEmpty(messages)) {
    console.log('chat messages is empty')
    return
  }
  console.log('chat messages:', messages)

  userMessage.value = inputMessage.value
  chatErrorMessage.value = ''
  quoteMessage.value = ''
  inputMessage.value = ''
  currentAssistantMessage.value = ''
  payloadMetadata.value = {}
  isChatting.value = true
  chatState.value = getDefaultChatState()
  lastChatId.value = Uuid()

  // reset scroll behavior
  resetScrollBehavior()
  nextTick(scrollToBottomIfNeeded)

  try {
    await invoke('chat_completion', {
      apiProtocol: currentModelProvider.value.apiProtocol,
      apiUrl: currentModelProvider.value.baseUrl,
      apiKey: currentModelProvider.value.apiKey,
      model: currentModelProvider.value.defaultModel,
      chatId: lastChatId.value,
      messages: messages,
      networkEnabled: networkEnabled.value,
      metadata: {
        maxTokens: currentModelProvider.value.maxTokens,
        temperature: currentModelProvider.value.temperature,
        topP: currentModelProvider.value.topP,
        topK: currentModelProvider.value.topK,
        windowLabel: settingStore.windowLabel,
        proxyType: proxyType.value,
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
const handleChatMessage = async payload => {
  let isDone = false
  chatState.value.isReasoning = payload?.type == 'reasoning'
  switch (payload?.type) {
    case 'step':
      currentAssistantMessage.value = payload?.chunk || ''
      return
    case 'reference':
      if (payload?.chunk) {
        try {
          if (typeof payload?.chunk === 'string') {
            const parsedChunk = JSON.parse(payload?.chunk || '[]')
            if (Array.isArray(parsedChunk)) {
              chatState.value.reference.push(...parsedChunk)
            } else {
              console.error('Expected an array but got:', typeof parsedChunk)
            }
          } else {
            chatState.value.reference.push(...payload?.chunk)
          }
        } catch (e) {
          console.error('error on parse reference:', e)
          console.log('chunk', payload?.chunk)
        }
      }
      break
    case 'reasoning':
      chatState.value.reasoning += payload?.chunk || ''
      break
    case 'error':
      chatErrorMessage.value = payload?.chunk || ''
      isDone = true
      break
    case 'finished':
      isDone = true
      chatState.value.message += payload?.chunk || ''
      break
    case 'text':
      chatState.value.message += payload?.chunk || ''

      // handle deepseek-r1 reasoning flag `<think></think>`
      if (
        chatState.value.message.startsWith('<think>') &&
        chatState.value.message.includes('</think>')
      ) {
        const messages = chatState.value.message.split('</think>')
        chatState.value.reasoning = messages[0].replace('<think>', '').trim()
        chatState.value.message = messages[1].trim()
      }
      break
  }

  currentAssistantMessage.value = chatState.value.message || ''
  nextTick(() => {
    scrollToBottomIfNeeded()
  })

  if (isDone) {
    isChatting.value = false
    payloadMetadata.value = {
      tokens: payload?.metadata?.tokens?.total || 0,
      prompt: payload?.metadata?.tokens?.prompt || 0,
      completion: payload?.metadata?.tokens?.completion || 0,
      provider: currentModelProvider.value.defaultModel || '',
      reference: chatState.value?.reference || [],
      reasoning: chatState.value?.reasoning || ''
    }
    nextTick(scrollToBottomIfNeeded)
  }
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
  if (!chatMessagesRef.value) return

  // 确保内容已经渲染
  nextTick(() => {
    if (!userHasScrolled.value || isScrolledToBottom.value) {
      chatMessagesRef.value.scrollTop =
        chatMessagesRef.value.scrollHeight - chatMessagesRef.value.clientHeight
    }
  })
}

/**
 * Handle scroll event of chat messages container
 */
const onScroll = () => {
  console.log('Scroll event triggered')

  if (!chatMessagesRef.value) {
    console.log('No chat messages ref')
    return
  }

  const element = chatMessagesRef.value
  const { scrollTop, scrollHeight, clientHeight } = element

  console.log('Scroll values:', { scrollTop, scrollHeight, clientHeight })

  isScrolledToBottom.value = Math.abs(scrollTop + clientHeight - scrollHeight) <= 2
  console.log('isScrolledToBottom', isScrolledToBottom.value)

  if (isScrolledToBottom.value) {
    userHasScrolled.value = false
  } else if (scrollTop < scrollHeight - clientHeight) {
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
  currentModelProvider.value = { ...model }
  csSetStorage(csStorageKey.defaultModelIdAtDialog, model.id)
}

/**
 * Handle sub model select event
 * @param {Object} model model config
 */
const onSubModelSelect = model => {
  currentModelProvider.value.defaultModel = model
  selectGroupVisible.value = false
  inputRef.value?.focus()
  csSetStorage(csStorageKey.defaultModelAtDialog, model)
}

/**
 * Handle skill item click event
 */
const onSkillItemClick = index => {
  skillIndex.value = index
  dispatchChatCompletion()
}

/**
 * Reply message
 * @param {Number} id message id
 */
const onReplyMessage = id => {
  if (!currentAssistantMessage.value?.trim()) {
    return
  }
  quoteMessage.value = currentAssistantMessage.value.trim()
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
        currentAssistantMessage.value,
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
  if (!currentAssistantMessage.value?.trim()) {
    return
  }
  try {
    navigator.clipboard.writeText(currentAssistantMessage.value.trim())
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
  if (inputMessage.value.trim()) {
    currentAssistantMessage.value = ''
  }
}

/**
 * Handle enter key event
 */
const onKeyEnter = event => {
  if (event.shiftKey) {
    return
  }
  if (!composing.value && !compositionJustEnded.value) {
    event.preventDefault()
    dispatchChatCompletion()
  }
}

/**
 * Handle keydown event
 */
const onKeydown = event => {
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    event.stopPropagation()
    skillIndex.value = (skillIndex.value + 1) % skills.value.length
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    event.stopPropagation()
    skillIndex.value = (skillIndex.value - 1 + skills.value.length) % skills.value.length
  } else if (event.key === 'Escape') {
    selectGroupVisible.value = false
  } else if (event.altKey) {
    switch (event.code) {
      case 'KeyC':
        event.preventDefault()
        event.stopPropagation()
        onCopyMessage()
        break
      case 'KeyQ':
        event.preventDefault()
        event.stopPropagation()
        if (currentAssistantMessage.value) {
          quoteMessage.value = currentAssistantMessage.value.trim()
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

    .skill-list {
      display: flex;
      flex-direction: column;
      gap: var(--cs-space-xxs);
      padding: var(--cs-space-sm) var(--cs-space-xs);
      box-sizing: border-box;

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
          .content {
            margin: var(--cs-space-sm);
            width: calc(100% - var(--cs-space-sm) * 2);
            height: unset;
            color: var(--cs-error-color);
            background-color: var(--cs-error-bg-color);
            position: relative;

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

.select-group {
  position: absolute;
  top: 45px;
  left: 10px;
  display: flex;
  flex-direction: row;
  gap: var(--cs-space-xxs);
  z-index: 10;

  .selector {
    position: relative;
    display: flex;
    flex-direction: column;
    background-color: var(--cs-bg-color);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    box-shadow: 0 2px 12px 0 var(--cs-shadow-color);

    .selector-content {
      max-width: 200px;
      min-width: 150px;
      max-height: 200px;
      overflow: auto;
      padding: var(--cs-space-xs);
    }

    &.arrow {
      &::before {
        content: '';
        position: absolute;
        top: -9px;
        left: var(--cs-space-sm);
        // left: 50%;
        // transform: translateX(-50%);
        border-width: 0 9px 9px;
        border-style: solid;
        border-color: transparent transparent var(--cs-border-color) transparent;
        pointer-events: none;
      }

      &::after {
        content: '';
        position: absolute;
        top: -8px;
        left: calc(var(--cs-space-sm) + 1px);
        // left: 50%;
        // transform: translateX(-50%);
        border-width: 0 8px 8px;
        border-style: solid;
        border-color: transparent transparent var(--cs-bg-color) transparent;
        pointer-events: none;
      }
    }

    .item {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: var(--cs-space-xs) var(--cs-space-sm);
      border-radius: var(--cs-border-radius);
      cursor: pointer;

      &:hover {
        background-color: var(--cs-bg-color-deep);
      }

      &.active {
        color: var(--cs-color-primary);
      }

      .name {
        display: flex;
        align-items: center;
        gap: var(--cs-space-xs);
        max-width: calc(100% - 24px);

        span {
          white-space: nowrap;
          text-overflow: ellipsis;
          overflow: hidden;
          font-size: var(--cs-font-size-sm);
        }

        .provider-logo {
          width: 16px;
          height: 16px;
          border-radius: 16px;
        }
      }

      .icon {
        flex-shrink: 0;
        display: flex;
      }

      &.group {
        border-bottom: 1px solid var(--cs-border-color);
        border-radius: 0;
        &:hover {
          background: none;
          cursor: default;
        }
        .name {
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-secondary);
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
</style>
