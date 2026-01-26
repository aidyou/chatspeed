<template>
  <div class="assistant-page" @click="selectGroupVisible = false">
    <header class="header">
      <div class="input-container">
        <!-- Attachments area -->
        <div class="attachments-area" v-if="attachments.length > 0">
          <div v-for="attachment in attachments" :key="attachment.id" class="attachment-item upperLayer">
            <img v-if="attachment.type === 'image'" :src="attachment.url" class="attachment-preview" />
            <cs v-else name="file" class="attachment-icon" />
            <span class="attachment-name">{{ attachment.name }}</span>
            <cs name="close" class="attachment-remove" @click="removeAttachment(attachment.id)" />
          </div>
        </div>

        <el-input class="input upperLayer" ref="inputRef" v-model="inputMessage" type="textarea" :disabled="!canChat"
          :autosize="{ minRows: 3, maxRows: 5 }" :placeholder="$t('assistant.chatPlaceholder')" @input="onInput"
          @keydown.enter="onKeyEnter" @compositionstart="onCompositionStart" @compositionend="onCompositionEnd"
          @paste="onPaste" />

        <div class="icons upperLayer" v-if="canChat">
          <!-- model selector -->
          <ModelSelector v-model="currentModelProvider" position="top" :useProviderAvatar="true" :triggerSize="14"
            @model-select="onModelSelect" @sub-model-select="onSubModelSelect"
            @selection-complete="onSelectionComplete" />

          <!-- attachment button -->
          <el-tooltip :content="$t('chat.addAttachment')" :hide-after="0" :enterable="false" placement="right">
            <cs name="attachment" @click="onOpenFileDialog" />
          </el-tooltip>

          <!-- sensitive filtering switch -->
          <el-tooltip :content="$t('chat.sensitiveFiltering')" :hide-after="0" :enterable="false" placement="right">
            <cs name="filter" @click="onToggleSensitiveFiltering" :class="{ active: sensitiveStore.config.enabled }" />
          </el-tooltip>

          <!-- netowrk switch -->
          <el-tooltip :content="$t(`chat.${!networkEnabled ? 'networkEnabled' : 'networkDisabled'}`)" :hide-after="0"
            :enterable="false" placement="right">
            <cs name="connected" @click="onToggleNetwork" :class="{ active: networkEnabled }" />
          </el-tooltip>

          <!-- MCP switch -->
          <el-tooltip :content="$t(`chat.${!mcpEnabled ? 'mcpEnabled' : 'mcpDisabled'}`)" :hide-after="0"
            :enterable="false" placement="right" v-if="mcpServers.length > 0">
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
              <el-dropdown-item v-for="lang in availableLanguages" :key="lang.code" @click="fromLang = lang.code">
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
              <el-dropdown-item v-for="lang in availableLanguages" :key="lang.code" :checked="toLang === lang.code"
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
        <el-tooltip :content="$t(`common.${isAlwaysOnTop ? 'autoHide' : 'pin'}`)" :hide-after="0" :enterable="false"
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
        <el-tooltip v-for="(skill, index) in skills" :key="skill.id"
          :content="skill.name + ': ' + skill.metadata.description" placement="top" :hide-after="0" :enterable="false"
          :disabled="!skill.metadata.description" transition="none">
          <div class="skill-item-compact" :class="{ active: skillIndex === index }" @click="onSelectSkill(index)">
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
              <chatting ref="chatMessagesRef" :key="lastChatId" :step="chatState.step"
                :content="chatState.lastMessageChunk" :reference="chatState.reference"
                :reasoning="chatState.lastReasoningChunk" :toolCalls="chatState.toolCall || []"
                :is-reasoning="chatState.isReasoning" :is-chatting="isChatting" />
            </div>
          </div>
        </div>
        <div class="skill-list" v-else>
          <div class="skill-item" v-for="(skill, index) in skills" :key="skill.id"
            :class="{ active: skillIndex === index }" @click="onSkillItemClick(index)">
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
          <el-tooltip :content="$t('chat.quoteMessage')" :hide-after="0" :enterable="false" placement="top"
            transition="none">
            <cs name="quote" @click="onReplyMessage()" class="icon-quote" />
          </el-tooltip>
          <el-tooltip :content="$t('chat.resendMessage')" :hide-after="0" :enterable="false" placement="top"
            transition="none" v-if="userMessage">
            <cs name="resend" @click="onReAsk()" class="icon-resend" />
          </el-tooltip>
          <el-tooltip :content="$t('chat.goToChat')" :hide-after="0" :enterable="false" placement="top"
            transition="none" v-if="userMessage">
            <cs name="skill-chat-square" @click="onGoToChat()" class="icon-chat" />
          </el-tooltip>
          <cs name="copy" @click="onCopyMessage()" class="icon-copy" />
        </div>
      </div>
    </footer>
  </div>

  <!-- File selection dialog -->

  <el-dialog v-model="fileDialogVisible" :title="$t('chat.selectFile')" width="500px">

    <el-upload drag :auto-upload="false" :on-change="onFileSelect" :show-file-list="false"
      accept="image/*,.txt,.md,.json,.xml,.csv,.log,.php,.go,.rs,.js,.py,.ts,.css,.html,.htm,.pdf,.docx,.xlsx,.xls">

      <div class="upload-area">

        <cs name="upload" size="48px" />

        <div class="upload-text">{{ $t('chat.dragOrClickToSelectFile') }}</div>

        <div class="upload-hint">

          {{ $t('chat.supportedImageFormats') }}<br />

          {{ $t('chat.supportedOfficeFormats') }}<br />

          {{ $t('chat.supportedTextFormats') }}

        </div>

      </div>

    </el-upload>

  </el-dialog>
</template>

<script setup>
import { computed, nextTick, ref, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { invokeWrapper, FrontendAppError } from '@/libs/tauri'
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
import { parseFileContent } from '@/libs/file-parser'
import { sendSyncState } from '@/libs/sync'
import { csStorageKey } from '@/config/config'

import { useChatStore } from '@/stores/chat'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'
import { useSensitiveStore } from '@/stores/sensitiveStore'
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
const sensitiveStore = useSensitiveStore()
const windowStore = useWindowStore()
const mcpStore = useMcpStore()

// network connection and deep search
// When enabled, it will automatically crawl the URLs in user queries
const networkEnabled = ref(csGetStorage(csStorageKey.assistNetworkEnabled, false))
// MCP enabled state
const mcpEnabled = ref(csGetStorage(csStorageKey.assistMcpEnabled, false))

/**
 * Toggle sensitive information filtering
 */
const onToggleSensitiveFiltering = () => {
  sensitiveStore.config.enabled = !sensitiveStore.config.enabled
  sensitiveStore.saveConfig()
}

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

// Attachments
const attachments = ref([])
const fileDialogVisible = ref(false)
const getDefaultChatState = () => ({
  step: '',
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
let unlistenSyncState = ref(null)

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
watch([() => chatState.value.message, () => chatState.value.reasoning, () => chatState.value.step], () => {
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
  await sensitiveStore.fetchConfig()

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

    // Skip vision-related messages in the global UI listener
    if (event.payload?.chatId === lastChatId.value) {
      handleChatMessage(event.payload)
    }
  })

  unlistenPasteResponse.value = await listen('cs://assistant-paste', async event => {
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
        const textarea = inputRef.value?.textarea
        if (textarea) {
          textarea.scrollTop = textarea.scrollHeight
        }
      })

      if (isTranslation.value) {
        setTimeout(() => {
          dispatchChatCompletion()
        }, 300)
      }
    }
  })

  // listen sync state event
  unlistenSyncState.value = await listen('cs://sync-state', event => {
    if (event?.payload?.type === 'sensitive_config_changed') {
      sensitiveStore.config = { ...event.payload.metadata }
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

  // unlisten sync state event
  if (unlistenSyncState.value) {
    unlistenSyncState.value()
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
  if ((!inputMessage.value?.trim() && attachments.value.length === 0) || !canChat.value) {
    return
  }

  // 1. 备份数据
  const backupMessage = inputMessage.value
  const backupAttachments = [...attachments.value]
  const rawUserMessage = inputMessage.value.trim()

  let processedAttachmentMetadata = null
  let visionAnalysisResult = ''

  // 2. 立即初始化状态并清空输入
  inputMessage.value = ''
  attachments.value = []
  chatErrorMessage.value = ''
  isChatting.value = true
  chatState.value = getDefaultChatState()

  nextTick(() => {
    resetScrollBehavior()
    scrollToBottomIfNeeded()
  })

  // Handle attachments
  if (backupAttachments.length > 0) {
    chatState.value.step = t('chat.preparingAttachments')

    // Prepare attachment metadata for storage
    processedAttachmentMetadata = {
      attachments: backupAttachments.map(a => ({
        type: a.type,
        name: a.name,
        size: a.size,
        url: a.url || null,
        content: a.content || null
      }))
    }

    const hasImageAttachments = backupAttachments.some(a => a.type === 'image')
    const visionModel = settingStore.settings.visionModel

    if (hasImageAttachments) {
      if (!visionModel.id || !visionModel.model) {
        // Rollback
        inputMessage.value = backupMessage
        attachments.value = backupAttachments
        isChatting.value = false
        showMessage(t('settings.general.visionModelRequired'), 'error')
        return
      }

      chatState.value.step = t('chat.analyzingImages')

      const imageAttachments = backupAttachments.filter(a => a.type === 'image')
      const textAttachments = backupAttachments.filter(a => a.type === 'text')

      const visionMessage = {
        role: 'user',
        content: [{ type: 'text', text: 'Please describe all the images in detail.' }]
      }

      for (const attachment of imageAttachments) {
        visionMessage.content.push({ type: 'image_url', image_url: { url: attachment.url } })
      }

      if (textAttachments.length > 0) {
        const textContent = textAttachments
          .map(a => `[File: ${a.name}]:\n${a.content}`)
          .join('\n\n')
        visionMessage.content.push({ type: 'text', text: textContent })
      }

      try {
        const visionLastChatId = Uuid()
        const visionChatId = `vision_${visionLastChatId}`

        const visionPromise = new Promise((resolve, reject) => {
          let fullContent = ''
          let finished = false
          let unlistenFn = null

          listen('chat_stream', (event) => {
            const payload = event.payload
            if (payload.chatId === visionChatId) {
              if (payload.type === 'text' && payload.chunk) {
                fullContent += payload.chunk
              } else if (payload.type === 'finished') {
                finished = true
                resolve(fullContent)
              } else if (payload.type === 'error') {
                finished = true
                reject(new Error(payload.chunk || 'Vision failed'))
              }
            }
          }).then(fn => unlistenFn = fn)

          setTimeout(() => {
            if (!finished) {
              if (unlistenFn) unlistenFn()
              reject(new Error('Vision timeout'))
            }
          }, 60000)
        })

        await invokeWrapper('chat_completion', {
          providerId: visionModel.id,
          model: visionModel.model,
          chatId: visionChatId,
          messages: [visionMessage],
          networkEnabled: false,
          mcpEnabled: false,
          stream: false,
          toolsEnabled: false,
          metadata: {}
        })

        visionAnalysisResult = await visionPromise
      } catch (error) {
        console.error('Error analyzing images:', error)
        // 回退
        inputMessage.value = backupMessage
        attachments.value = backupAttachments
        chatErrorMessage.value = t('chat.errorOnAddAttachment', { error: error.message })
        isChatting.value = false
        return
      }
    } else {
      const textAttachments = backupAttachments.filter(a => a.type === 'text')
      textAttachments.forEach(attachment => {
        visionAnalysisResult += `\n\n[File: ${attachment.name}]:\n${attachment.content}`
      })
    }
  }

  // Construct final message
  let finalMessageToSend = rawUserMessage
  if (visionAnalysisResult) {
    finalMessageToSend = `[Image Analysis]:\n${visionAnalysisResult}\n\n[User Question]: ${rawUserMessage}`
  }

  if (!finalMessageToSend) {
    isChatting.value = false
    return
  }

  const metadata = {
    vision_analysis: visionAnalysisResult
  }

  if (isTranslation.value) {
    metadata.sourceLang = fromLang.value
    metadata.targetLang = toLang.value
  }

  if (processedAttachmentMetadata?.attachments?.length > 0) {
    metadata.attachments = processedAttachmentMetadata.attachments
  }

  let quotedMessage = buildUserMessage(finalMessageToSend, quoteMessage.value)
  quoteMessage.value = ''

  const messages = await chatPreProcess(quotedMessage, [], currentSkill.value, metadata)
  if (isEmpty(messages)) {
    isChatting.value = false
    return
  }

  chatState.value.step = t('chat.generatingResponse')
  userMessage.value = rawUserMessage
  payloadMetadata.value = {}
  lastChatId.value = Uuid()

  // reset scroll behavior
  resetScrollBehavior()
  nextTick(scrollToBottomIfNeeded)

  try {
    await invokeWrapper('chat_completion', {
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
    chatErrorMessage.value = t('chat.errorOnSendMessage', { error: String(error) })
    isChatting.value = false
  }
}
const payloadMetadata = ref({})
/**
 * Handle chat message event
 */
const handleChatMessage = async payload => {
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
      invokeWrapper('show_window', { windowLabel: 'main' })
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`error on go to chat: ` + error.toFormattedString(), error.originalError)
        showMessage(t('chat.errorOnGoToChat', { error: error.toFormattedString() }), 'error', 3000)
      } else {
        console.error('error on go to chat:', error)
        showMessage(
          t('chat.errorOnGoToChat', { error: error.message || String(error) }),
          'error',
          3000
        )
      }
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

// =================================================
// Attachment handling
// =================================================

/**
 * Generate unique ID for attachment
 */
const generateAttachmentId = () => {
  return `attachment_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`
}

/**
 * Add attachment
 * @param {Object} attachment - Attachment object
 */
const addAttachment = attachment => {
  attachments.value.push({
    id: generateAttachmentId(),
    ...attachment
  })
}

/**
 * Remove attachment
 * @param {String} id - Attachment ID
 */
const removeAttachment = id => {
  const index = attachments.value.findIndex(a => a.id === id)
  if (index > -1) {
    attachments.value.splice(index, 1)
  }
}

/**
 * Clear all attachments
 */
const clearAttachments = () => {
  attachments.value = []
}

/**
 * Handle paste event for images
 * @param {ClipboardEvent} event - Paste event
 */
const onPaste = async event => {
  const items = event.clipboardData?.items
  if (!items) return

  let hasImage = false
  for (let i = 0; i < items.length; i++) {
    const item = items[i]
    if (item.type.startsWith('image/')) {
      hasImage = true
      const file = item.getAsFile()
      if (file) {
        await handleImageFile(file)
      }
    }
  }

  if (hasImage) {
    event.preventDefault()
  }
}

/**
 * Handle image file
 * @param {File} file - Image file
 */
const handleImageFile = async file => {
  try {
    // Element Plus wraps the file, so we need to get the raw file
    const rawFile = file.raw || file

    return new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = (e) => {
        addAttachment({
          type: 'image',
          name: rawFile.name,
          url: e.target.result,
          size: rawFile.size
        })
        resolve()
      }
      reader.onerror = (error) => {
        console.error('FileReader error:', error)
        showMessage(t('chat.errorOnAddAttachment', { error: 'Failed to read image' }), 'error')
        reject(error)
      }
      reader.readAsDataURL(rawFile)
    })
  } catch (error) {
    console.error('Error handling image file:', error)
    showMessage(t('chat.errorOnAddAttachment', { error: error.message }), 'error')
  }
}

/**
 * Handle text file
 * @param {File} file - Text file
 */
const handleTextFile = async file => {
  try {
    const rawFile = file.raw || file

    return new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = (e) => {
        addAttachment({
          type: 'text',
          name: rawFile.name,
          content: e.target.result,
          size: rawFile.size
        })
        resolve()
      }
      reader.onerror = (error) => {
        console.error('FileReader error:', error)
        showMessage(t('chat.errorOnAddAttachment', { error: 'Failed to read text file' }), 'error')
        reject(error)
      }
      reader.readAsText(rawFile)
    })
  } catch (error) {
    console.error('Error handling text file:', error)
    showMessage(t('chat.errorOnAddAttachment', { error: error.message || 'Failed to read file' }), 'error')
  }
}

const originalAlwaysOnTopState = ref(false)
const isTemporarilyPinned = ref(false)

/**
 * Open file dialog and temporarily pin window if needed
 */
const onOpenFileDialog = async () => {
  originalAlwaysOnTopState.value = windowStore.assistantAlwaysOnTop

  if (!originalAlwaysOnTopState.value) {
    // Temporarily pin the window
    try {
      await invokeWrapper('toggle_window_always_on_top', {
        windowLabel: 'assistant',
        newState: true
      })
      windowStore.assistantAlwaysOnTop = true
      isTemporarilyPinned.value = true
    } catch (e) {
      console.error('Failed to temporarily pin window:', e)
    }
  }

  fileDialogVisible.value = true
}

/**
 * Restore window pin state
 */
const restorePinState = async () => {
  if (isTemporarilyPinned.value) {
    try {
      await invokeWrapper('toggle_window_always_on_top', {
        windowLabel: 'assistant',
        newState: originalAlwaysOnTopState.value
      })
      windowStore.assistantAlwaysOnTop = originalAlwaysOnTopState.value
    } catch (e) {
      console.error('Failed to restore window pin state:', e)
    } finally {
      isTemporarilyPinned.value = false
    }
  }
}

// Watch for dialog close (e.g. by clicking overlay) to restore pin state
watch(fileDialogVisible, (val) => {
  if (!val) {
    restorePinState()
  }
})

/**
 * Handle file selection
 * @param {File} file - Selected file
 */
const onFileSelect = async file => {
  // Element Plus wraps the file, so we need to get the raw file
  const rawFile = file.raw || file

  const imageTypes = ['image/jpeg', 'image/png', 'image/gif', 'image/webp', 'image/svg+xml', 'image/bmp']
  const imageExtensions = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.svg', '.bmp']
  const textExtensions = ['.txt', '.md', '.json', '.xml', '.csv', '.log', '.php', '.go', '.rs', '.js', '.py', '.ts', '.css', '.html', '.htm', '.pdf', '.docx', '.xlsx', '.xls']

  const fileName = rawFile.name.toLowerCase()

  // Hide dialog and restore pin state
  fileDialogVisible.value = false

  // Check if it's an image by MIME type or extension
  if (imageTypes.includes(rawFile.type) || imageExtensions.some(ext => fileName.endsWith(ext))) {
    handleImageFile(file)
  } else if (textExtensions.some(ext => fileName.endsWith(ext))) {
    try {
      const content = await parseFileContent(rawFile)
      if (content || content === "") {
        addAttachment({
          type: 'text',
          name: rawFile.name,
          content: content,
          size: rawFile.size
        })
      }
    } catch (error) {
      console.error('Error parsing file:', error)
      showMessage(t('chat.errorOnAddAttachment', { error: error.message || 'Parse failed' }), 'error')
    }
  } else {
    showMessage(t('chat.unsupportedFileType'), 'error')
  }
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

const onAddModel = async () => {
  try {
    await invokeWrapper('open_setting_window', { settingType: 'model' })
  } catch (error) {
    if (error instanceof FrontendAppError) {
      console.error(
        `Error opening setting window: ` + error.toFormattedString(),
        error.originalError
      )
      showMessage(
        t('chat.errorOnOpenSettingWindow', {
          error: error.toFormattedString()
        }),
        'error',
        3000
      )
    } else {
      console.error('Error opening setting window:', error)
      showMessage(
        t('chat.errorOnOpenSettingWindow', { error: error.message || String(error) }),
        'error',
        3000
      )
    }
  }
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
      flex-direction: column;
      align-items: stretch;
      width: 100%;

      .icons {
        display: flex;
        flex-direction: row;
        align-items: center;
        justify-content: flex-start;
        flex-shrink: 0;
        height: auto;
        padding: var(--cs-space-xs) 0 0;
        cursor: pointer;
        border-radius: var(--cs-border-radius);
        gap: var(--cs-space-sm);

        .cs {
          color: var(--cs-text-color-secondary);
          font-size: var(--cs-font-size-md);

          &:hover {
            color: var(--cs-color-primary);
          }
        }
      }

      .input>.el-textarea__inner {
        box-shadow: none !important;
        border: none !important;
        border-radius: var(--cs-border-radius-sm);
        resize: none !important;
        background-color: transparent;
        padding: 0;
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

// =================================================
// Attachments
// =================================================

.attachments-area {
  display: flex;
  flex-wrap: wrap;
  gap: var(--cs-space-sm);
  padding: var(--cs-space-sm);
  background: var(--cs-bg-color-light);
  border-radius: var(--cs-border-radius-sm);
  margin-bottom: var(--cs-space-sm);
}

.attachment-item {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  padding: var(--cs-space-xs) var(--cs-space-sm);
  background: var(--cs-bg-color);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-sm);
  font-size: var(--cs-font-size-xs);
}

.attachment-preview {
  width: 32px;
  height: 32px;
  object-fit: cover;
  border-radius: var(--cs-border-radius-xs);
}

.attachment-icon {
  font-size: 16px;
  color: var(--cs-text-color-secondary);
}

.attachment-name {
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--cs-text-color-secondary);
}

.attachment-remove {
  cursor: pointer;
  color: var(--cs-text-color-secondary);
  font-size: 12px;

  &:hover {
    color: var(--cs-color-danger);
  }
}

.upload-area {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--cs-space-md);
  padding: var(--cs-space-xl);
}

.upload-text {
  font-size: var(--cs-font-size-base);
  color: var(--cs-text-color);
}

.upload-hint {
  font-size: var(--cs-font-size-xs);
  color: var(--cs-text-color-secondary);
  text-align: center;
  line-height: 1.6;
}

.el-dropdown__popper .el-dropdown__list {
  max-height: 300px;
}
</style>
