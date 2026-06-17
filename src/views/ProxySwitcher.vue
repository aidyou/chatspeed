<template>
  <div
    class="proxy-switcher-window"
    @mousedown.stop
    tabindex="0"
    @keydown="handleKeyDown"
    ref="windowRef">
    <div class="header">
      <span class="title">{{ $t('proxySwitcher.title') }}</span>
      <div class="header-actions upperLayer">
        <span class="icon-btn close-btn" @click.stop="handleHide">
          <cs name="close" size="14px" />
        </span>
      </div>
    </div>

    <el-tabs v-model="activeTab" class="switcher-tabs">
      <el-tab-pane :label="$t('proxySwitcher.serverSwitch')" name="servers" />
      <el-tab-pane :label="$t('proxySwitcher.groupSwitch')" name="groups" />
    </el-tabs>

    <div v-if="activeTab === 'servers'" class="server-switch-panel">
      <div class="proxy-service-list" v-if="hasChatCompletionProxy">
        <div v-for="group in sortedProxyServerGroups" :key="group.name" class="list">
          <div
            class="title group-title"
            :class="{ active: expandedServerGroup === group.name }"
            @click="toggleServerGroup(group.name)">
            <span>{{ group.name }}</span>
            <cs
              :name="expandedServerGroup === group.name ? 'caret-down' : 'caret-right'"
              size="12px"
              class="arrow" />
          </div>

          <el-collapse-transition>
            <div v-show="expandedServerGroup === group.name" class="group-content">
              <div
                v-for="proxy in group.aliases"
                :key="proxy.alias"
                class="item"
                :class="{ active: selectedProxyKey === proxy.key }"
                @click="openServerModelSelector(group.name, proxy.alias)">
                <div class="label">
                  <Avatar :size="36" :text="proxy.alias" />
                  <div class="label-text">
                    {{ proxy.alias }}
                    <small>{{
                      $t('settings.proxy.mapsToModels', { count: proxy.targets.length })
                    }}</small>
                  </div>
                </div>

                <div class="value" @click.stop>
                  <el-tooltip
                    :content="$t('proxySwitcher.switchBackendModels')"
                    placement="top"
                    :hide-after="0"
                    :enterable="false">
                    <span
                      class="icon-btn action-btn"
                      :class="{ active: selectedProxyKey === proxy.key }"
                      @click="openServerModelSelector(group.name, proxy.alias)">
                      <cs name="edit" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                </div>
              </div>
            </div>
          </el-collapse-transition>
        </div>
      </div>
      <div v-else class="empty-state">
        {{ $t('settings.proxy.noProxiesFound') }}
      </div>

      <el-drawer
        v-model="modelDrawerVisible"
        direction="btt"
        size="86%"
        :show-close="false"
        :with-header="false"
        class="proxy-model-drawer">
        <div class="model-selector-panel">
          <div class="model-selector-header">
            <div class="model-selector-title">
              <span>{{ selectedProxyAlias }}</span>
              <small>{{ selectedProxyGroup }}</small>
            </div>
            <span class="icon-btn close-btn" @click="modelDrawerVisible = false">
              <cs name="close" size="14px" />
            </span>
          </div>

          <div class="model-selector-toolbar">
            <el-input
              v-model="searchQuery"
              :placeholder="$t('settings.proxy.form.searchModelsPlaceholder')"
              clearable
              class="search-input">
              <template #prefix>
                <cs name="search" />
              </template>
            </el-input>
            <el-checkbox v-model="filterByChecked">
              {{ $t('settings.proxy.form.checked') }}
            </el-checkbox>
          </div>

          <div class="selected-status">
            <span>{{ $t('settings.proxy.form.selectedCount') }}</span>
            <strong>{{ selectedTargets.length }}</strong>
          </div>

          <div class="providers-list">
            <el-scrollbar class="providers-scrollbar">
              <div v-if="filteredProviders.length === 0" class="no-models-found">
                {{ $t('settings.proxy.form.noMatchingModels') }}
              </div>

              <div v-for="provider in filteredProviders" :key="provider.id" class="provider-card">
                <div class="provider-header">
                  <div class="provider-title">
                    <img
                      v-if="provider.providerLogo"
                      :src="provider.providerLogo"
                      class="provider-logo"
                      alt="logo" />
                    <Avatar v-else :text="provider.name" :size="20" class="provider-avatar" />
                    <span>{{ provider.name }}</span>
                  </div>
                  <el-checkbox
                    :model-value="areAllModelsFromProviderSelected(provider)"
                    :indeterminate="
                      isAnyModelFromProviderSelected(provider) &&
                      !areAllModelsFromProviderSelected(provider)
                    "
                    @change="checked => handleSelectAllModelsFromProvider(provider, checked)">
                    {{ $t('settings.proxy.form.selectAll') }}
                  </el-checkbox>
                </div>

                <div class="models-grid">
                  <el-checkbox
                    v-for="model in provider.models"
                    :key="model.id"
                    :model-value="isTargetSelected(provider.id, model.id)"
                    :label="model.id"
                    border
                    class="model-checkbox"
                    @change="
                      checked => handleTargetSelectionChange(checked, provider.id, model.id)
                    ">
                    {{ model.id }}
                  </el-checkbox>
                </div>
              </div>
            </el-scrollbar>
          </div>
        </div>
      </el-drawer>
    </div>

    <div v-else-if="proxyGroupStore.list.length > 0" class="proxy-list" ref="listRef">
      <div
        v-for="(group, index) in sortedProxyGroupList"
        :key="group.id"
        class="proxy-item"
        :class="{
          active: proxyGroupStore.activeGroup === group.name,
          focused: selectedIndex === index
        }"
        @click="handleActivateGroup(group.name)"
        @mouseenter="selectedIndex = index">
        <div class="group-info">
          <div class="name-row">
            <span class="name">{{ group.name }}</span>
            <el-tag
              v-if="proxyGroupStore.activeGroup === group.name"
              type="success"
              size="small"
              effect="plain"
              round
              class="active-tag">
              {{ $t('settings.proxyGroup.activeGroup') }}
            </el-tag>
          </div>
          <div class="description" v-if="group.description">{{ group.description }}</div>
        </div>

        <div class="actions" @click.stop>
          <el-tooltip placement="top" :hide-after="0">
            <template #content>
              {{ $t('settings.proxyGroup.toolCompatMode') }}:
              {{
                $t(
                  `settings.proxyGroup.toolCompatModes.${group.metadata?.toolCompatMode || 'auto'}`
                )
              }}
            </template>
            <span class="icon-btn action-btn" @click="handleToggleToolCompatMode(group)">
              <cs
                :name="getToolCompatModeIcon(group.metadata?.toolCompatMode || 'auto')"
                size="14px"
                :active="(group.metadata?.toolCompatMode || 'auto') !== 'auto'" />
            </span>
          </el-tooltip>

          <span
            class="icon-btn action-btn activate-btn"
            v-if="proxyGroupStore.activeGroup !== group.name"
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
import { onMounted, computed, onUnmounted, ref, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { useSettingStore } from '@/stores/setting'
import { useModelStore } from '@/stores/model'
import { showMessage, isEmpty } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'
import Avatar from '@/components/common/Avatar.vue'

const { t } = useI18n()
const proxyGroupStore = useProxyGroupStore()
const settingStore = useSettingStore()
const modelStore = useModelStore()
const appWindow = getCurrentWebviewWindow()

const windowRef = ref(null)
const listRef = ref(null)
const selectedIndex = ref(0)
const isHiding = ref(false)
const activeTab = ref('servers')
const expandedServerGroup = ref('')
const selectedProxyGroup = ref('')
const selectedProxyAlias = ref('')
const modelDrawerVisible = ref(false)
const selectedTargets = ref([])
const searchQuery = ref('')
const filterByChecked = ref(false)
const saveTimer = ref(null)
let unlistenFocus = null

const sortedProxyGroupList = computed(() => {
  return [...proxyGroupStore.list].sort((a, b) => {
    return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })
  })
})

const chatCompletionProxy = computed(() => {
  const proxy = settingStore.settings.chatCompletionProxy || {}
  return Object.keys(proxy)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' }))
    .reduce((result, groupName) => {
      const groupProxies = proxy[groupName] || {}
      result[groupName] = Object.keys(groupProxies)
        .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' }))
        .reduce((groupResult, alias) => {
          groupResult[alias] = Array.isArray(groupProxies[alias]) ? groupProxies[alias] : []
          return groupResult
        }, {})
      return result
    }, {})
})

const sortedProxyServerGroups = computed(() => {
  return Object.entries(chatCompletionProxy.value)
    .map(([name, aliases]) => ({
      name,
      aliases: Object.entries(aliases).map(([alias, targets]) => ({
        alias,
        targets,
        key: `${name}::${alias}`
      }))
    }))
    .filter(group => group.aliases.length > 0)
})

const hasChatCompletionProxy = computed(() => sortedProxyServerGroups.value.length > 0)

const selectedProxyKey = computed(() => {
  if (!selectedProxyGroup.value || !selectedProxyAlias.value) return ''
  return `${selectedProxyGroup.value}::${selectedProxyAlias.value}`
})

const allProviders = computed(() =>
  modelStore.providers.filter(provider => {
    const proxyPort = settingStore.settings.chatCompletionProxyPort
    return (
      !provider?.disabled &&
      !provider?.baseUrl?.includes(`127.0.0.1:${proxyPort}`) &&
      !provider?.baseUrl?.includes(`localhost:${proxyPort}`)
    )
  })
)

const filteredProviders = computed(() => {
  let providers = [...allProviders.value]

  if (filterByChecked.value) {
    providers = providers
      .map(provider => ({
        ...provider,
        models: (provider.models || []).filter(model => isTargetSelected(provider.id, model.id))
      }))
      .filter(provider => provider.models.length > 0)
  }

  if (!searchQuery.value) {
    return providers
  }

  const query = searchQuery.value.toLowerCase()
  return providers
    .map(provider => {
      const providerNameMatch = provider.name.toLowerCase().includes(query)
      const models = provider.models || []
      if (providerNameMatch) return { ...provider, models }
      return {
        ...provider,
        models: models.filter(
          model =>
            model.name?.toLowerCase().includes(query) || model.id?.toLowerCase().includes(query)
        )
      }
    })
    .filter(provider => provider.models.length > 0)
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

const formatError = error => {
  if (typeof error?.toFormattedString === 'function') {
    return error.toFormattedString()
  }
  return error?.message || String(error)
}

const handleActivateGroup = async name => {
  if (proxyGroupStore.activeGroup === name) return
  try {
    await proxyGroupStore.setActiveGroup(name)
    sendSyncState('proxy_group_changed', 'proxy_switcher', { activeGroup: name })
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: formatError(error) }), 'error')
  }
}

const handleKeyDown = e => {
  if (activeTab.value !== 'groups') return
  if (sortedProxyGroupList.value.length === 0) return
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    selectedIndex.value = (selectedIndex.value + 1) % sortedProxyGroupList.value.length
    ensureVisible()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    selectedIndex.value =
      (selectedIndex.value - 1 + sortedProxyGroupList.value.length) %
      sortedProxyGroupList.value.length
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

const toggleServerGroup = groupName => {
  expandedServerGroup.value = expandedServerGroup.value === groupName ? '' : groupName
}

const openServerModelSelector = (groupName, alias) => {
  selectedProxyGroup.value = groupName
  selectedProxyAlias.value = alias
  const availableModelIds = new Set()
  allProviders.value.forEach(provider => {
    ;(provider.models || []).forEach(model => {
      availableModelIds.add(`${provider.id}::${model.id}`)
    })
  })
  const targets = JSON.parse(JSON.stringify(chatCompletionProxy.value[groupName]?.[alias] || []))
  selectedTargets.value =
    availableModelIds.size > 0
      ? targets.filter(target => availableModelIds.has(`${target.id}::${target.model}`))
      : targets
  searchQuery.value = ''
  filterByChecked.value = false
  modelDrawerVisible.value = true
}

const isTargetSelected = (providerId, modelId) => {
  return selectedTargets.value.some(target => target.id === providerId && target.model === modelId)
}

const saveSelectedTargets = async targets => {
  if (!selectedProxyGroup.value || !selectedProxyAlias.value) return
  if (targets.length === 0) {
    showMessage(t('settings.proxy.validation.targetsRequired'), 'warning')
    return
  }

  try {
    const newProxies = JSON.parse(JSON.stringify(settingStore.settings.chatCompletionProxy || {}))
    if (!newProxies[selectedProxyGroup.value]) {
      newProxies[selectedProxyGroup.value] = {}
    }
    newProxies[selectedProxyGroup.value][selectedProxyAlias.value] = targets
    await settingStore.setSetting('chatCompletionProxy', newProxies)
    sendSyncState('proxy_server_updated', 'proxy_switcher', {
      group: selectedProxyGroup.value,
      alias: selectedProxyAlias.value
    })
  } catch (error) {
    showMessage(t('settings.proxy.saveFailed', { error: formatError(error) }), 'error')
  }
}

const queueSaveSelectedTargets = () => {
  if (saveTimer.value) {
    clearTimeout(saveTimer.value)
  }
  saveTimer.value = setTimeout(() => {
    saveSelectedTargets([...selectedTargets.value])
    saveTimer.value = null
  }, 250)
}

const handleTargetSelectionChange = (isChecked, providerId, modelId) => {
  if (isChecked) {
    if (!isTargetSelected(providerId, modelId)) {
      selectedTargets.value.push({ id: providerId, model: modelId })
    }
  } else {
    if (selectedTargets.value.length <= 1) {
      showMessage(t('settings.proxy.validation.targetsRequired'), 'warning')
      return
    }
    selectedTargets.value = selectedTargets.value.filter(
      target => !(target.id === providerId && target.model === modelId)
    )
  }
  queueSaveSelectedTargets()
}

const areAllModelsFromProviderSelected = provider => {
  if (!provider.models || provider.models.length === 0) return false
  return provider.models.every(model => isTargetSelected(provider.id, model.id))
}

const isAnyModelFromProviderSelected = provider => {
  if (!provider.models || provider.models.length === 0) return false
  return provider.models.some(model => isTargetSelected(provider.id, model.id))
}

const handleSelectAllModelsFromProvider = (provider, checked) => {
  if (!checked) {
    const nextTargets = selectedTargets.value.filter(
      target =>
        target.id !== provider.id ||
        !(provider.models || []).some(model => model.id === target.model)
    )
    if (nextTargets.length === 0) {
      showMessage(t('settings.proxy.validation.targetsRequired'), 'warning')
      return
    }
    selectedTargets.value = nextTargets
    queueSaveSelectedTargets()
    return
  }

  ;(provider.models || []).forEach(model => {
    if (!isTargetSelected(provider.id, model.id)) {
      selectedTargets.value.push({ id: provider.id, model: model.id })
    }
  })
  queueSaveSelectedTargets()
}

const ensureVisible = () => {
  nextTick(() => {
    const focusedItem = listRef.value?.querySelector('.proxy-item.focused')
    if (focusedItem) {
      focusedItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
    }
  })
}

const getToolCompatModeIcon = mode => {
  switch (mode) {
    case 'compat':
      return 'xml'
    case 'native':
      return 'hammer'
    default:
      return 'setting'
  }
}

const handleToggleToolCompatMode = async group => {
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
    showMessage(
      t('settings.proxyGroup.toolCompatModeChanged', {
        mode: t(`settings.proxyGroup.toolCompatModes.${newMode}`)
      }),
      'success'
    )
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: formatError(error) }), 'error')
  }
}

watch(
  () => sortedProxyServerGroups.value,
  groups => {
    if (!expandedServerGroup.value && groups.length > 0) {
      expandedServerGroup.value = groups[0].name
    }
  },
  { immediate: true }
)

watch(
  () => modelStore.providers,
  providers => {
    if (isEmpty(providers)) {
      modelStore.updateModelStore()
    }
  },
  { immediate: true }
)

onUnmounted(() => {
  if (unlistenFocus) unlistenFocus()
  if (saveTimer.value) clearTimeout(saveTimer.value)
})

onMounted(async () => {
  await proxyGroupStore.getList()
  settingStore.updateSettingStore()
  const activeIdx = sortedProxyGroupList.value.findIndex(
    g => g.name === proxyGroupStore.activeGroup
  )
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

.switcher-tabs {
  flex-shrink: 0;
  background-color: var(--cs-bg-color);

  :deep(.el-tabs__header) {
    margin: 0;
    padding: 0 var(--cs-space-sm);
    border-bottom: 1px solid var(--cs-border-color);
    background-color: var(--cs-bg-color-light);
  }

  :deep(.el-tabs__nav-wrap::after) {
    display: none;
  }

  :deep(.el-tabs__item) {
    height: 34px;
    color: var(--cs-text-color-primary);
    font-size: var(--cs-font-size-sm);
    font-weight: 600;
  }

  :deep(.el-tabs__item.is-active) {
    color: var(--cs-color-primary);
  }

  :deep(.el-tabs__active-bar) {
    background-color: var(--cs-color-primary);
  }
}

.server-switch-panel {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.proxy-service-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--cs-space-sm);
  padding-bottom: var(--cs-space-sm);

  &::-webkit-scrollbar {
    width: 4px;
  }

  &::-webkit-scrollbar-thumb {
    background: var(--cs-border-color);
    border-radius: var(--cs-space-xxs);
  }

  .list {
    margin-top: var(--cs-space-xs);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius);
    overflow: hidden;
    background-color: var(--cs-bg-color-light);

    &:first-child {
      margin-top: 0;
    }
  }

  .title.group-title {
    min-height: 48px;
    padding: 0 var(--cs-space-sm);
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--cs-color-primary);
    font-size: var(--cs-font-size-sm);
    font-weight: 600;
    cursor: pointer;
    user-select: none;

    &:not(.active) {
      color: var(--cs-text-color-primary);
    }

    &:hover {
      color: var(--cs-color-primary);
    }

    .arrow {
      opacity: 0.8;
    }
  }

  .group-content {
    padding: 0 var(--cs-space-sm) var(--cs-space-sm);
  }

  .item {
    /* min-height: 56px; */
    padding: var(--cs-space-sm) 0;
    border-top: 1px solid var(--cs-border-color);
    display: flex;
    align-items: center;
    justify-content: space-between;
    cursor: pointer;

    &:last-child {
      padding-bottom: 0;
    }

    &:hover,
    &.active {
      .label-text {
        color: var(--cs-color-primary);
      }
    }
  }

  .label {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--cs-space-sm);
  }

  .label-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-xxs);
    color: var(--cs-text-color-primary);
    font-size: var(--cs-font-size-sm);
    font-weight: 600;

    small {
      overflow: hidden;
      color: var(--cs-text-color-secondary);
      font-size: var(--cs-font-size-xs);
      font-weight: 400;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
  }

  .value {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    margin-left: var(--cs-space-sm);
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

:deep(.proxy-model-drawer) {
  background-color: var(--cs-bg-color);
  border-top-left-radius: var(--cs-border-radius-lg);
  border-top-right-radius: var(--cs-border-radius-lg);
}

.model-selector-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  background-color: var(--cs-bg-color);
}

.model-selector-header {
  min-height: 44px;
  padding: 0 var(--cs-space);
  border-bottom: 1px solid var(--cs-border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  background-color: var(--cs-bg-color-light);
}

.model-selector-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-xxs);

  span {
    overflow: hidden;
    color: var(--cs-text-color-primary);
    font-size: var(--cs-font-size);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  small {
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-xs);
  }
}

.model-selector-toolbar {
  padding: var(--cs-space-sm);
  display: flex;
  align-items: center;
  gap: var(--cs-space-sm);

  .search-input {
    flex: 1;
  }
}

.selected-status {
  padding: 0 var(--cs-space-sm) var(--cs-space-xs);
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);

  strong {
    color: var(--cs-color-primary);
    font-size: var(--cs-font-size);
  }
}

.providers-list {
  flex: 1;
  min-height: 0;
  padding: 0 var(--cs-space-sm) var(--cs-space-sm);
}

.providers-scrollbar {
  height: 100%;
}

.provider-card {
  margin-bottom: var(--cs-space-sm);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius);
  overflow: hidden;
  background-color: var(--cs-bg-color-light);
}

.provider-header {
  padding: var(--cs-space-sm);
  border-bottom: 1px solid var(--cs-border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--cs-space-sm);
}

.provider-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  color: var(--cs-text-color-primary);
  font-size: var(--cs-font-size-sm);
  font-weight: 600;

  span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.provider-logo {
  width: var(--cs-font-size-xl);
  height: var(--cs-font-size-xl);
  border-radius: var(--cs-border-radius-round);
  object-fit: cover;
  flex-shrink: 0;
}

.models-grid {
  padding: var(--cs-space-sm);
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: var(--cs-space-xs);
}

.model-checkbox {
  width: 100%;
  margin-right: 0;
  overflow: hidden;

  :deep(.el-checkbox__label) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.no-models-found {
  padding: var(--cs-space-md) var(--cs-space-sm);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-sm);
  text-align: center;
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
