<template>
  <div class="card">
    <div class="title">
      <span>{{ $t('settings.chatHub.title') }}</span>
      <el-tooltip :content="$t('settings.chatHub.add')" placement="left" :hide-after="0" :enterable="false">
        <span class="icon" @click="openChatHubDialog()">
          <cs name="add" />
        </span>
      </el-tooltip>
    </div>

    <Sortable v-if="chatHubStore.list.length > 0" class="list" item-key="id" :list="chatHubStore.list"
      :options="{
        animation: 150,
        ghostClass: 'ghost',
        dragClass: 'drag',
        draggable: '.draggable',
        forceFallback: true,
        bubbleScroll: true
      }" @update="onChatHubSortUpdate" @end="onChatHubDragEnd">
      <template #item="{ element }">
        <div class="item draggable chat-hub-item" :key="element.id">
          <div class="label">
            <img v-if="chatHubLogo(element)" :src="chatHubLogo(element)" class="chat-hub-logo"
              @error="onChatHubLogoError(element)" />
            <avatar v-else :text="element.name" :size="20" />
            <span class="chat-hub-name" :title="element.url">{{ element.name }}</span>
            <el-tag v-if="element.isDefault" size="small" type="info">
              {{ $t('settings.chatHub.preset') }}
            </el-tag>
          </div>
          <div class="value">
            <el-tooltip :content="$t('settings.chatHub.edit')" placement="top" :hide-after="0" :enterable="false"
              transition="none">
              <div class="icon" @click="openChatHubDialog(element)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip :content="$t('settings.chatHub.delete')" placement="top" :hide-after="0" :enterable="false"
              transition="none">
              <div class="icon" @click="deleteChatHub(element)" @mousedown.stop>
                <cs name="trash" size="16px" color="secondary" />
              </div>
            </el-tooltip>
          </div>
        </div>
      </template>
    </Sortable>

    <div class="list" v-else>
      <div class="item">
        <div class="label">{{ $t('settings.chatHub.empty') }}</div>
      </div>
    </div>

    <div class="chat-hub-hint">
      <small class="tooltip">{{ $t('settings.chatHub.manageTooltip') }}</small>
    </div>
  </div>

  <el-dialog v-model="chatHubDialogVisible" width="560px"
    :title="chatHubForm.id ? $t('settings.chatHub.editTitle') : $t('settings.chatHub.addTitle')"
    @closed="resetChatHubForm">
    <el-form label-width="90px">
      <el-form-item :label="$t('settings.chatHub.name')">
        <el-input v-model="chatHubForm.name" maxlength="200"
          :placeholder="$t('settings.chatHub.namePlaceholder')" />
      </el-form-item>
      <el-form-item :label="$t('settings.chatHub.url')">
        <el-input v-model="chatHubForm.url" :placeholder="$t('settings.chatHub.urlPlaceholder')" />
      </el-form-item>
      <el-form-item :label="$t('settings.chatHub.logo')">
        <div class="chat-hub-logo-field">
          <el-input v-model="chatHubForm.logo" :placeholder="$t('settings.chatHub.logoPlaceholder')" />
          <el-button @click="fillChatHubFavicon">
            {{ $t('settings.chatHub.useFavicon') }}
          </el-button>
        </div>
      </el-form-item>
      <el-form-item :label="$t('settings.chatHub.logoPreview')">
        <div class="chat-hub-logo-preview">
          <img v-if="chatHubFormLogo" :src="chatHubFormLogo" class="chat-hub-logo" @error="chatHubFormLogoFailed = true" />
          <avatar v-else :text="chatHubForm.name" :size="20" />
          <small class="tooltip">{{ $t('settings.chatHub.logoPreviewHint') }}</small>
        </div>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="chatHubDialogVisible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="chatHubSaving" @click="saveChatHub">{{ $t('common.confirm') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Sortable } from 'sortablejs-vue3'

import { FrontendAppError } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useChatHubStore } from '@/stores/chatHub'

/**
 * ChatHub settings page.
 *
 * Entries are stored in their own database table through the chat hub store, and
 * this page only renders and edits that metadata. Nothing here is written into
 * the generic settings map.
 */
const { t } = useI18n()
const chatHubStore = useChatHubStore()

const chatHubDialogVisible = ref(false)
const chatHubSaving = ref(false)
const chatHubForm = reactive({ id: 0, name: '', logo: '', url: '' })
const chatHubFormLogoFailed = ref(false)
const chatHubBrokenLogos = ref(new Set())

const chatHubFormLogo = computed(() =>
  chatHubFormLogoFailed.value ? '' : chatHubForm.logo.trim()
)

const chatHubLogo = hub => (hub.logo && !chatHubBrokenLogos.value.has(hub.id) ? hub.logo : '')

const onChatHubLogoError = hub => {
  chatHubBrokenLogos.value.add(hub.id)
}

const isValidChatHubUrl = value => {
  const trimmed = (value || '').trim()
  if (!trimmed) {
    return false
  }
  try {
    const parsed = new URL(trimmed)
    return (parsed.protocol === 'http:' || parsed.protocol === 'https:') && !!parsed.hostname
  } catch {
    return false
  }
}

const reportChatHubError = (message, error) => {
  showMessage(message, 'error')
  if (error instanceof FrontendAppError) {
    console.error('ChatHub operation failed:', error.toFormattedString(), error.originalError)
  } else {
    console.error('ChatHub operation failed:', error)
  }
}

const reorderListByIndexes = (list, oldIndex, newIndex) => {
  if (!Array.isArray(list) || oldIndex === null || newIndex === null || oldIndex === newIndex) {
    return list
  }
  const nextList = [...list]
  const [movedItem] = nextList.splice(oldIndex, 1)
  if (!movedItem) {
    return list
  }
  nextList.splice(newIndex, 0, movedItem)
  return nextList
}

const onChatHubSortUpdate = event => {
  const reordered = reorderListByIndexes(chatHubStore.list, event.oldIndex, event.newIndex)
  if (reordered !== chatHubStore.list) {
    chatHubStore.list.splice(0, chatHubStore.list.length, ...reordered)
  }
}

const onChatHubDragEnd = async () => {
  try {
    await chatHubStore.reorder(chatHubStore.list.map(hub => hub.id))
  } catch (error) {
    reportChatHubError(t('settings.chatHub.reorderFailed'), error)
    await chatHubStore.load().catch(() => {})
  }
}

const openChatHubDialog = hub => {
  chatHubForm.id = hub?.id || 0
  chatHubForm.name = hub?.name || ''
  chatHubForm.logo = hub?.logo || ''
  chatHubForm.url = hub?.url || ''
  chatHubFormLogoFailed.value = false
  chatHubDialogVisible.value = true
}

const resetChatHubForm = () => {
  chatHubForm.id = 0
  chatHubForm.name = ''
  chatHubForm.logo = ''
  chatHubForm.url = ''
  chatHubFormLogoFailed.value = false
}

/**
 * Fills the logo field with the public Google favicon endpoint for the current
 * url. The image is never downloaded or parsed here, and an invalid or empty url
 * only reports an error without touching the existing logo.
 */
const fillChatHubFavicon = () => {
  if (!isValidChatHubUrl(chatHubForm.url)) {
    showMessage(t('settings.chatHub.faviconInvalidUrl'), 'error')
    return
  }
  chatHubForm.logo = `https://www.google.com/s2/favicons?sz=64&domain_url=${encodeURIComponent(
    chatHubForm.url.trim()
  )}`
  chatHubFormLogoFailed.value = false
}

const saveChatHub = async () => {
  const name = chatHubForm.name.trim()
  const url = chatHubForm.url.trim()
  const logo = chatHubForm.logo.trim()

  if (!name) {
    showMessage(t('settings.chatHub.nameRequired'), 'error')
    return
  }
  if (!isValidChatHubUrl(url)) {
    showMessage(t('settings.chatHub.urlInvalid'), 'error')
    return
  }
  if (logo && !isValidChatHubUrl(logo)) {
    showMessage(t('settings.chatHub.logoInvalid'), 'error')
    return
  }

  chatHubSaving.value = true
  try {
    if (chatHubForm.id) {
      await chatHubStore.update({ id: chatHubForm.id, name, logo, url })
      chatHubBrokenLogos.value.delete(chatHubForm.id)
    } else {
      await chatHubStore.add({ name, logo, url })
    }
    chatHubDialogVisible.value = false
    showMessage(t('settings.chatHub.saveSuccess'), 'success')
  } catch (error) {
    reportChatHubError(t('settings.chatHub.saveFailed'), error)
  } finally {
    chatHubSaving.value = false
  }
}

const deleteChatHub = async hub => {
  try {
    await ElMessageBox.confirm(
      t('settings.chatHub.deleteConfirm', { name: hub.name }),
      t('settings.chatHub.deleteConfirmTitle'),
      {
        confirmButtonText: t('common.confirm'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      }
    )
  } catch {
    // User clicked cancel
    return
  }

  try {
    await chatHubStore.remove(hub.id)
    chatHubBrokenLogos.value.delete(hub.id)
    showMessage(t('settings.chatHub.deleteSuccess', { name: hub.name }), 'success')
  } catch (error) {
    reportChatHubError(t('settings.chatHub.deleteFailed'), error)
  }
}

onMounted(async () => {
  await chatHubStore.load().catch(() => {})
  await chatHubStore.startSyncListener().catch(() => {})
})

onBeforeUnmount(() => {
  chatHubStore.stopSyncListener()
})
</script>

<style lang="scss">
.chat-hub-item {
  .chat-hub-name {
    overflow: hidden;
    max-width: 260px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .el-tag {
    margin-left: var(--cs-space-xs);
  }

  .icon {
    cursor: pointer;
  }
}

.chat-hub-logo {
  width: 18px;
  height: 18px;
  margin-right: var(--cs-space-xs);
  border-radius: var(--cs-border-radius-round);
  object-fit: contain;
}

.chat-hub-hint {
  padding: var(--cs-space-xs) var(--cs-space-sm) var(--cs-space-sm);
  color: var(--cs-text-color-secondary);
}

.chat-hub-logo-field {
  display: flex;
  width: 100%;
  gap: var(--cs-space-xs);

  .el-button + .el-button {
    margin-left: 0;
  }
}

.chat-hub-logo-preview {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  color: var(--cs-text-color-secondary);
}
</style>