<template>
  <div class="proxy-switcher-window" @mousedown.stop tabindex="0" @keydown="handleKeyDown" ref="windowRef">
    <div class="header">
      <span class="title">{{ $t('proxySwitcher.title') }}</span>
      <div class="header-actions upperLayer">
        <span class="icon-btn close-btn" @click.stop="handleHide">
          <cs name="close" size="14px" />
        </span>
      </div>
    </div>

    <div class="proxy-list" v-if="proxyGroupStore.list.length > 0" ref="listRef">
      <div v-for="(group, index) in sortedProxyGroupList" :key="group.id" class="proxy-item" :class="{
        active: proxyGroupStore.activeGroup === group.name,
        focused: selectedIndex === index
      }" @click="handleActivateGroup(group.name)" @mouseenter="selectedIndex = index">
        <div class="group-info">
          <div class="name-row">
            <span class="name">{{ group.name }}</span>
            <el-tag v-if="proxyGroupStore.activeGroup === group.name" type="success" size="small" effect="plain" round
              class="active-tag">
              {{ $t('settings.proxyGroup.activeGroup') }}
            </el-tag>
          </div>
          <div class="description" v-if="group.description">{{ group.description }}</div>
        </div>

        <div class="actions" @click.stop>
          <el-tooltip placement="top" :hide-after="0">
            <template #content>
              {{ $t('settings.proxyGroup.toolCompatMode') }}: {{
                $t(`settings.proxyGroup.toolCompatModes.${group.metadata?.toolCompatMode || 'auto'}`) }}
            </template>
            <span class="icon-btn action-btn" @click="handleToggleToolCompatMode(group)">
              <cs :name="getToolCompatModeIcon(group.metadata?.toolCompatMode || 'auto')" size="14px"
                :active="(group.metadata?.toolCompatMode || 'auto') !== 'auto'" />
            </span>
          </el-tooltip>

          <span class="icon-btn action-btn activate-btn" v-if="proxyGroupStore.activeGroup !== group.name"
            @click="handleActivateGroup(group.name)">
            <cs name="check-circle" size="16px" color="secondary" />
          </span>
          <span class="icon-btn action-btn active" v-else>
            <cs name="check-circle" size="16px" :active="true" />
          </span>
        </div>
      </div>
    </div>
    <div v-else class="empty-state">
      {{ $t('settings.proxyGroup.noGroupsFound') }}
    </div>
  </div>
</template>

<script setup>
import { onMounted, computed, onUnmounted, ref, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { showMessage } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

const { t } = useI18n()
const proxyGroupStore = useProxyGroupStore()
const appWindow = getCurrentWebviewWindow()

const windowRef = ref(null)
const listRef = ref(null)
const selectedIndex = ref(0)
const isHiding = ref(false)
let unlistenFocus = null

const sortedProxyGroupList = computed(() => {
  return [...proxyGroupStore.list].sort((a, b) => {
    return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })
  })
})

const handleHide = async () => {
  if (isHiding.value) return
  isHiding.value = true
  try {
    await appWindow.hide()
  } catch (e) {
    console.error('Failed to hide window:', e)
  } finally {
    isHiding.value = false
  }
}

const handleActivateGroup = async (name) => {
  if (proxyGroupStore.activeGroup === name) return
  try {
    await proxyGroupStore.setActiveGroup(name)
    sendSyncState('proxy_group_changed', 'proxy_switcher', { activeGroup: name })
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: String(error) }), 'error')
  }
}

const handleKeyDown = (e) => {
  if (sortedProxyGroupList.value.length === 0) return
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    selectedIndex.value = (selectedIndex.value + 1) % sortedProxyGroupList.value.length
    ensureVisible()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    selectedIndex.value = (selectedIndex.value - 1 + sortedProxyGroupList.value.length) % sortedProxyGroupList.value.length
    ensureVisible()
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const group = sortedProxyGroupList.value[selectedIndex.value]
    if (group) handleActivateGroup(group.name)
  } else if (e.key === 'Escape') {
    e.preventDefault()
    handleHide()
  }
}

const ensureVisible = () => {
  nextTick(() => {
    const focusedItem = listRef.value?.querySelector('.proxy-item.focused')
    if (focusedItem) {
      focusedItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
    }
  })
}

const getToolCompatModeIcon = (mode) => {
  switch (mode) {
    case 'compat': return 'xml'
    case 'native': return 'hammer'
    default: return 'setting'
  }
}

const handleToggleToolCompatMode = async (group) => {
  const currentMode = group.metadata?.toolCompatMode || 'auto'
  const modeMap = { auto: 'compat', compat: 'native', native: 'auto' }
  const newMode = modeMap[currentMode]
  try {
    const updatedGroup = {
      ...group,
      metadata: { ...group.metadata, toolCompatMode: newMode }
    }
    await proxyGroupStore.update(updatedGroup)
    sendSyncState('proxy_group_updated', 'proxy_switcher', { group: updatedGroup })
    showMessage(t('settings.proxyGroup.toolCompatModeChanged', { mode: t(`settings.proxyGroup.toolCompatModes.${newMode}`) }), 'success')
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: String(error) }), 'error')
  }
}

onUnmounted(() => {
  if (unlistenFocus) unlistenFocus()
})

onMounted(async () => {
  await proxyGroupStore.getList()
  const activeIdx = sortedProxyGroupList.value.findIndex(g => g.name === proxyGroupStore.activeGroup)
  if (activeIdx !== -1) selectedIndex.value = activeIdx
  
  nextTick(() => {
    windowRef.value?.focus()
  })

  unlistenFocus = await appWindow.onFocusChanged(({ payload: focused }) => {
    if (!focused && !isHiding.value) {
      handleHide()
    }
  })
})
</script>

<style lang="scss" scoped>
.proxy-switcher-window {
  width: 100%;
  height: 100vh;
  background-color: var(--cs-bg-color);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: var(--cs-shadow-lg);
  user-select: none;
  outline: none;
}

.header {
  height: 40px;
  min-height: 40px;
  padding: 0 var(--cs-space);
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--cs-border-color);
  background-color: var(--cs-bg-color-light);
  -webkit-app-region: drag;

  .title {
    font-size: 14px;
    font-weight: 600;
    color: var(--cs-text-color);
  }

  .header-actions {
    display: flex;
    gap: var(--cs-space-xs);
    -webkit-app-region: no-drag;

    .close-btn {
      &:hover {
        background-color: var(--el-color-danger-light-9);
        color: var(--el-color-danger);
      }
    }
  }
}

.proxy-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--cs-space-sm);
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-xs);

  &::-webkit-scrollbar {
    width: 4px;
  }

  &::-webkit-scrollbar-thumb {
    background: var(--cs-border-color);
    border-radius: 2px;
  }
}

.proxy-item {
  padding: var(--cs-space-sm) var(--cs-space);
  border-radius: var(--cs-border-radius-md);
  border: 1px solid transparent;
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
  transition: all 0.1s;
  background-color: var(--cs-bg-color-light);

  &:hover,
  &.focused {
    background-color: var(--cs-bg-color-hover);
    border-color: var(--cs-border-color);
  }

  &.active {
    border-color: var(--cs-color-primary);
  }

  .group-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;

    .name-row {
      display: flex;
      align-items: center;
      gap: 8px;

      .name {
        font-size: 13px;
        font-weight: 500;
        color: var(--cs-text-color);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }

      .active-tag {
        font-size: 10px;
        height: 16px;
        padding: 0 4px;
        line-height: 14px;
      }
    }

    .description {
      font-size: 11px;
      color: var(--cs-text-color-secondary);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    margin-left: var(--cs-space-sm);
  }
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--cs-border-radius-sm);
  cursor: pointer;
  color: var(--cs-text-color-secondary);
  transition: all 0.2s;

  &:hover {
    background-color: var(--cs-bg-color-hover);
    color: var(--cs-text-color);
  }

  &.action-btn {
    width: 28px;
    height: 28px;

    &:hover {
      background-color: rgba(var(--cs-color-primary-rgb), 0.1);
      color: var(--cs-color-primary);
    }
  }

  &.active {
    color: var(--cs-color-primary);
  }
}

.empty-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--cs-text-color-secondary);
  font-size: 13px;
}
</style>