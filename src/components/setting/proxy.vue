<template>
  <div class="proxy-settings-container">
    <el-tabs v-model="activeTab">
      <el-tab-pane :label="$t('settings.proxy.tabs.servers')" name="servers">
        <!-- proxy list -->
        <div class="card">
          <!-- card title -->
          <div class="title">
            <span>{{ $t('settings.proxy.title') }}</span>
            <el-tooltip :content="$t('settings.proxy.addProxy')" placement="top">
              <span class="icon" @click="openAddDialog">
                <cs name="add" />
              </span>
            </el-tooltip>
          </div>
          <template v-if="chatCompletionProxy">
            <div
              class="list"
              v-for="(groupProxies, groupName) in chatCompletionProxy"
              :key="groupName">
              <div class="title">{{ groupName }}</div>
              <div v-for="(proxyTargets, alias) in groupProxies" :key="alias" class="item">
                <div class="label">
                  <Avatar :size="36" :text="alias" />
                  <div class="label-text">
                    {{ alias }}
                    <small>{{
                      $t('settings.proxy.mapsToModels', { count: proxyTargets.length })
                    }}</small>
                  </div>
                </div>

                <div class="value">
                  <el-tooltip
                    :content="$t('settings.proxy.copyProxyAlias')"
                    placement="top"
                    :hide-after="0"
                    transition="none">
                    <span class="icon" @click="copyModelToClipboard(alias)">
                      <cs name="copy" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('settings.proxy.editProxy')"
                    placement="top"
                    :hide-after="0"
                    transition="none">
                    <span class="icon" @click="openEditDialog(groupName, alias, proxyTargets)">
                      <cs name="edit" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('settings.proxy.deleteProxy')"
                    placement="top"
                    :hide-after="0"
                    transition="none">
                    <span class="icon" @click="handleDeleteProxyConfirmation(groupName, alias)">
                      <cs name="trash" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                </div>
              </div>
            </div>
          </template>
          <div v-else class="list">
            <div class="empty-state">
              {{ $t('settings.proxy.noProxiesFound') }}
              <el-button type="primary" @click="openAddDialog" size="small">
                <cs name="add" />{{ $t('settings.proxy.addNow') }}
              </el-button>
            </div>
          </div>
        </div>
        <!-- Proxy auth key -->
        <div class="card">
          <div class="title">
            <span>{{ $t('settings.proxy.proxyKey.title') }}</span>
            <el-tooltip :content="$t('settings.proxy.proxyKey.addKey')" placement="top">
              <span class="icon" @click="openAddKeyDialog">
                <cs name="add" />
              </span>
            </el-tooltip>
          </div>
          <div class="list">
            <template v-if="proxyKeysList.length > 0">
              <div v-for="(keyItem, index) in proxyKeysList" :key="index" class="item">
                <div class="label">
                  <Avatar :size="36" :text="keyItem.name" />
                  <div class="label-text">
                    {{ keyItem.name }}
                    <small>{{ maskToken(keyItem.token) }}</small>
                  </div>
                </div>
                <div class="value">
                  <el-tooltip
                    :content="$t('settings.proxy.proxyKey.copyKey')"
                    placement="top"
                    :hide-after="0"
                    transition="none">
                    <span class="icon" @click="copyKeyToClipboard(keyItem.token)">
                      <cs name="copy" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('settings.proxy.proxyKey.deleteKey')"
                    placement="top"
                    :hide-after="0"
                    transition="none">
                    <span class="icon" @click="handleDeleteKeyConfirmation(index)">
                      <cs name="trash" size="16px" color="secondary" />
                    </span>
                  </el-tooltip>
                </div>
              </div>
            </template>
            <template v-else>
              <div class="empty-state">
                {{ $t('settings.proxy.proxyKey.noKeysFound') }}
                <el-button type="primary" @click="openAddKeyDialog" size="small">
                  <cs name="add" />{{ $t('settings.proxy.proxyKey.addNow') }}
                </el-button>
              </div>
            </template>
          </div>
        </div>
      </el-tab-pane>

      <el-tab-pane :label="$t('settings.proxy.tabs.groups')" name="groups">
        <ProxyGroup />
      </el-tab-pane>

      <el-tab-pane :label="$t('settings.proxy.tabs.settings')" name="settings">
        <div class="card">
          <div class="title">
            <span>{{ $t('settings.proxy.settings.title') }}</span>
          </div>
          <div class="list">
            <div class="item">
              <div class="label">
                <div class="label-text">
                  {{ $t('settings.proxy.settings.port') }}
                  <small class="important">{{
                    $t('settings.proxy.settings.portChangedRestartRequired')
                  }}</small>
                </div>
              </div>
              <div class="value">
                <el-input-number
                  v-model="settings.chatCompletionProxyPort"
                  :min="1"
                  :max="65535"
                  @change="saveProxySettings('chatCompletionProxyPort')" />
              </div>
            </div>
            <div class="item">
              <div class="label">
                <div class="label-text">
                  {{ $t('settings.proxy.settings.logToFile') }}
                  <el-space>
                    <small>{{ $t('settings.proxy.settings.logToFileNote') }}</small>
                    <a
                      class="small important"
                      href="javascript:"
                      @click="openLogFile"
                      v-if="logFilePath"
                      >{{ $t('settings.proxy.settings.openLogFile') }}</a
                    >
                  </el-space>
                </div>
              </div>
              <div class="value">
                <el-switch
                  v-model="settings.chatCompletionProxyLogToFile"
                  @change="saveProxySettings('chatCompletionProxyLogToFile')" />
              </div>
            </div>
          </div>
        </div>

        <div class="tip">
          <div class="openapi-access">
            <h3>{{ $t('settings.proxy.settings.api.title') }}</h3>
            <el-table :data="genTableData()" stripe class="api-table">
              <el-table-column
                prop="protocol"
                :label="$t('settings.proxy.settings.api.protocol')" />
              <el-table-column prop="group" :label="$t('settings.proxy.settings.api.group')" />
              <el-table-column prop="compat" :label="$t('settings.proxy.settings.api.compat')" />
              <el-table-column
                prop="apiUrl"
                :label="$t('settings.proxy.settings.api.apiUrl')"
                width="400" />
              <el-table-column
                prop="note"
                :label="$t('settings.proxy.settings.api.note')"
                width="300" />
            </el-table>
            <el-text>
              {{ $t('settings.proxy.settings.api.example', { baseUrl: baseUrl }) }}
            </el-text>
          </div>
        </div>
      </el-tab-pane>
    </el-tabs>

    <!-- Dialogs and other elements from the original component -->
    <el-dialog
      v-model="dialogVisible"
      :title="isEditing ? $t('settings.proxy.editTitle') : $t('settings.proxy.addTitle')"
      width="600px"
      align-center
      @closed="resetForm"
      class="proxy-edit-dialog"
      :show-close="false"
      :close-on-click-modal="false"
      :close-on-press-escape="false">
      <div class="form-container">
        <el-form
          :model="currentProxyConfig"
          label-width="auto"
          ref="proxyFormRef"
          style="padding-top: 10px">
          <el-form-item :label="$t('settings.proxy.form.group')" prop="group">
            <el-select v-model="currentProxyConfig.group">
              <el-option :label="$t('settings.proxy.defaultGroup')" value="default" />
              <el-option
                v-for="group in proxyGroupStore.list"
                :key="group.id"
                :label="group.name"
                :value="group.name" />
            </el-select>
          </el-form-item>

          <el-form-item
            :label="$t('settings.proxy.form.aliasName')"
            prop="name"
            :rules="[
              { required: true, message: $t('settings.proxy.validation.aliasRequired') },
              { validator: validateAliasUniqueness, trigger: 'blur' }
            ]">
            <el-input
              v-model="currentProxyConfig.name"
              :placeholder="$t('settings.proxy.form.aliasPlaceholder')" />
          </el-form-item>

          <el-divider>{{ $t('settings.proxy.form.targetModelsTitle') }}</el-divider>

          <div style="display: flex; flex-direction: row; gap: 10px">
            <el-input
              v-model="searchQuery"
              :placeholder="$t('settings.proxy.form.searchModelsPlaceholder')"
              clearable
              class="search-input-dialog">
              <template #prefix>
                <cs name="search" />
              </template>
            </el-input>
            <el-checkbox type="primary" @click="handleFilterByChecked">
              {{ $t('settings.proxy.form.checked') }}
            </el-checkbox>
          </div>

          <div class="providers-list-container">
            <el-scrollbar height="400px">
              <div v-if="filteredProviders.length === 0" class="no-models-found">
                {{ $t('settings.proxy.form.noMatchingModels') }}
              </div>
              <el-card
                v-for="provider in filteredProviders"
                :key="provider.id"
                class="provider-card"
                shadow="never">
                <template #header>
                  <div class="card-header">
                    <div class="provider-title">
                      <img
                        v-if="provider.providerLogo"
                        :src="provider.providerLogo"
                        class="provider-logo-small"
                        alt="logo" />
                      <avatar
                        v-else
                        :text="provider.name"
                        :size="20"
                        class="provider-avatar-small" />
                      <span>{{ provider.name }}</span>
                    </div>

                    <el-checkbox
                      :model-value="areAllModelsFromProviderSelected(provider)"
                      :indeterminate="
                        isAnyModelFromProviderSelected(provider) &&
                        !areAllModelsFromProviderSelected(provider)
                      "
                      @change="checked => handleSelectAllModelsFromProvider(provider, checked)">
                      {{ $t('settings.proxy.form.selectAll') }}</el-checkbox
                    >
                  </div>
                </template>
                <div class="models-grid">
                  <template v-for="model in provider.models" :key="model.id">
                    <el-checkbox
                      :model-value="isTargetSelected(provider.id, model.id)"
                      @change="
                        checked => handleTargetSelectionChange(checked, provider.id, model.id)
                      "
                      :label="`${model.id}`"
                      border
                      class="model-checkbox">
                      {{ model.id }}
                    </el-checkbox>
                  </template>
                </div>
              </el-card>
            </el-scrollbar>
          </div>
        </el-form>
      </div>
      <template #footer>
        <div class="dialog-footer-wrap">
          <el-form-item :label="$t('settings.proxy.form.selectedCount')">
            <span>{{ currentProxyConfig.targets.length }}</span>
          </el-form-item>
          <span class="dialog-footer">
            <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
            <el-button type="primary" @click="handleProxyConfigSubmit" :loading="formLoading">
              {{ $t('common.confirm') }}
            </el-button>
          </span>
        </div>
      </template>
    </el-dialog>

    <!-- Key Management and other elements -->
    <el-dialog
      v-model="keyDialogVisible"
      :title="$t('settings.proxy.proxyKey.addTitle')"
      width="500px"
      align-center
      @closed="resetKeyForm"
      class="proxy-key-dialog"
      :show-close="false"
      :close-on-click-modal="!keyFormLoading"
      :close-on-press-escape="false">
      <el-form
        :model="currentKeyItem"
        label-width="auto"
        ref="proxyKeyFormRef"
        style="padding-top: 10px">
        <el-form-item
          :label="$t('settings.proxy.proxyKey.form.name')"
          prop="name"
          :rules="[
            { required: true, message: $t('settings.proxy.proxyKey.validation.nameRequired') }
          ]">
          <el-input
            v-model.trim="currentKeyItem.name"
            :placeholder="$t('settings.proxy.proxyKey.form.namePlaceholder')" />
        </el-form-item>
        <!-- Token input removed, will be auto-generated -->
      </el-form>
      <template #footer>
        <span class="dialog-footer">
          <el-button @click="keyDialogVisible = false">{{ $t('common.cancel') }}</el-button>
          <el-button type="primary" @click="handleKeySubmit" :loading="keyFormLoading">
            {{ $t('common.confirm') }}
          </el-button>
        </span>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'
import { openPath } from '@tauri-apps/plugin-opener'

import { useSettingStore } from '@/stores/setting'
import { useModelStore } from '@/stores/model'
import { useProxyGroupStore } from '@/stores/proxy_group'
import {
  ElMessageBox,
  ElScrollbar,
  ElCard,
  ElCheckbox,
  ElDivider,
  ElTabs,
  ElTabPane,
  ElInputNumber
} from 'element-plus'
import { showMessage, isEmpty } from '@/libs/util'
import ProxyGroup from './ProxyGroup.vue'
// import Avatar from '@/components/common/Avatar.vue'

const { t } = useI18n()
const settingStore = useSettingStore()
const modelStore = useModelStore()
const proxyGroupStore = useProxyGroupStore()

const activeTab = ref('servers')
// const chatCompletionProxyPort = ref(settingStore.settings.chatCompletionProxyPort || 11434)
// const chatCompletionProxyLogToFile = ref(settingStore.settings.chatCompletionProxyLogToFile || false)
const { settings, env } = storeToRefs(settingStore)

// Dialog state
const dialogVisible = ref(false)
const isEditing = ref(false)
const formLoading = ref(false)
const proxyFormRef = ref(null)
const editingAliasName = ref('')
const editingGroupName = ref('')
const filterByChecked = ref(false)

const initialProxyFormState = () => ({
  name: '',
  targets: [],
  group: 'default'
})
// Key Management State
const keyDialogVisible = ref(false)
const keyFormLoading = ref(false)
const proxyKeyFormRef = ref(null)
const initialKeyItemState = () => ({ name: '' }) // Token will be auto-generated
const currentKeyItem = ref(initialKeyItemState())
const currentProxyConfig = ref(initialProxyFormState())

// Search query for models in dialog
const searchQuery = ref('')

const baseUrl = computed(() => {
  return (
    env.value.chatCompletionProxy || 'http://127.0.0.1:' + settings.value.chatCompletionProxyPort
  )
})

const chatCompletionProxy = computed(() => settingStore.settings.chatCompletionProxy || {})

const proxyKeysList = computed(() => {
  return settingStore.settings.chatCompletionProxyKeys || []
})

const allProviders = computed(() =>
  modelStore.providers.filter(
    provider =>
      !provider?.disabled &&
      !provider?.baseUrl.includes('127.0.0.1:' + settings.value.chatCompletionProxyPort) &&
      !provider?.baseUrl.includes('localhost:' + settings.value.chatCompletionProxyPort)
  )
)

const filteredProviders = computed(() => {
  const result = []
  let currentProviders = [...allProviders.value]

  if (filterByChecked.value) {
    currentProviders = currentProviders.filter(provider => {
      // Return true if any model within the provider has its 'checked' property set to true
      return (
        provider.models &&
        provider.models.some(model =>
          currentProxyConfig.value.targets.some(config => config.model === model.id)
        )
      )
    })
  }

  if (!searchQuery.value) {
    return currentProviders
  }
  const query = searchQuery.value.toLowerCase()

  currentProviders.forEach(provider => {
    const providerNameMatch = provider.name.toLowerCase().includes(query)
    let matchingModels = []

    if (provider.models) {
      matchingModels = provider.models.filter(
        model => model.name.toLowerCase().includes(query) || model.id.toLowerCase().includes(query)
      )
    }

    if (providerNameMatch) {
      result.push({ ...provider, models: provider.models })
    } else if (matchingModels.length > 0) {
      result.push({ ...provider, models: matchingModels })
    }
  })
  return result
})

// Watch for model store updates to ensure providers are loaded
watch(
  () => modelStore.providers,
  newProviders => {
    if (isEmpty(newProviders)) {
      modelStore.updateModelStore() // Ensure models are loaded if not already
    }
  },
  { immediate: true }
)

const logFilePath = computed(() => {
  return env.value.logDir ? env.value.logDir + '/ccproxy.log' : ''
})

onMounted(() => {
  settingStore.getEnv()
})

const openLogFile = async () => {
  if (logFilePath.value) {
    await openPath(logFilePath.value)
  }
}

// --- Dialog Form Logic ---
const openAddDialog = () => {
  isEditing.value = false
  currentProxyConfig.value = initialProxyFormState()
  editingAliasName.value = ''
  editingGroupName.value = ''
  dialogVisible.value = true
}

const openEditDialog = (groupName, alias, proxyTargets) => {
  isEditing.value = true
  editingAliasName.value = alias
  editingGroupName.value = groupName
  currentProxyConfig.value = {
    name: alias,
    targets: JSON.parse(JSON.stringify(proxyTargets)), // Deep copy
    group: groupName
  }
  dialogVisible.value = true
}

const resetForm = () => {
  currentProxyConfig.value = initialProxyFormState()
  isEditing.value = false
  editingAliasName.value = ''
  editingGroupName.value = ''
  searchQuery.value = ''
  if (proxyFormRef.value) {
    proxyFormRef.value.resetFields()
    currentProxyConfig.value.targets = []
  }
  formLoading.value = false
}

const validateAliasUniqueness = (_rule, value, callback) => {
  if (!value) {
    return callback(new Error(t('settings.proxy.validation.aliasRequired')))
  }
  const groupName = currentProxyConfig.value || 'default'
  // Check uniqueness across all groups
  if (Object.prototype.hasOwnProperty.call(chatCompletionProxy.value, groupName)) {
    const groupProxies = chatCompletionProxy.value[groupName]
    if (Object.keys(groupProxies).includes(value)) {
      // If editing, allow the current alias to be the same
      if (isEditing.value && editingAliasName.value === value) {
        return callback()
      }
      return callback(new Error(t('settings.proxy.validation.aliasUnique')))
    }
  }
  return callback()
}

const isTargetSelected = (providerId, modelId) => {
  return currentProxyConfig.value.targets.some(
    target => target.id === providerId && target.model === modelId
  )
}

const handleTargetSelectionChange = (isChecked, providerId, modelId) => {
  if (isChecked) {
    if (!isTargetSelected(providerId, modelId)) {
      currentProxyConfig.value.targets.push({ id: providerId, model: modelId })
    }
  } else {
    currentProxyConfig.value.targets = currentProxyConfig.value.targets.filter(
      target => !(target.id === providerId && target.model === modelId)
    )
  }
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
  provider.models.forEach(model => {
    handleTargetSelectionChange(checked, provider.id, model.id)
  })
}

const handleFilterByChecked = () => {
  filterByChecked.value = !filterByChecked.value
}

const handleProxyConfigSubmit = async () => {
  if (!proxyFormRef.value) return
  await proxyFormRef.value.validate(async valid => {
    if (valid) {
      if (currentProxyConfig.value.targets.length === 0) {
        showMessage(t('settings.proxy.validation.targetsRequired'), 'warning')
        return
      }
      formLoading.value = true
      try {
        const newProxies = { ...chatCompletionProxy.value }

        // If editing and alias or group changed, remove old entry
        if (isEditing.value && editingAliasName.value) {
          // Check if the alias or group has changed
          const oldGroup = editingGroupName.value
          const oldAlias = editingAliasName.value

          if (
            oldGroup !== currentProxyConfig.value.group ||
            oldAlias !== currentProxyConfig.value.name
          ) {
            if (newProxies[oldGroup] && newProxies[oldGroup][oldAlias]) {
              delete newProxies[oldGroup][oldAlias]
              // If the group becomes empty, delete the group
              if (Object.keys(newProxies[oldGroup]).length === 0) {
                delete newProxies[oldGroup]
              }
            }
          }
        }

        // Ensure the target group exists
        if (!newProxies[currentProxyConfig.value.group]) {
          newProxies[currentProxyConfig.value.group] = {}
        }

        // Add or update the proxy in the new structure
        newProxies[currentProxyConfig.value.group][currentProxyConfig.value.name] =
          currentProxyConfig.value.targets

        await settingStore.setSetting('chatCompletionProxy', newProxies)
        showMessage(
          isEditing.value ? t('settings.proxy.updateSuccess') : t('settings.proxy.addSuccess'),
          'success'
        )
        dialogVisible.value = false
      } catch (error) {
        console.error('Failed to save proxy config:', error)
        showMessage(t('settings.proxy.saveFailed', { error: error.message || error }), 'error')
      } finally {
        formLoading.value = false
      }
    }
  })
}

const handleDeleteProxyConfirmation = (groupName, alias) => {
  ElMessageBox.confirm(
    t('settings.proxy.deleteConfirmText', { alias }),
    t('settings.proxy.deleteConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  )
    .then(async () => {
      await handleDeleteProxy(groupName, alias)
    })
    .catch(() => {})
}

const handleDeleteProxy = async (groupName, aliasToDelete) => {
  try {
    const newProxies = { ...chatCompletionProxy.value }
    if (newProxies[groupName] && newProxies[groupName][aliasToDelete]) {
      delete newProxies[groupName][aliasToDelete]
      // If the group becomes empty after deletion, remove the group
      if (Object.keys(newProxies[groupName]).length === 0) {
        delete newProxies[groupName]
      }
    }
    await settingStore.setSetting('chatCompletionProxy', newProxies)
    showMessage(t('settings.proxy.deleteSuccess'), 'success')
  } catch (error) {
    console.error('Failed to delete proxy config:', error)
    showMessage(t('settings.proxy.deleteFailed', { error: error.message || error }), 'error')
  }
}

// --- Key Management Logic ---
const copyKeyToClipboard = async token => {
  try {
    await navigator.clipboard.writeText(token)
    showMessage(t('settings.proxy.proxyKey.copySuccess'), 'success')
  } catch (err) {
    console.error('Failed to copy key: ', err)
    showMessage(t('settings.proxy.proxyKey.copyFailed', { error: err.message }), 'error')
  }
}

const maskToken = token => {
  if (!token || token.length < 8) return '********'
  return `${token.substring(0, 10)}******${token.substring(token.length - 10)}`
}

const openAddKeyDialog = () => {
  currentKeyItem.value = initialKeyItemState()
  keyDialogVisible.value = true
}

const resetKeyForm = () => {
  currentKeyItem.value = initialKeyItemState()
  if (proxyKeyFormRef.value) {
    proxyKeyFormRef.value.resetFields()
  }
  keyFormLoading.value = false
}

const handleKeySubmit = async () => {
  if (!proxyKeyFormRef.value) return

  // Auto-generate token before validation/submission
  const characters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'
  let generatedToken = 'cs-'
  for (let i = 0; i < 61; i++) {
    // 64 total - 3 for "cs-"
    generatedToken += characters.charAt(Math.floor(Math.random() * characters.length))
  }

  await proxyKeyFormRef.value.validate(async valid => {
    if (valid) {
      keyFormLoading.value = true
      try {
        const updatedKeys = [...proxyKeysList.value]
        updatedKeys.push({ name: currentKeyItem.value.name, token: generatedToken })
        await settingStore.setSetting('chatCompletionProxyKeys', updatedKeys)
        showMessage(t('settings.proxy.proxyKey.addSuccess'), 'success')
        keyDialogVisible.value = false
      } catch (error) {
        console.error('Failed to save proxy key:', error)
        showMessage(
          t('settings.proxy.proxyKey.saveFailed', { error: error.message || error }),
          'error'
        )
      } finally {
        keyFormLoading.value = false
      }
    }
  })
}

const handleDeleteKeyConfirmation = index => {
  ElMessageBox.confirm(
    t('settings.proxy.proxyKey.deleteConfirmText', { name: proxyKeysList.value[index].name }),
    t('settings.proxy.proxyKey.deleteConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  )
    .then(async () => {
      await handleDeleteKey(index)
    })
    .catch(() => {})
}

const handleDeleteKey = async indexToDelete => {
  try {
    const updatedKeys = proxyKeysList.value.filter((_, index) => index !== indexToDelete)
    await settingStore.setSetting('chatCompletionProxyKeys', updatedKeys)
    showMessage(t('settings.proxy.proxyKey.deleteSuccess'), 'success')
  } catch (error) {
    console.error('Failed to delete proxy key:', error)
    showMessage(
      t('settings.proxy.proxyKey.deleteFailed', { error: error.message || error }),
      'error'
    )
  }
}

// =================================================
// Copy
// =================================================

const saveProxySettings = async key => {
  try {
    const val =
      key === 'chatCompletionProxyPort'
        ? settings.value.chatCompletionProxyPort
        : settings.value.chatCompletionProxyLogToFile
    await settingStore.setSetting(key, val)
    showMessage(t('settings.proxy.settings.saveSuccess'), 'success')
  } catch (error) {
    showMessage(t('settings.proxy.settings.saveFailed', { error: error.message || error }), 'error')
  }
}

const copyModelToClipboard = async model => {
  try {
    await navigator.clipboard.writeText(model)
    showMessage(t('settings.proxy.modelCopySuccess'), 'success')
  } catch (err) {
    console.error('Failed to copy key: ', err)
    showMessage(t('settings.proxy.modelCopyFailed', { error: err.message }), 'error')
  }
}
const copyBaseUrlToClipboard = async () => {
  try {
    await navigator.clipboard.writeText(baseUrl.value || '')
    showMessage(t('settings.proxy.baseUrlCopySuccess'), 'success')
  } catch (err) {
    console.error('Failed to copy key: ', err)
    showMessage(t('settings.proxy.baseUrlCopyFailed', { error: err.message }), 'error')
  }
}
const genTableData = () => {
  return [
    {
      protocol: 'Openai',
      group: '',
      compat: 'false',
      apiUrl: '/v1/chat/completions',
      note: ''
    },
    {
      protocol: 'Openai',
      group: '{group}',
      compat: 'false',
      apiUrl: '/{group}/v1/chat/completions',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Openai',
      group: '{group}',
      compat: 'true',
      apiUrl: '/{group}/compat_mode/v1/chat/completions',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Openai',
      group: '',
      compat: 'true',
      apiUrl: '/compat_mode/v1/chat/completions',
      note: ''
    },
    {
      protocol: 'Claude',
      group: '',
      compat: 'false',
      apiUrl: '/v1/messages',
      note: ''
    },
    {
      protocol: 'Claude',
      group: '{group}',
      compat: 'false',
      apiUrl: '/{group}/v1/messages',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Claude',
      group: '{group}',
      compat: 'true',
      apiUrl: '/{group}/compat_mode/v1/messages',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Claude',
      group: '',
      compat: 'true',
      apiUrl: '/compat_mode/v1/messages',
      note: ''
    },
    {
      protocol: 'Gemini',
      group: '',
      compat: 'false',
      apiUrl: '/v1beta/models/{model}/generateContent?key={key}',
      note: '将{model}替换为模型名称，将{key}替换为API Key'
    },
    {
      protocol: 'Gemini',
      group: '{group}',
      compat: 'false',
      apiUrl: '/{group}/v1beta/models/{model}/generateContent?key={key}',
      note: '将{group}替换为分组名称，将{model}替换为模型名称，将{key}替换为API Key'
    },
    {
      protocol: 'Gemini',
      group: '{group}',
      compat: 'true',
      apiUrl: '/{group}/compat_mode/v1beta/models/{model}/generateContent?key={key}',
      note: '将{group}替换为分组名称，将{model}替换为模型名称，将{key}替换为API Key'
    },
    {
      protocol: 'Gemini',
      group: '',
      compat: 'true',
      apiUrl: '/compat_mode/v1beta/models/{model}/generateContent?key={key}',
      note: '将{model}替换为模型名称，将{key}替换为API Key'
    },
    {
      protocol: 'Ollama',
      group: '',
      compat: 'false',
      apiUrl: '/api/chat',
      note: ''
    },
    {
      protocol: 'Ollama',
      group: '{group}',
      compat: 'false',
      apiUrl: '/{group}/api/chat',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Ollama',
      group: '{group}',
      compat: 'true',
      apiUrl: '/{group}/compat_mode/api/chat',
      note: '将{group}替换为分组名称'
    },
    {
      protocol: 'Ollama',
      group: '',
      compat: 'true',
      apiUrl: '/compat_mode/api/chat',
      note: ''
    }
  ]
}
</script>

<style lang="scss" scoped>
.proxy-settings-container {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-lg);
}

.card {
  // This is a general .card style from your global styles or mcp.vue
  // We might need to adjust padding if it's too much for list items
  // or if el-card inside dialog adds its own.
  :deep(.el-card__body) {
    // For el-card used within the dialog's model list
    padding: var(--cs-space-sm) var(--cs-space-md);
  }
}

// Styles for the list items, reusing from global or mcp.vue if possible
// .list is defined in global styles
// .item is defined in global styles

.label-text {
  // Specific to this component's list item structure
  display: flex;
  flex-direction: column;
  gap: 2px; // Small gap between alias name and target count
  font-weight: 500;
  color: var(--cs-text-color);

  small {
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-xs);
  }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--cs-space-sm);
  color: var(--cs-text-color-secondary);
  margin: var(--cs-space-lg) auto;
  text-align: center;

  .el-button {
    // For "Add Now" button
    display: flex;
    align-items: center;
    gap: var(--cs-space-xxs);

    .cs {
      font-size: 1.1em; // Make icon slightly larger than text
    }
  }
}

.form-container {
  max-height: calc(70vh - 120px);
}

.search-input-dialog {
  margin-bottom: var(--cs-space-md);
}

.providers-list-container {
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-sm);
  margin-bottom: var(--cs-space-md);
}

.no-models-found {
  text-align: center;
  color: var(--cs-text-color-placeholder);
  padding: var(--cs-space-lg);
}

.provider-card {
  margin-bottom: var(--cs-space-sm);
  background-color: var(--cs-primary-bg-color);
  border: 1px solid var(--cs-border-color-light);

  &:last-child {
    margin-bottom: 0;
  }

  :deep(.el-card__header) {
    padding: var(--cs-space-sm) var(--cs-space-md);
    background-color: var(--cs-secondary-bg-color);
  }

  :deep(.el-card__body) {
    padding: var(--cs-space-sm) var(--cs-space-md);
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
  }

  .provider-title {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    font-weight: 500;
  }

  .provider-logo-small {
    width: 20px;
    height: 20px;
    object-fit: contain;
  }

  .provider-avatar-small {
    font-size: 10px;
  }
}

.models-grid {
  display: flex;
  flex-wrap: wrap;
  gap: var(--cs-space-xs);
  padding-top: var(--cs-space-xs);
}

.model-checkbox {
  margin-right: var(--cs-space-xs) !important;
  margin-bottom: var(--cs-space-xs);
  padding: var(--cs-space-xxs) var(--cs-space-sm) !important;

  :deep(.el-checkbox__label) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-left: var(--cs-space-xxs);
  }
}

.proxy-edit-dialog {
  :deep(.el-dialog__body) {
    padding-top: 0px;
    padding-bottom: 0px;
  }

  :deep(.el-dialog__footer) {
    padding-top: var(--cs-space-sm);
  }

  :deep(.el-divider__text) {
    font-size: var(--cs-font-size-sm);
    color: var(--cs-text-color-secondary);
  }

  .dialog-footer-wrap {
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
  }
}

.tip {
  font-size: var(--cs-font-size);
  margin: var(--cs-space-lg) 0 var(--cs-space-lg);
  // padding: var(--cs-space);
  // background-color: var(--cs-bg-color-deep);
  border-radius: var(--cs-border-radius);

  ul > li {
    font-size: var(--cs-font-size-md);
    color: var(--el-text-color-primary);
    line-height: 2;
  }

  .api-table {
    width: 100%;
    margin-bottom: var(--cs-space);
  }
}
</style>
