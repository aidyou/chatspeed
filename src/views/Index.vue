<template>
  <div class="chat">
    <el-container class="chat-container">
      <!-- header -->
      <titlebar :show-menu-button="settingStore.settings.showMenuButton">
        <template #left>
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
          <div
            class="icon-btn upperLayer pin-btn"
            @click="onPin"
            :class="{ active: mainWindowIsAlwaysOnTop }">
            <el-tooltip
              :content="$t(`common.${mainWindowIsAlwaysOnTop ? 'unpin' : 'pin'}`)"
              :hide-after="0"
              placement="bottom">
              <cs name="pin" />
            </el-tooltip>
          </div>
        </template>
        <template #center>
          <div class="model-selector" v-if="modelProviders.length > 0">
            <el-dropdown @command="onModelChange" trigger="click">
              <div class="dropdown-text upperLayer">
                <span class="text">
                  <img
                    :src="currentModel?.providerLogo"
                    v-if="currentModel?.providerLogo !== ''"
                    class="provider-logo-sm" />
                  <avatar :text="currentModel?.name" size="12px" v-else />
                  {{ currentModel?.name }}
                </span>
                <cs name="caret-down" />
              </div>
              <template #dropdown>
                <el-dropdown-menu class="dropdown">
                  <el-dropdown-item
                    v-for="model in modelProviders"
                    :key="model.id"
                    :command="model.id"
                    :class="{ 'is-active': currentModel.id === model.id }">
                    <div class="item">
                      <div class="name">
                        <img
                          :src="model.providerLogo"
                          v-if="model.providerLogo !== ''"
                          class="provider-logo" />
                        <avatar :text="model.name" size="16px" v-else />
                        {{ model.name }}
                      </div>
                    </div>
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
            <el-dropdown @command="onSubModelChange" trigger="click">
              <div class="dropdown-text upperLayer">
                <span class="text">{{ currentModelAlias }}</span>
                <cs name="caret-down" />
              </div>
              <template #dropdown>
                <el-dropdown-menu class="dropdown">
                  <template v-for="(models, group) in currentSubModels" :key="group">
                    <el-dropdown-item disabled>
                      <div class="item">
                        <div class="name">
                          {{ group }}
                        </div>
                      </div>
                    </el-dropdown-item>
                    <el-dropdown-item
                      v-for="(model, index) in models"
                      :divided="index == 0"
                      :key="index"
                      :command="model.id"
                      :class="{ 'is-active': currentModel?.defaultModel === model.id }">
                      <div class="item">
                        <div class="name">
                          {{ model.name || model.id.split('/').pop() }}
                        </div>
                      </div>
                    </el-dropdown-item>
                  </template>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>
          <div class="model-selector" v-else>
            <div class="icon-btn dropdown-text upperLayer" @click="onOpenSettingWindow('model')">
              <span class="text">{{ $t('settings.model.add') }}</span>
              <cs name="add" style="margin-left: var(--cs-space-xs)" />
            </div>
          </div>
        </template>
        <template #right>
          <div
            class="icon-btn upperLayer"
            :class="{ disabled: !canCreateNewConversation }"
            @click="newChat">
            <cs name="new-chat" />
          </div>
        </template>
      </titlebar>

      <div class="chat-main">
        <!-- side bar -->
        <el-aside :width="sidebarWidth" class="sidebar" :class="{ collapsed: sidebarCollapsed }">
          <div class="sidebar-header upperLayer">
            <el-input v-model="searchQuery" :placeholder="$t('chat.searchChat')" :clearable="true">
              <template #prefix>
                <cs name="search" />
              </template>
            </el-input>
            <el-tooltip
              :content="$t('chat.showFavoriteConversations')"
              placement="top"
              :hide-after="0"
              transition="none">
              <cs
                class="favourite-flag-icon"
                :name="favoriteFlag ? 'favourite-fill' : 'favourite1'"
                :active="favoriteFlag"
                @click.stop="favoriteFlag = !favoriteFlag" />
            </el-tooltip>
          </div>
          <div v-show="!sidebarCollapsed" class="conversations">
            <div class="list">
              <template v-for="(date, idx) in dateGroupKeys" :key="idx">
                <template v-if="conversationsForShow[date]?.length > 0">
                  <div class="date">{{ $t(`chat.date.${date}`) }}</div>
                  <div
                    class="item"
                    v-for="(chat, index) in conversationsForShow[date]"
                    @click="selectConversation(chat.id)"
                    @mouseenter="hoveredConversionIndex = chat.id"
                    @mouseleave="hoveredConversionIndex = null"
                    :key="index"
                    :class="{ active: chat.id === chatStore.currentConversationId }">
                    {{ chat.title }}
                    <div class="icons" v-show="chat.id === hoveredConversionIndex">
                      <div
                        class="icon icon-favourite"
                        @click.stop="onFavouriteConversation(chat.id)">
                        <cs
                          :name="chat.isFavorite ? 'favourite-fill' : 'favourite'"
                          :active="chat.isFavorite" />
                      </div>
                      <div class="icon icon-edit" @click.stop="onEditConversation(chat.id)">
                        <cs name="edit" />
                      </div>
                      <div class="icon icon-delete" @click.stop="onDeleteConversation(chat.id)">
                        <cs name="delete" />
                      </div>
                    </div>
                  </div>
                </template>
              </template>
            </div>
          </div>
        </el-aside>

        <!-- main container -->
        <el-container class="main-container">
          <!-- conversation container -->
          <div class="messages" ref="chatMessagesRef" :key="forceRefreshKey">
            <div ref="observerTarget"></div>

            <div class="empty-message" v-if="!canChat">
              {{ $t('chat.haveNoModel') }}
            </div>
            <div v-else-if="chatStore.messages.length === 0 && !isChatting" class="empty-message">
              <logo :name="currentModel?.logo || 'ai-common'" class="logo" size="40" />
            </div>

            <!-- message list -->
            <div
              v-for="(message, index) in messagesForShow"
              :key="index"
              :id="'message-' + message.id"
              class="message"
              :class="message.role"
              @mouseenter="hoveredMessageIndex = index"
              @mouseleave="hoveredMessageIndex = null">
              <div class="avatar">
                <cs v-if="message.role === 'user'" name="talk" class="user-icon" />
                <logo
                  v-else
                  :name="
                    message?.metadata?.provider
                      ? getModelLogo(message.metadata.provider)
                      : currentModel?.logo
                  " />
                <span class="provider" v-if="message.metadata?.provider">
                  {{ message.metadata.provider }}
                </span>
              </div>
              <div class="content-container">
                <!-- <div
                  class="content"
                  v-html="parseMarkdown(message.content)"
                  v-highlight
                  v-link
                  v-katex
                  v-mermaid
                  v-think /> -->
                <div class="content" v-if="message.role === 'user'">
                  <pre class="simple-text">{{ message.content }}</pre>
                </div>
                <markdown
                  :content="message.content"
                  :reference="message.metadata?.reference || []"
                  :reasoning="message.metadata?.reasoning || ''"
                  :toolCalls="message.metadata?.toolCall || []"
                  :log="chatState.log"
                  :plan="chatState.plan"
                  v-else />
                <div class="metadata">
                  <div class="buttons">
                    <el-tooltip
                      :content="$t('chat.resendMessage')"
                      :hide-after="0"
                      placement="top"
                      transition="none"
                      v-if="message.role == 'user'">
                      <cs name="resend" @click="onResendMessage(message.id)" class="icon-resend" />
                    </el-tooltip>
                    <el-tooltip
                      :content="$t('chat.quoteMessage')"
                      :hide-after="0"
                      placement="top"
                      transition="none"
                      v-else>
                      <cs name="quote" @click="onReplyMessage(message.id)" class="icon-quote" />
                    </el-tooltip>
                    <el-tooltip
                      :content="$t('chat.copyMessage')"
                      :hide-after="0"
                      placement="top"
                      transition="none">
                      <cs name="copy" @click="onCopyMessage(message.id)" class="icon-copy" />
                    </el-tooltip>
                    <el-tooltip
                      :content="$t('chat.takeNote')"
                      :hide-after="0"
                      placement="top"
                      transition="none"
                      v-if="message.role != 'user'">
                      <cs name="note" @click="onTakeNote(message)" class="icon-note" />
                    </el-tooltip>
                    <el-tooltip
                      :content="$t('chat.deleteMessage')"
                      :hide-after="0"
                      placement="top"
                      transition="none">
                      <cs name="delete" @click="onDeleteMessage(message.id)" class="icon-delete" />
                    </el-tooltip>
                  </div>
                  <div class="tokens" v-show="hoveredMessageIndex === index">
                    <div class="item" v-if="message?.metadata?.prompt">
                      <label>{{ $t('chat.metadata.prompt') }}:</label>
                      <span>{{ message?.metadata?.prompt }}</span>
                    </div>
                    <div class="item" v-if="message?.metadata?.completion">
                      <label>{{ $t('chat.metadata.completion') }}:</label>
                      <span>{{ message?.metadata?.completion }}</span>
                    </div>
                    <div class="item" v-if="message?.metadata?.tokens">
                      <label>{{ $t('chat.metadata.tokens') }}:</label>
                      <span>{{ message?.metadata?.tokens }}</span>
                    </div>
                    <div class="item" v-if="message?.metadata?.tokensPerSecond">
                      <span>
                        {{
                          $t('chat.metadata.speed', {
                            speed: Math.round((message?.metadata?.tokensPerSecond * 100) / 100)
                          })
                        }}</span
                      >
                    </div>
                  </div>
                </div>

                <div v-if="message?.metadata?.contextCleared" class="context-cleared">
                  <label>{{ $t('chat.contextCleared') }}</label>
                </div>
              </div>
            </div>

            <!-- chatting message -->
            <div v-if="isChatting" class="message assistant" :class="{ loading: isChatting }">
              <div class="avatar">
                <logo
                  :name="chatState.model ? getModelLogo(chatState.model) : currentModel?.logo" />
              </div>
              <div class="content-container" :class="{ chatting: isChatting }">
                <chatting
                  :content="currentAssistantMessage"
                  :reference="chatState.reference"
                  :reasoning="chatState.reasoning"
                  :log="chatState.log"
                  :plan="chatState.plan"
                  :toolCalls="chatState.toolCall || []"
                  :is-reasoning="chatState.isReasoning" />
              </div>
            </div>

            <!-- error message -->
            <div v-if="chatErrorMessage" class="message error">
              <div class="avatar">
                <cs name="error" />
              </div>
              <div class="content-container">
                <div class="content">{{ chatErrorMessage }}</div>
              </div>
            </div>
            <div style="height: var(--cs-space)" v-if="selectedSkill || replyMessage" />
          </div>

          <!-- footer -->
          <el-footer class="input-container">
            <div class="skill-list-container" v-show="isSkillListVisible">
              <SkillList
                ref="skillListRef"
                @onSelected="onSkillSelected"
                @visibleChanged="onSkillListVisibleChanged"
                :searchKw="skillSearchKeyword" />
            </div>
            <div class="additional" v-if="selectedSkill || replyMessage">
              <div class="additional-item" v-if="selectedSkill">
                <div class="data">
                  <SkillItem :skill="selectedSkill" class="skill-item" />
                </div>
                <div class="close-btn" @click="selectedSkill = null">
                  <cs name="delete" />
                </div>
              </div>
              <div class="additional-item" v-if="replyMessage">
                <div class="data">
                  <span class="cs cs-quote message-text">{{ replyMessage }}</span>
                </div>
                <div class="close-btn" @click="replyMessage = ''">
                  <cs name="delete" />
                </div>
              </div>
            </div>
            <div class="input">
              <!-- message input -->
              <el-input
                ref="inputRef"
                v-model="inputMessage"
                type="textarea"
                :autosize="{ minRows: 1, maxRows: 10 }"
                :disabled="!canChat"
                :placeholder="$t('chat.inputMessagePlaceholder', { at: '@' })"
                @keydown.enter="onKeyEnter"
                @keydown="onKeyDown"
                @input="onInput"
                @compositionstart="onCompositionStart"
                @compositionend="onCompositionEnd" />

              <!-- chat icons -->
              <div class="input-footer">
                <div class="icons">
                  <!-- <el-tooltip
                    :content="
                      $t(`chat.${!deepSearchEnabled ? 'deepSearchEnabled' : 'deepSearchDisabled'}`)
                    "
                    :hide-after="0"
                    placement="top">
                    <label @click="onDeepSearchEnabled" :class="{ active: deepSearchEnabled }">
                      <cs name="skill-deep-search" class="small" />
                      {{ $t('chat.deepsearch') }}
                    </label>
                  </el-tooltip> -->
                  <el-tooltip
                    :content="$t(`chat.${!networkEnabled ? 'networkEnabled' : 'networkDisabled'}`)"
                    :hide-after="0"
                    placement="top"
                    v-if="!deepSearchEnabled">
                    <label @click="onToggleNetwork" :class="{ active: networkEnabled }">
                      <cs name="connected" class="small" />
                      {{ $t('chat.network') }}
                    </label>
                  </el-tooltip>
                  <label
                    @click="onToggleSkillSelector"
                    :class="{ active: isSkillListVisible }"
                    v-if="!deepSearchEnabled">
                    <cs class="small" name="tool" />{{ $t('chat.skills') }}
                  </label>
                  <el-tooltip
                    :content="$t(`chat.${disableContext ? 'enableContext' : 'disableContext'}`)"
                    :hide-after="0"
                    placement="top"
                    v-if="!deepSearchEnabled">
                    <label @click="onGlobalClearContext" :class="{ active: !disableContext }">
                      <cs name="clear-context" class="small" />
                      {{ $t('chat.context') }}
                    </label>
                  </el-tooltip>
                </div>
                <div class="icons">
                  <cs name="stop" @click="onStopChat" v-if="isChatting" />
                  <cs
                    v-else
                    name="send"
                    @click="dispatchChatCompletion(null)"
                    :class="{ disabled: !canSendMessage }" />
                </div>
              </div>
            </div>
          </el-footer>
        </el-container>
      </div>
    </el-container>

    <!-- eidt conversation -->
    <el-dialog
      v-model="editConversationDialogVisible"
      :title="$t('chat.editConversationTitle')"
      :close-on-press-escape="false"
      width="50%">
      <el-form>
        <el-form-item :label="$t('chat.conversationTitle')">
          <el-input v-model="editConversationTitle" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editConversationDialogVisible = false">{{
          $t('common.cancel')
        }}</el-button>
        <el-button type="primary" @click="onSaveEditConversation">{{
          $t('common.save')
        }}</el-button>
      </template>
    </el-dialog>

    <!-- add note dialog -->
    <el-dialog
      v-model="takeNoteDialogVisible"
      :title="$t('chat.takeNote')"
      :close-on-press-escape="false"
      :close-on-click-modal="false"
      width="50%"
      class="take-note-dialog">
      <el-form
        ref="takeNoteFormRef"
        :model="takeNoteForm"
        :rules="takeNoteRules"
        label-width="80px">
        <el-form-item :label="$t('chat.noteTags')" prop="tags">
          <el-select
            ref="tagsInputRef"
            v-model="takeNoteForm.tags"
            filterable
            allow-create
            multiple
            default-first-option
            :placeholder="$t('chat.noteTagsPlaceholder')"
            :no-data-text="$t('common.noData')"
            class="w-full">
            <el-option
              v-for="tag in noteStore.tags"
              :key="tag.id"
              :label="tag.name"
              :value="tag.name" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('chat.noteTitle')" prop="title">
          <el-input v-model="takeNoteForm.title" :placeholder="$t('chat.noteTitlePlaceholder')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="takeNoteDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="onSaveTakeNote(takeNoteFormRef)">
          {{ $t('common.save') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount, reactive, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import markdown from '@/components/chat/Markdown.vue'
import chatting from '@/components/chat/Chatting.vue'
import SkillList from '@/components/chat/SkillList.vue'
import Titlebar from '@/components/window/Titlebar.vue'

import { csStorageKey } from '@/config/config'
import { chatPreProcess } from '@/libs/chat'
import { getModelLogo } from '@/libs/logo'
import { getLanguageByCode } from '@/i18n/langUtils'
import { isEmpty, showMessage, csGetStorage, csSetStorage, Uuid } from '@/libs/util'

import { useChatStore } from '@/stores/chat'
import { useModelStore } from '@/stores/model'
import { useNoteStore } from '@/stores/note'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'

const { t } = useI18n()
const unlistenChunkResponse = ref(null)
const unlistenSendMessage = ref(null)

const chatStore = useChatStore()
const modelStore = useModelStore()
const noteStore = useNoteStore()
const windowStore = useWindowStore()
const settingStore = useSettingStore()

const mainWindowIsAlwaysOnTop = computed(() => windowStore.mainWindowAlwaysOnTop)

// edit conversation dialog
const editConversationDialogVisible = ref(false)
const editConversationId = ref(null)
const editConversationTitle = ref('')

// load model providers
const modelProviders = computed(() => modelStore.getAvailableProviders)
const currentModel = computed(() => modelStore.defaultModelProvider)
const currentModelDetail = computed(() =>
  currentModel.value?.models.find(m => m.id === currentModel.value.defaultModel)
)
const currentSubModels = computed(() =>
  modelStore.defaultModelProvider?.models?.reduce((groups, x) => {
    if (!x.group) {
      x.group = t('settings.model.ungrouped')
    }
    groups[x.group] = groups[x.group] || []
    groups[x.group].push(x)
    return groups
  }, {})
)
const currentModelAlias = computed(() => {
  const cfg = currentModel.value.models.find(m => m.id === currentModel.value.defaultModel)
  console.log(currentModel.value.defaultModel)
  return cfg?.name || cfg?.id.split('/').pop()
})

const canChat = computed(() => modelProviders.value.length > 0)

const chatMessagesRef = ref(null)
const sidebarCollapsed = ref(!windowStore.chatSidebarShow)
const sidebarWidth = computed(() => (sidebarCollapsed.value ? '0px' : '200px'))
const searchQuery = ref('')

// 每次只加载20条消息，根据用户滚动，用户向上滚动触顶后再加载一页
const observerTarget = ref(null)
const messageReady = ref(false)
const messagesForShow = ref([])
const isLoadingMore = ref(false)
const hasMoreMessages = ref(true)
const userHasScrolled = ref(false)
const isScrolledToBottom = ref(true)
const hoveredMessageIndex = ref(null)
const hoveredConversionIndex = ref(null)
const pageSize = 10

const inputRef = ref(null)
const inputMessage = ref('')
const chatErrorMessage = ref('')
const replyMessage = ref('')
const composing = ref(false)
const compositionJustEnded = ref(false)
const isChatting = ref(false)
const currentAssistantMessage = ref('')
const lastChatId = ref('')
const titleChatId = ref('')
const getDefaultChatState = () => ({
  message: '',
  reference: [],
  reasoning: '',
  isReasoning: false,
  log: [],
  plan: [],
  model: '',
  toolCall: []
})

const chatState = ref(getDefaultChatState())

// network connection and deep search
// When enabled, it will automatically crawl the URLs in user queries
const networkEnabled = ref(csGetStorage(csStorageKey.networkEnabled, true))
// When deep search is enabled, the AI will automatically plan the user's questions
// and break them down into executable steps for research.
const deepSearchEnabled = ref(csGetStorage(csStorageKey.deepSearchEnabled, false))

const skillListRef = ref(null)
const selectedSkill = ref(null)
const isSkillListVisible = ref(false)

const forceRefreshKey = ref(0)

// take note dialog
const takeNoteDialogVisible = ref(false)
const takeNoteFormRef = ref(null)
const tagsInputRef = ref(null)
const takeNoteForm = reactive({
  title: '',
  content: '',
  conversationId: 0,
  messageId: 0,
  tags: [],
  reference: [],
  reasoning: ''
})
const takeNoteRules = {
  tags: [{ required: true, message: t('chat.noteTagsRequired'), trigger: 'blur' }],
  title: [{ required: true, message: t('chat.noteTitleRequired'), trigger: 'blur' }]
}
// clear context
const disableContext = ref(csGetStorage(csStorageKey.disableContext, false))

/**
 * Try to get the user's language from the setting, if not found, return 'English'
 */
const myLanguage = computed(() => {
  const language = settingStore.settings.primaryLanguage
  return getLanguageByCode(language) || 'English'
})

/**
 * The user must have at least one model available and should not have initiated a new topic to create a new conversation.
 */
const canCreateNewConversation = computed(
  () =>
    canChat.value &&
    (chatStore.messages.length > 0 ||
      (chatStore.messages.length == 0 && chatStore.currentConversationId < 1))
)

/**
 * The user must have at least one model available,
 * and should not be sending a message and message is not empty.
 */
const canSendMessage = computed(
  () => canChat.value && !isChatting.value && !isEmpty(inputMessage.value.trim())
)

/**
 * Check if the current skill is a translation skill
 */
const isTranslation = computed(() => {
  return selectedSkill.value?.metadata?.type === 'translation'
})

/**
 * Check if the current skill has tools enabled
 */
const toolsEnabled = computed(() => {
  if (!currentModelDetail.value?.functionCall) {
    return false
  }
  // 1. If no skill is selected, tools can be enabled (global tools or default behavior)
  if (!selectedSkill.value) {
    return true
  }
  // 2. If a skill is selected, it must not be a translation skill AND its metadata must allow tools
  return !isTranslation.value && !!selectedSkill.value.metadata?.toolsEnabled
})

// listen AI response, update scroll
watch([() => currentAssistantMessage.value, () => chatState.value.reasoning], () => {
  nextTick(() => {
    if (!userHasScrolled.value || isScrolledToBottom.value) {
      scrollToBottomIfNeeded()
    }
  })
})

watch(
  () => chatStore.messages?.length,
  newLength => {
    if (!messageReady.value) {
      return
    }
    if (newLength <= 0) {
      messagesForShow.value = []
      hasMoreMessages.value = false
      return
    }

    // 只在消息数量变化时才重置
    if (newLength !== messagesForShow.value.length) {
      const startIndex = Math.max(0, newLength - pageSize)
      messagesForShow.value = chatStore.messages.slice(startIndex, newLength)

      scrollToBottomIfNeeded()
    }
  }
)

// =================================================
//  handle scroll
// =================================================

/**
 * Scroll to the bottom of the chat messages if conditions are met
 */
const scrollToBottomIfNeeded = () => {
  if (chatMessagesRef.value && (!userHasScrolled.value || isScrolledToBottom.value)) {
    chatMessagesRef.value.scrollTop = chatMessagesRef.value.scrollHeight
  }
}

/**
 * Handle scroll event of chat messages container
 */
const onScroll = () => {
  if (chatMessagesRef.value) {
    const { scrollTop, scrollHeight, clientHeight } = chatMessagesRef.value
    isScrolledToBottom.value = scrollTop + clientHeight >= scrollHeight - 10
    if (!isScrolledToBottom.value) {
      userHasScrolled.value = true
    }
  }
}

/**
 * Reset scroll behavior when starting a new chat or sending a message
 */
const resetScrollBehavior = () => {
  userHasScrolled.value = false
  isScrolledToBottom.value = true
}

/**
 * Scroll to the bottom of the chat messages
 */
const scrollToBottom = () => {
  if (chatMessagesRef.value) {
    chatMessagesRef.value.scrollTop = chatMessagesRef.value.scrollHeight
  }
}

// =================================================
//  conversations and chat messages
// =================================================
const dateGroupKeys = [
  'today',
  'yesterday',
  'twoDaysAgo',
  'thisWeek',
  'lastWeek',
  'thisMonth',
  'lastMonth',
  'thisYear',
  'lastYear',
  'earlier'
]

const favoriteFlag = ref(false)

/**
 * 创建新的日期分组对象
 * @returns {Object}
 */
const createDateGroups = () => {
  return dateGroupKeys.reduce((acc, key) => {
    acc[key] = []
    return acc
  }, {})
}

/**
 * Group conversations by date
 * @param {Array} conversations
 * @returns {Object}
 */
const groupConversationsByDate = conversations => {
  if (isEmpty(conversations)) {
    return {}
  }
  const now = new Date()
  const oneDay = 24 * 60 * 60 * 1000
  const oneWeek = 7 * oneDay

  const groups = createDateGroups()

  conversations.forEach(conversation => {
    if (favoriteFlag.value) {
      if (!conversation.isFavorite) {
        return
      }
    }
    const createdDate = new Date(conversation.createdAt)
    const timeDiff = now - createdDate

    if (timeDiff < oneDay) {
      groups.today.push(conversation)
    } else if (timeDiff < 2 * oneDay) {
      groups.yesterday.push(conversation)
    } else if (timeDiff < 3 * oneDay) {
      groups.twoDaysAgo.push(conversation)
    } else if (timeDiff < oneWeek) {
      groups.thisWeek.push(conversation)
    } else if (timeDiff < 2 * oneWeek) {
      groups.lastWeek.push(conversation)
    } else if (
      createdDate.getMonth() === now.getMonth() &&
      createdDate.getFullYear() === now.getFullYear()
    ) {
      groups.thisMonth.push(conversation)
    } else if (
      createdDate.getMonth() === now.getMonth() - 1 &&
      createdDate.getFullYear() === now.getFullYear()
    ) {
      groups.lastMonth.push(conversation)
    } else if (createdDate.getFullYear() === now.getFullYear()) {
      groups.thisYear.push(conversation)
    } else if (createdDate.getFullYear() === now.getFullYear() - 1) {
      groups.lastYear.push(conversation)
    } else {
      groups.earlier.push(conversation)
    }
  })

  return groups
}

/**
 * conversations for show
 */
const conversationsForShow = computed(() => {
  if (isEmpty(searchQuery.value)) {
    return groupConversationsByDate(chatStore.conversations)
  }

  const filteredConversations = chatStore.conversations.filter(conversation =>
    conversation.title.toLowerCase().includes(searchQuery.value.toLowerCase())
  )
  return groupConversationsByDate(filteredConversations)
})

/**
 * Load messages with pagination
 */
const loadMessagesForObserver = async () => {
  if (!messageReady.value || isLoadingMore.value) {
    return
  }

  const totalMessages = chatStore.messages.length
  const loadedMessages = messagesForShow.value.length

  if (loadedMessages >= totalMessages) {
    hasMoreMessages.value = false
    return
  }

  isLoadingMore.value = true
  try {
    const endIndex = totalMessages - loadedMessages
    const startIndex = Math.max(0, endIndex - pageSize)
    const newMessages = chatStore.messages.slice(startIndex, endIndex)

    if (newMessages.length === 0) {
      hasMoreMessages.value = false
      return
    }

    console.debug(
      'Loading messages:',
      startIndex,
      'to',
      endIndex,
      'hasMore:',
      hasMoreMessages.value
    )

    // Remember the ID of the last new message (this message will be inserted in front of the currently displayed messages)
    const anchorMessageId =
      messagesForShow.value.length > 0
        ? messagesForShow.value[0].id
        : newMessages[newMessages.length - 1].id

    // Add new messages
    messagesForShow.value.splice(0, 0, ...newMessages)
    hasMoreMessages.value = startIndex > 0

    // Wait for DOM update and scroll to the correct position
    await nextTick()

    // Use requestAnimationFrame to ensure scrolling to the correct position after the next frame is rendered
    requestAnimationFrame(() => {
      const targetMessage = document.getElementById(`message-${anchorMessageId}`)
      const container = chatMessagesRef.value
      if (targetMessage && container) {
        // Scroll to the target message and leave a little space
        container.scrollTop = targetMessage.offsetTop
      }
    })
  } catch (error) {
    console.error('Error loading messages:', error)
  } finally {
    isLoadingMore.value = false
  }
}

/**
 * Reset pagination state when switching conversations
 */
const resetPagination = () => {
  messagesForShow.value = []
  hasMoreMessages.value = true
  isLoadingMore.value = false
}

/**
 * Select a conversation and load messages
 */
const selectConversation = id => {
  chatStore.setCurrentConversationId(id)
  messageReady.value = false
  resetPagination() // Reset pagination state
  chatStore.loadMessages(id, settingStore.windowLabel)
}

/**
 * Dispatch chat completion event to the backend
 */
const dispatchChatCompletion = async (messageId = null) => {
  if (!canSendMessage.value && !messageId) {
    return
  }

  if (chatStore.currentConversationId < 1) {
    await chatStore.getCurrentConversationId()
  }

  let userMessage = ''
  // 如果回复消息，则将回复消息设置为空
  if (replyMessage.value) {
    userMessage =
      replyMessage.value.replace(/<think[^>]*>[\s\S]+?<\/think>/g, '').trim() +
      '\n\n' +
      t('chat.quoteMessagePrompt') +
      '\n\n'
    replyMessage.value = ''
  }

  userMessage += messageId
    ? chatStore.messages.find(m => m.id === messageId)?.content?.trim() || ''
    : inputMessage.value.trim()
  if (!userMessage) {
    console.error('no user message to send')
    return
  }

  let historyMessages = []
  if (settingStore.settings.historyMessages > 0 && !disableContext.value) {
    historyMessages = chatStore.messages.slice(-1 * (settingStore.settings.historyMessages * 2 + 1))
    if (
      historyMessages.length > 0 &&
      historyMessages[historyMessages.length - 1].id === messageId
    ) {
      historyMessages.pop()
    }
  }
  const messages = await chatPreProcess(userMessage, historyMessages, '', selectedSkill.value, {})
  if (messages.length < 1) {
    console.error('no messages to send')
    return
  }
  console.log('messages:', messages)

  resetScrollBehavior() // reset scroll behavior

  chatStore
    .addChatMessage(chatStore.currentConversationId, 'user', userMessage, null, messageId)
    .then(async () => {
      resetChat()
      isChatting.value = true
      lastChatId.value = Uuid()

      // Scroll to bottom immediately
      nextTick(() => {
        scrollToBottom()
      })

      try {
        console.log('tool enabled:', toolsEnabled.value)
        await invoke('chat_completion', {
          providerId: currentModel.value.id,
          model: currentModel.value.defaultModel,
          chatId: lastChatId.value,
          messages: messages,
          networkEnabled: networkEnabled.value,
          metadata: {
            windowLabel: settingStore.windowLabel,
            toolsEnabled: toolsEnabled.value,
            reasoning: currentModelDetail.value?.reasoning || false
          }
        })
      } catch (error) {
        chatErrorMessage.value = t('chat.errorOnSendMessage', { error })
        console.error('error on sendMessage:', error)
        isChatting.value = false
      }
    })
    .catch(error => {
      chatErrorMessage.value = t('chat.errorOnSaveMessage', { error })
      console.error('error on addChatMessage:', error)
    })
}

const proxyType = computed(() => {
  return currentModel.value?.metadata?.proxyType === 'none'
    ? 'none'
    : settingStore.settings.proxyType || 'bySetting'
})

/**
 * Create a new chat and focus on the input field
 */
const newChat = () => {
  if (!canCreateNewConversation.value) {
    return
  }
  chatErrorMessage.value = ''
  resetScrollBehavior() // reset scroll behavior
  chatStore.createConversation().then(() => {
    nextTick(() => {
      if (inputRef.value) {
        inputRef.value.focus()
      }
      scrollToBottomIfNeeded()
    })
  })
}

/**
 * Execute deep search
 * @param {string} messageId
 */
const deepSearch = (messageId = null) => {
  // TODO: Implement deep search functionality
  return
  if (!deepSearchEnabled.value) {
    return
  }
  let userMessage = messageId
    ? chatStore.messages.find(m => m.id === messageId)?.content?.trim() || ''
    : inputMessage.value.trim()
  if (!userMessage) {
    console.error('no user message to send')
    return
  }
  resetScrollBehavior() // reset scroll behavior

  chatStore
    .addChatMessage(chatStore.currentConversationId, 'user', userMessage, null, messageId)
    .then(async () => {
      resetChat()
      isChatting.value = true
      lastChatId.value = Uuid()

      // Scroll to bottom immediately
      nextTick(() => {
        scrollToBottom()
      })
      try {
        await invoke('deep_search', {
          chatId: lastChatId.value,
          question: userMessage,
          metadata: {
            windowLabel: settingStore.windowLabel,
            model: currentModel.value.defaultModel
          }
        })
      } catch (error) {
        chatErrorMessage.value = t('chat.errorOnSendMessage', { error })
        console.error('error on sendMessage:', error)
        isChatting.value = false
      }
    })
    .catch(error => {
      chatErrorMessage.value = t('chat.errorOnSaveMessage', { error })
      console.error('error on addChatMessage:', error)
    })
}

/**
 * Automatically send message or deep search based on settings
 * @param {int} messageId
 */
const sendMessageAuto = (messageId = null) => {
  if (deepSearchEnabled.value) {
    deepSearch(messageId)
  } else {
    dispatchChatCompletion(messageId)
  }
}

/**
 * Reset chat state
 */
const resetChat = () => {
  chatState.value = getDefaultChatState()
  chatErrorMessage.value = ''
  replyMessage.value = ''
  inputMessage.value = ''
  currentAssistantMessage.value = ''
}

const title = ref('')
const titleGenerating = ref(false)
const titleRetryCount = ref(0)
const MAX_TITLE_RETRY = 3

/**
 * Generate a title for the current conversation by AI
 */
const genTitleByAi = () => {
  if (chatStore.messages.length < 2 || titleGenerating.value) {
    return
  }
  console.log('generate title by ai')
  titleGenerating.value = true
  const messages = [
    ...chatStore.messages
      .slice(0, 2)
      .map(message => ({ role: message.role, content: message.content.trim() })),
    {
      role: 'user',
      content: `Please generate a clear topic for this conversation, limited to 10 characters, without including quotation marks, apostrophes, backticks, or any non-alphanumeric characters. Respond in ${myLanguage.value} if supported; otherwise, use English.`
    }
  ]
  let genModel = currentModel.value
  let model = currentModel.value.defaultModel
  if (settingStore.settings.conversationTitleGenModel?.id) {
    genModel =
      modelStore.getModelProviderById(settingStore.settings.conversationTitleGenModel.id) ||
      currentModel.value
    model = settingStore.settings.conversationTitleGenModel?.model || model
  }
  titleChatId.value = Uuid()
  invoke('chat_completion', {
    providerId: genModel.id,
    model: model,
    chatId: titleChatId.value,
    messages: messages,
    metadata: {
      stream: true,
      maxTokens: 50,
      action: 'gen_title',
      conversationId: chatStore.currentConversationId,
      windowLabel: settingStore.windowLabel,
      toolsEnabled: toolsEnabled.value
    }
  }).catch(error => {
    titleGenerating.value = false
    console.error('error on genTitleByAi:', error)
    // add retry logic
    if (titleRetryCount.value < MAX_TITLE_RETRY) {
      titleRetryCount.value++
      console.log(`Retrying generate title (${titleRetryCount.value}/${MAX_TITLE_RETRY})...`)
      setTimeout(() => {
        genTitleByAi()
      }, 3000) // retry after 3 seconds
    } else {
      console.error('Max retry attempts reached for title generation')
      titleRetryCount.value = 0 // reset retry count
      showMessage(t('chat.errorOnGenerateTitle'), 'error', 3000)
    }
  })
}

/**
 * Handle title generated event
 */
const handleTitleGenerated = payload => {
  switch (payload?.type) {
    case 'error':
      console.error('error on genTitleByAi:', payload.error)
      titleGenerating.value = false
      // add retry logic
      if (titleRetryCount.value < MAX_TITLE_RETRY) {
        titleRetryCount.value++
        console.log(`Retrying generate title (${titleRetryCount.value}/${MAX_TITLE_RETRY})...`)
        setTimeout(() => {
          genTitleByAi()
        }, 3000) // retry after 3 seconds
      } else {
        console.error('Max retry attempts reached for title generation')
        titleRetryCount.value = 0 // reset retry count
        showMessage(t('chat.errorOnGenerateTitle'), 'error', 3000)
      }
      return

    case 'text':
      title.value += payload?.chunk || ''
      break

    case 'finished':
      payload.isDone = true
      break
  }
  if (payload?.isDone) {
    if (title.value.trim().length > 0) {
      // remove leading and trailing double quotes
      title.value = title.value.replace(/^"|"$/g, '').replace(/<think[^>]*>[\s\S]+?<\/think>/g, '')
      if (title.value.length > 0) {
        console.log('conversation title:', title.value)
        chatStore.updateConversation(payload?.metadata?.conversationId, title.value)
      }
    }
    title.value = ''
    titleGenerating.value = false
    titleRetryCount.value = 0 // reset retry count
  }
}

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
        console.log('reference', payload?.chunk)
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

    case 'toolCalls':
      if (!chatState.value.message.includes('<!--[ToolCalls]-->')) {
        chatState.value.message += '\n<!--[ToolCalls]-->\n'
      }
      break

    case 'toolResults':
      if (typeof payload?.chunk === 'string') {
        const parsedChunk = JSON.parse(payload?.chunk || '[]')
        if (Array.isArray(parsedChunk)) {
          chatState.value.toolCall.push(...parsedChunk)
        } else {
          console.error('Expected an array but got:', typeof parsedChunk)
        }
      } else {
        chatState.value.toolCall.push(...payload?.chunk)
        hasToolCalls = true
      }
      break

    case 'log':
      chatState.value.log.push(payload?.chunk || '')
      break
    case 'plan':
      if (payload?.chunk) {
        try {
          const plan = JSON.parse(payload?.chunk || '[]')
          chatState.value.plan = Array.isArray(plan) ? [...plan] : []
        } catch (e) {
          console.log('error on parse plan:', e)
          console.log('chunk', payload?.chunk)
        }
      }
      break
    default:
      console.warn('Unknown message type:', payload?.type, payload?.chunk)
      break
  }

  chatState.value.model = payload?.metadata?.model || currentModel.value.defaultModel || ''
  currentAssistantMessage.value = chatState.value.message
  nextTick(() => {
    if (!userHasScrolled.value || isScrolledToBottom.value) {
      scrollToBottomIfNeeded()
    }
  })

  if (isDone) {
    isChatting.value = false
    lastChatId.value = ''
    if (chatState.value.message.trim().length > 0) {
      // 保存当前滚动位置和高度，用于后续恢复
      const scrollInfo = {
        top: chatMessagesRef.value.scrollTop,
        height: chatMessagesRef.value.scrollHeight,
        isAtBottom:
          chatMessagesRef.value.scrollTop + chatMessagesRef.value.clientHeight >=
          chatMessagesRef.value.scrollHeight - 10
      }
      // 保存当前状态用于后续恢复
      const originalMessage = chatState.value.message.trim()
      const originalReference =
        chatState.value.reference && Array.isArray(chatState.value.reference)
          ? [...chatState.value.reference]
          : []
      const originalReasoning = chatState.value.reasoning || ''
      const originalToolCall = chatState.value.toolCall || []

      // 提前重置状态（核心优化点）
      chatState.value = getDefaultChatState()
      currentAssistantMessage.value = ''

      try {
        await chatStore.addChatMessage(
          chatStore.currentConversationId,
          'assistant',
          originalMessage,
          {
            tokens: payload?.metadata?.tokens?.total || 0,
            prompt: payload?.metadata?.tokens?.prompt || 0,
            completion: payload?.metadata?.tokens?.completion || 0,
            tokensPerSecond: payload?.metadata?.tokens?.tokensPerSecond || 0,
            provider: payload?.metadata?.model || currentModel.value.defaultModel || '',
            reference: originalReference,
            reasoning: originalReasoning,
            toolCall: originalToolCall
          }
        )
        // 一次性更新所有状态，减少DOM重绘次数
        // chatState.value = getDefaultChatState()
        // currentAssistantMessage.value = ''

        // 在DOM更新后恢复滚动位置
        nextTick(() => {
          if (scrollInfo.isAtBottom) {
            // 如果原本在底部，则滚动到新的底部
            scrollToBottom()
          } else {
            // 否则尝试保持相对滚动位置
            const heightDiff = chatMessagesRef.value.scrollHeight - scrollInfo.height
            chatMessagesRef.value.scrollTop = scrollInfo.top + heightDiff
          }
        })

        // generate title if needed
        if (chatStore.messages.length <= 2) {
          genTitleByAi()
        }
      } catch (error) {
        chatErrorMessage.value = t('chat.errorOnSaveMessage', { error })
      }
    }
  }
}

// =================================================
//  lifecycle
// =================================================
const cleanupObserver = ref(null)
const setupObserver = () => {
  const options = {
    root: chatMessagesRef.value,
    threshold: 0.1,
    rootMargin: '100px 0px 0px 0px'
  }

  const observer = new IntersectionObserver(entries => {
    const entry = entries[0]
    if (entry.isIntersecting && hasMoreMessages.value && !isLoadingMore.value) {
      loadMessagesForObserver()
    }
  }, options)

  nextTick(() => {
    if (observerTarget.value) {
      observer.observe(observerTarget.value)
    }
  })

  return () => observer.disconnect()
}

onMounted(async () => {
  if (inputRef.value) {
    inputRef.value.focus()
  }

  await chatStore.loadConversations() // Ensure this is awaited

  // listen send_message event
  unlistenSendMessage.value = await listen('chat_message', async event => {
    if (event?.payload?.windowLabel === settingStore.windowLabel) {
      if (event?.payload?.done) {
        messageReady.value = true
        loadMessagesForObserver()
        setTimeout(() => {
          scrollToBottom()
        }, 300)
      } else {
        chatStore.appendMessage(event?.payload?.message)
      }
    }
  })

  chatStore
    .getCurrentConversationId()
    .then(() => {
      if (chatStore.currentConversationId > 0) {
        messageReady.value = false
        chatStore.loadMessages(chatStore.currentConversationId, settingStore.windowLabel)
      }
    })
    .catch(error => {
      chatErrorMessage.value = t('chat.errorOnGetCurrentConversationId', { error })
    })

  // listen chat_stream event
  unlistenChunkResponse.value = await listen('chat_stream', async event => {
    // we don't want to process messages from other windows
    if (event.payload?.metadata?.windowLabel !== settingStore.windowLabel) {
      return
    }
    // console.log('payload', event?.payload)
    // console.log('chat_stream', event)
    const payload = event.payload
    if (payload?.metadata?.action === 'gen_title') {
      if (payload?.chatId === titleChatId.value) {
        handleTitleGenerated(payload)
      }
    } else {
      if (payload?.chatId === lastChatId.value) {
        handleChatMessage(payload)
      } else {
        console.log('chatId not matched,', 'lastChatId:', lastChatId.value, ', payload:', payload)
      }
    }
  })

  await listen('sync_state', event => {
    if (event.payload.windowLabel !== settingStore.windowLabel) {
      return
    }
    if (
      event?.payload?.type === 'conversation_switch' &&
      event?.payload?.metadata?.conversationId
    ) {
      resetPagination()
      chatStore.setCurrentConversationId(event.payload.metadata.conversationId)
      chatStore.loadConversations()
      chatStore
        .loadMessages(event.payload.metadata.conversationId, settingStore.windowLabel)
        .then(() => {
          genTitleByAi()

          currentAssistantMessage.value = ''
          isChatting.value = false
          nextTick(() => {
            scrollToBottomIfNeeded()
          })
        })
        .catch(error => {
          chatErrorMessage.value = t('chat.errorOnLoadMessages', { error })
        })
      inputRef.value?.focus()
    }
  })

  if (chatMessagesRef.value) {
    chatMessagesRef.value.addEventListener('scroll', onScroll)
  }

  cleanupObserver.value = setupObserver()

  windowStore.initMainWindowAlwaysOnTop()
})

onBeforeUnmount(() => {
  if (isChatting.value) {
    // stop chat
    invoke('stop_chat', { apiProtocol: currentModel.value.apiProtocol })
    isChatting.value = false
  }
  // unlisten send_message event
  unlistenSendMessage.value?.()
  // unlisten chat_stream event
  unlistenChunkResponse.value?.()

  chatMessagesRef.value?.removeEventListener('scroll', onScroll)

  cleanupObserver.value?.()
})

// =================================================
//  handle events
// =================================================

/**
 * Toggle the sidebar and update the local storage
 */
const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  windowStore.setChatSidebarShow(!sidebarCollapsed.value)
}

/**
 * Handle pin event
 */
const onPin = async () => {
  await windowStore.toggleMainWindowAlwaysOnTop()
}

/**
 * Toggle the favorite status of a conversation
 * @param {Number} id conversation id
 */
const onFavouriteConversation = id => {
  const conversation = chatStore.conversations.find(conversation => conversation.id === id)
  if (!conversation) {
    showMessage(t('chat.conversationNotFound'), 'error', 3000)
    return
  }
  chatStore.updateConversation(id, null, !conversation.isFavorite).catch(error => {
    showMessage(t('chat.errorOnUpdateConversation', { error }), 'error', 3000)
  })
}

/**
 * Edit conversation title, open a dialog
 * @param {Number} id conversation id
 */
const onEditConversation = id => {
  editConversationId.value = id
  editConversationTitle.value = chatStore.conversations.find(
    conversation => conversation.id === id
  ).title
  editConversationDialogVisible.value = true
}
/**
 * Save the edited conversation title
 */
const onSaveEditConversation = () => {
  if (!editConversationId.value) {
    return
  }
  chatStore
    .updateConversation(editConversationId.value, editConversationTitle.value)
    .then(() => {
      editConversationDialogVisible.value = false
      editConversationTitle.value = ''
      editConversationId.value = null
      showMessage(t('chat.conversationTitleUpdated'), 'success', 1000)
    })
    .catch(error => {
      showMessage(t('chat.errorOnUpdateConversation', { error }), 'error', 3000)
    })
}
/**
 * Delete a conversation
 * @param {Number} id conversation id
 */
const onDeleteConversation = id => {
  ElMessageBox.confirm(t('chat.confirmDeleteConversation'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel')
  }).then(() => {
    resetPagination()
    const oldCurrentConversationId = chatStore.currentConversationId
    chatStore
      .deleteConversation(id)
      .then(() => {
        // 用户删除了当前的会话，则跳转到下一个会话，如果当前没有会话，则新建一个会话
        if (oldCurrentConversationId == id) {
          if (chatStore.currentConversationId > 0) {
            messageReady.value = false
            chatStore.loadMessages(chatStore.currentConversationId, settingStore.windowLabel)
          } else {
            newChat()
          }
        }
      })
      .catch(error => {
        showMessage(t('chat.errorOnDeleteConversation', { error }), 'error', 3000)
      })
  })
}

const onOpenSettingWindow = type => {
  invoke('open_setting_window', { settingType: type })
}

/**
 * Set the default model
 * @param {Object} id model config
 */
const onModelChange = id => {
  modelStore.setDefaultModelProvider(modelProviders.value.find(model => model.id === id))
}

/**
 * Set the default sub model
 * @param {String} modelId sub model id
 */
const onSubModelChange = modelId => {
  currentModel.value.defaultModel = modelId
  modelStore.setDefaultModelProvider(currentModel.value)
  const logo = currentModel.value?.metadata?.logo || ''
  // update the database record
  modelStore
    .setModelProvider({
      ...currentModel.value,
      defaultModel: modelId,
      metadata: {
        ...currentModel.value.metadata,
        proxyType: currentModel.value?.metadata?.proxyType || 'bySetting',
        logo: logo
      }
    })
    .catch(error => {
      console.error(error)
    })
}

/**
 * Stop chat
 */
const onStopChat = () => {
  const cmd = deepSearchEnabled.value ? 'stop_deep_search' : 'stop_chat'
  const param = { chatId: lastChatId.value }
  if (!deepSearchEnabled.value) {
    param.apiProtocol = currentModel.value.apiProtocol
  }
  invoke(cmd, param)
    .then(() => {
      if (chatState.value.message.trim()) {
        chatStore
          .addChatMessage(
            chatStore.currentConversationId,
            'assistant',
            chatState.value.message.trim(),
            {
              reference: chatState.value?.reference || [],
              reasoning: chatState.value?.reasoning || ''
            }
          )
          .catch(error => {
            chatErrorMessage.value = t('chat.errorOnSaveMessage', { error })
          })
      }
    })
    .catch(error => {
      showMessage(t('chat.errorOnStopChat', { error }), 'error', 3000)
    })
    .finally(() => {
      lastChatId.value = ''
      isChatting.value = false
      currentAssistantMessage.value = ''
      chatState.value = getDefaultChatState()

      nextTick(() => {
        if (!userHasScrolled.value || isScrolledToBottom.value) {
          scrollToBottomIfNeeded()
        }
      })
    })
}

/**
 * Resend message
 * @param {Number} id message id
 */
const onResendMessage = id => {
  sendMessageAuto(id)
}

/**
 * Reply message
 * @param {Number} id message id
 */
const onReplyMessage = id => {
  replyMessage.value = chatStore.messages.find(message => message.id === id)?.content?.trim() || ''
  inputRef.value.focus()
}

/**
 * Copy message content
 * @param {Number} id message id
 */
const onCopyMessage = id => {
  try {
    const content = chatStore.messages.find(message => message.id === id).content
    navigator.clipboard.writeText(content)
    showMessage(t('chat.messageCopied'), 'success', 1000)
  } catch (error) {
    showMessage(t('chat.errorOnCopyMessage', { error }), 'error', 3000)
  }
}

/**
 * Delete message
 * @param {Number} id message id
 */
const onDeleteMessage = id => {
  ElMessageBox.confirm(t('chat.confirmDeleteMessage'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel')
  }).then(() => {
    chatStore
      .deleteMessage(id)
      .then(() => {
        scrollToBottomIfNeeded()
      })
      .catch(error => {
        showMessage(t('chat.errorOnDeleteMessage', { error }), 'error', 3000)
      })
  })
}
/**
 * Handle skill selected event
 * @param {Object} skill skill object
 */
const onSkillSelected = skill => {
  // save the selected skill
  selectedSkill.value = skill

  // handle the input content, delete @ and search keyword
  if (lastInputValue.value) {
    const lastAtIndex = lastInputValue.value.lastIndexOf('@')
    if (lastAtIndex !== -1) {
      // keep the content before @
      inputMessage.value = lastInputValue.value.slice(0, lastAtIndex)
      // update lastInputValue, ensure the state is synchronized
      lastInputValue.value = inputMessage.value
    }
  }

  // clear the search keyword
  skillSearchKeyword.value = ''
}
/**
 * Toggle the network enabled state
 */
const onToggleNetwork = () => {
  networkEnabled.value = !networkEnabled.value
  csSetStorage(csStorageKey.networkEnabled, networkEnabled.value)
}

/**
 * Toggle the deep search enabled state
 */
const onDeepSearchEnabled = () => {
  deepSearchEnabled.value = !deepSearchEnabled.value
  csSetStorage(csStorageKey.deepSearchEnabled, deepSearchEnabled.value)

  if (deepSearchEnabled.value) {
    replyMessage.value = ''
  }
}

/**
 * Toggle the skill selector visibility
 * @param {Boolean} v
 */
const onSkillListVisibleChanged = v => {
  isSkillListVisible.value = v
}
/**
 * Open the skill selector
 */
const onToggleSkillSelector = () => {
  if (skillListRef.value) {
    skillListRef.value.toggle()
  }
}
/**
 * Toggle the clear the global context enabled state
 */
const onGlobalClearContext = () => {
  disableContext.value = !disableContext.value
  csSetStorage(csStorageKey.disableContext, disableContext.value)
}

// =================================================
// handle keyboard events
// =================================================

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

/**
 * Handle enter key event
 */
const onKeyEnter = event => {
  // if the skill list is visible, do not handle the enter event
  if (skillListRef.value?.isVisible) {
    event.preventDefault()
    return
  }

  if (event.shiftKey) {
    // if shift+enter is pressed, allow line breaks
    return
  }
  if (!composing.value && !compositionJustEnded.value) {
    event.preventDefault()
    sendMessageAuto()
  }
}

// add a variable to track the last input content
const lastInputValue = ref('')
const skillSearchKeyword = ref('')

/**
 * Handle input event
 * @param {String} value
 */
const onInput = value => {
  // if typing with pinyin, do not handle
  if (composing.value) return

  // check if just input @
  const currentValue = value
  const lastChar = currentValue.slice(-1)

  if (lastChar === '@') {
    // show the skill list
    skillListRef.value?.show()
    skillSearchKeyword.value = ''
  } else if (lastInputValue.value.includes('@')) {
    // if the last input contains @, it means the user is typing the search keyword
    const lastAtIndex = lastInputValue.value.lastIndexOf('@')
    skillSearchKeyword.value = currentValue.slice(lastAtIndex + 1)
  }

  lastInputValue.value = currentValue
}

/**
 * Handle keydown event
 * @param {KeyboardEvent} event
 */
const onKeyDown = event => {
  // if the Esc key is pressed, hide the skill list
  if (event.key === 'Escape') {
    skillListRef.value?.hide()
  }
}

/**
 * Save take note
 */
const onSaveTakeNote = async () => {
  if (!takeNoteFormRef.value) return

  await takeNoteFormRef.value.validate((valid, fields) => {
    if (valid) {
      takeNoteDialogVisible.value = false
      noteStore
        .addNote(
          takeNoteForm.title,
          takeNoteForm.content,
          takeNoteForm.conversationId,
          takeNoteForm.messageId,
          takeNoteForm.tags.join(','),
          {
            reference: takeNoteForm.reference,
            reasoning: takeNoteForm.reasoning
          }
        )
        .then(() => {
          showMessage(t('chat.noteSaved'), 'success', 3000)
        })
        .catch(error => {
          showMessage(t('chat.noteSaveFailed', { error }), 'error', 5000)
          console.log(error)
        })
    }
  })
}

/**
 * Reset form when take note dialog is opened
 * @param {Object} message
 */
const onTakeNote = message => {
  noteStore.getTagList()

  nextTick(() => {
    takeNoteFormRef.value?.resetFields()
    takeNoteForm.tags = []
    takeNoteForm.title = ''
    takeNoteForm.content = message.content
    takeNoteForm.conversationId = chatStore.currentConversationId
    takeNoteForm.messageId = message.id
    takeNoteForm.reference = message?.metadata?.reference
    takeNoteForm.reasoning = message?.metadata?.reasoning
    takeNoteDialogVisible.value = true
    // Focus on tags input after dialog is shown
    setTimeout(() => {
      tagsInputRef.value?.focus()
    }, 300)
  })
}
</script>

<style lang="scss">
.chat {
  height: 100vh;
  overflow: hidden;

  .chat-container {
    height: 100%;
    display: flex;
    flex-direction: column;

    .header {
      .model-selector {
        display: flex;
        flex-direction: row;
        align-items: center;
        justify-content: center;
      }
    }

    .chat-main {
      flex: 1;
      display: flex;
      flex-direction: row;
      height: 100%;
      overflow: hidden;
    }
  }

  .sidebar {
    height: 100%;
    overflow-y: auto;
    background-color: var(--cs-bg-color-deep);
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    color: var(--cs-text-color-primary);
    border-right: 0.5px solid var(--cs-titlebar-border-color);
    box-shadow: 0 0 2px 0 var(--cs-titlebar-border-color);
    transition: width 0.3s ease-in-out;

    .sidebar-header {
      display: flex;
      flex-direction: row;
      align-items: center;
      justify-content: space-between;
      margin: var(--cs-space-sm) var(--cs-space) var(--cs-space-sm);

      .favourite-flag-icon {
        flex-shrink: 0;
        cursor: pointer;
        width: 24px;
        height: 24px;
        font-size: 20px !important;
        margin-left: var(--cs-space-xs);
      }

      .el-input {
        width: calc(100% - var(--cs-space) * 2 - 24px);
        flex: 1;

        box-sizing: border-box;

        .el-input__wrapper {
          padding: 0;
          background: var(--cs-input-bg-color) !important;
          border-radius: var(--cs-border-radius-xxl);
          font-size: var(--cs-font-size-sm);
        }

        .el-input__prefix {
          display: flex;
          align-items: center;
          padding-left: var(--cs-space-sm);

          .cs {
            font-size: var(--cs-font-size-md);
            color: var(--cs-text-color-secondary);
          }
        }
      }
    }

    .conversations {
      flex: 1;
      display: flex;
      flex-direction: column;
      overflow-y: auto;
      padding: 0 var(--cs-space-sm);
      transition: all 0.3s ease-in-out;

      .list {
        display: flex;
        flex-direction: column;
        flex-grow: 1;
        border-right: none;
        background: transparent;

        .date {
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-secondary);
          padding: var(--cs-space-sm);
        }

        .item {
          cursor: pointer;
          padding: var(--cs-space-sm);
          border-radius: var(--cs-border-radius);
          font-size: var(--cs-font-size);
          box-sizing: border-box;
          width: 100%;
          overflow: hidden;
          white-space: nowrap;
          text-overflow: ellipsis;
          transition: all 0.3s ease-in-out;
          position: relative;

          &.active {
            background-color: var(--cs-active-bg-color);
            color: var(--cs-text-color-primary) !important;
          }

          &:hover {
            background-color: var(--cs-hover-bg-color);
          }

          .icons {
            position: absolute;
            right: var(--cs-space-xxs);
            top: 7px;
            display: flex;
            flex-direction: row;

            .icon {
              display: flex;
              align-items: center;
              justify-content: center;
              width: 24px;
              height: 24px;
              margin-left: var(--cs-space-xxs);
              background-color: var(--cs-bg-color-deep);
              border-radius: var(--cs-border-radius-round);
              cursor: pointer;

              .cs {
                color: var(--cs-text-color-primary);
              }

              &.icon-delete .cs {
                font-size: var(--cs-font-size-xxs) !important;
              }
            }
          }
        }
      }
    }
  }

  .main-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    position: relative;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: var(--cs-space-xs);
    padding-bottom: var(--cs-space);

    .message {
      display: flex;
      flex-direction: column;
      align-items: flex-start;
      margin-bottom: var(--cs-space);
      position: relative;
      transform: translateZ(0);

      .avatar {
        display: flex;
        align-items: center;
        margin: 0 var(--cs-space-xs);
        flex-shrink: 0;

        .provider {
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-secondary);
          margin-left: var(--cs-space-xxs);
        }
      }

      .content-container {
        display: flex;
        flex-direction: column;

        position: relative;
        max-width: calc(100% - var(--cs-font-size-xxl) * 2 - var(--cs-space-xs) * 2);
      }

      &.user {
        flex-direction: row-reverse;

        .content {
          background-color: var(--cs-bg-color-deep);

          code {
            max-height: 300px;
          }
        }
      }

      &.assistant {
        .content-container {
          flex: 1;
          margin-left: var(--cs-space-lg);
          width: calc(100vw - var(--cs-space-lg));

          &.chatting {
            flex: unset;
          }
        }
      }

      &.error {
        .avatar {
          .cs {
            color: var(--cs-error-color) !important;
          }
        }

        .content {
          color: var(--cs-error-color);
          background-color: var(--cs-error-bg-color);
        }
      }
    }

    .empty-message {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 100%;
      width: 100%;
      padding: var(--cs-space-lg);
      box-sizing: border-box;
    }
  }

  footer.input-container {
    flex-shrink: 0;
    background-color: transparent;
    padding: 0 var(--cs-space-sm) var(--cs-space-sm);
    height: unset;
    z-index: 1;

    .additional {
      display: flex;
      gap: 1px;
      margin-bottom: var(--cs-space-xs);

      .additional-item {
        display: flex;
        align-items: center;
        flex: 1;
        max-width: 50%;
        background-color: var(--cs-input-bg-color);
        border-radius: var(--cs-border-radius-xxl);
        padding: var(--cs-space-xs);
        box-sizing: border-box;

        .data {
          flex: 1;
          min-width: 0;

          .skill-item {
            padding: 0;
          }

          .message-text {
            padding-left: var(--cs-space);
            display: block;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
            color: var(--cs-text-color-secondary);
            font-size: var(--cs-font-size-sm);
            line-height: 1.5;
            position: relative;

            &:before {
              position: absolute;
              top: -3px;
              left: 3px;
            }
          }
        }

        .close-btn {
          display: flex;
          align-items: center;
          justify-content: center;
          width: 24px;
          height: 24px;
          margin-left: var(--cs-space-xs);
          flex-shrink: 0;
          cursor: pointer;
          border-radius: var(--cs-border-radius-round);
          color: var(--cs-text-color-secondary);

          &:hover {
            background-color: var(--cs-bg-color-light);
          }
        }
      }
    }

    .input {
      display: flex;
      flex-direction: column;
      background-color: var(--cs-input-bg-color);
      border-radius: var(--cs-border-radius-lg);
      padding: var(--cs-space-sm) var(--cs-space) var(--cs-space-xs);

      .icons {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: var(--cs-space-xs);
        cursor: pointer;
        gap: var(--cs-space-xs);

        .cs {
          font-size: var(--cs-font-size-xl) !important;
          color: var(--cs-text-color-secondary);

          &.small {
            font-size: var(--cs-font-size-md) !important;
          }

          &.cs-send {
            color: var(--cs-text-color-primary);
          }
        }

        label {
          font-size: var(--cs-font-size-sm);
          display: flex;
          align-items: center;
          justify-content: center;
          cursor: pointer;
          color: var(--cs-text-color-secondary);
          background-color: var(--cs-bg-color);
          border-radius: var(--cs-border-radius-lg);
          padding: var(--cs-space-xs) var(--cs-space-sm);
          border: 1px solid var(--cs-bg-color);

          &:hover,
          &.active {
            color: var(--cs-color-primary);
            border: 1px solid var(--cs-color-primary);

            .cs {
              color: var(--cs-color-primary);
            }
          }
        }
      }

      .el-textarea {
        flex-grow: 1;

        .el-textarea__inner {
          border: none;
          box-shadow: none;
          background: var(--cs-input-bg-color) !important;
          resize: none !important;
          color: var(--cs-text-color-primary);
          padding-left: var(--cs-space-xxs);
          padding-right: var(--cs-space-xxs);
        }
      }

      .input-footer {
        display: flex;
        flex-direction: row;
        align-items: center;
        justify-content: space-between;
      }
    }
  }
}

.take-note-dialog {
  &.el-dialog {
    min-width: 350px;
  }
}

.pin-btn {
  border-radius: var(--cs-border-radius-xs);
  color: var(--cs-text-color-secondary);

  &:hover .cs {
    color: var(--cs-color-primary) !important;
  }

  .cs {
    font-size: var(--cs-font-size-md) !important;
    transform: rotate(45deg);
    transition: all 0.3s ease-in-out;
  }

  &.active {
    .cs {
      color: var(--cs-color-primary);
      transform: rotate(0deg);
    }
  }
}
</style>
