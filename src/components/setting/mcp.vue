<template>
  <div class="card">
    <!-- 顶部标题和添加按钮 -->
    <div class="title">
      <span>{{ $t('settings.mcp.title') }}</span>
      <el-tooltip :content="$t('settings.mcp.addServer')" placement="left" :hide-after="0">
        <span class="icon" @click="showPresetMcpDialog">
          <cs name="add" />
        </span>
      </el-tooltip>
    </div>

    <!-- 服务器列表/空状态 -->
    <div class="list">
      <template v-if="mcpStore.servers.length > 0">
        <div v-for="server in mcpStore.servers" :key="server.id" class="item-wrapper">
          <div class="item">
            <div class="server-left">
              <div class="expand-btn" @click="toggleServerToolsExpansion(server)">
                <cs :class="server.status" :name="mcpStore.getOrInitServerUiState(server.id).expanded &&
                    server.status === 'running'
                    ? 'caret-down'
                    : 'caret-right'
                  " />
              </div>
              <avatar :text="server.name" :size="32" />
              <div class="server-info">
                <div class="server-name">{{ server.name }}</div>
                <div class="server-status" :class="getServerStatusClass(server.status)">
                  <span class="status-dot"></span>
                  <span class="status-text">{{ getServerStatusText(server.status) }}</span>
                </div>
              </div>
            </div>

            <div class="value">
              <el-tooltip :content="$t('settings.mcp.' + (server.disabled ? 'enable' : 'disable') + 'Server')"
                placement="top" :hide-after="0" transition="none">
                <el-switch :disabled="mcpStore.getOrInitServerUiState(server.id).loading"
                  :model-value="!server.disabled" :loading="mcpStore.getOrInitServerUiState(server.id).loading"
                  @update:model-value="toggleServerStatus(server)" />
              </el-tooltip>

              <el-tooltip :content="$t('settings.mcp.restart')" placement="top" :hide-after="0" transition="none"
                :disabled="server.disabled">
                <span class="icon" :class="{ disabled: server.disabled }" @click="restartMcpServer(server)">
                  <cs name="restart" size="16px" color="secondary" />
                </span>
              </el-tooltip>

              <el-tooltip :content="$t('settings.mcp.editServer')" placement="top" :hide-after="0" transition="none">
                <span class="icon" @click="openEditDialog(server)">
                  <cs name="edit" size="16px" color="secondary" />
                </span>
              </el-tooltip>
              <el-tooltip :content="$t('settings.mcp.deleteServer')" placement="top" :hide-after="0" transition="none">
                <span class="icon" @click="handleDeleteServerConfirmation(server)">
                  <cs name="trash" size="16px" color="secondary" />
                </span>
              </el-tooltip>
            </div>
          </div>

          <!-- expandable tools list -->
          <div v-if="
            mcpStore.getOrInitServerUiState(server.id).expanded && server.status === 'running'
          " class="tools-list">
            <div v-if="mcpStore.getOrInitServerUiState(server.id).loading" class="tool-loading">
              {{ $t('common.loading') }}...
            </div>
            <ul v-if="mcpStore.serverTools[server.id] && mcpStore.serverTools[server.id].length">
              <li v-for="tool in mcpStore.serverTools[server.id] || []" :key="tool.name" class="tool-item">
                <div class="tool-info">
                  <div class="tool-name">{{ tool.name }}</div>
                  <div class="tool-description">{{ tool.description }}</div>
                </div>
                <div class="tool-actions">
                  <el-switch size="small" :model-value="!(server?.config?.disabled_tools || []).includes(tool.name)"
                    @update:model-value="toggleDisableTool(server.id, tool)" />
                </div>
              </li>
            </ul>
            <div v-else-if="
              !mcpStore.getOrInitServerUiState(server.id).loading &&
              mcpStore.serverTools[server.id]
            ">
              {{ $t('settings.mcp.noTools') }}
            </div>
          </div>
        </div>
      </template>
      <template v-else>
        <div class="empty-state">
          <p>
            {{ $t('settings.mcp.noServersFound') }}
            <!-- Assuming a key like noServersFound -->
            <button @click="showPresetMcpDialog">
              <cs name="add" />{{ $t('settings.mcp.addNow') }}
            </button>
          </p>
        </div>
      </template>
    </div>

    <!-- 添加/编辑对话框 -->
    <el-dialog v-model="dialogVisible" width="600px" align-center @closed="resetForm" class="model-edit-dialog"
      :show-close="false" :close-on-click-modal="false" :close-on-press-escape="false">
      <div class="form-container">
        <el-tabs v-model="activeTabName">
          <!-- Ensure activeTabName is used here if defined in script -->
          <el-tab-pane :label="$t('settings.mcp.form.tabForm')" name="formEditor">
            <el-form :model="currentServerForm" label-width="150px" ref="serverFormRef" style="padding-top: 20px">
              <el-form-item :label="$t('settings.mcp.form.name')" prop="name" :rules="[
                { required: true, message: $t('settings.mcp.validation.nameRequired') },
                {
                  validator: (rule, value, callback) => {
                    const regex = /^[a-zA-Z_$][a-zA-Z0-9_$]*$/
                    if (!value || regex.test(value)) {
                      callback()
                    } else {
                      callback(
                        new Error(t('settings.mcp.validation.invalidName', { name: value }))
                      )
                    }
                  },
                  trigger: 'blur'
                }
              ]">
                <el-input v-model="currentServerForm.name" />
              </el-form-item>
              <el-form-item :label="$t('settings.mcp.form.description')" prop="description">
                <el-input v-model="currentServerForm.description" type="textarea" />
              </el-form-item>
              <el-form-item :label="$t('settings.mcp.form.disabled')" prop="disabled">
                <el-switch v-model="currentServerForm.disabled" />
              </el-form-item>

              <el-form-item :label="$t('settings.mcp.form.type')" prop="config.type"
                :rules="[{ required: true, message: $t('settings.mcp.validation.typeRequired') }]">
                <el-radio-group v-model="currentServerForm.config.type">
                  <el-radio value="stdio">Stdio</el-radio>
                  <el-radio value="sse">SSE</el-radio>
                  <el-radio value="streamable_http">Streamable HTTP</el-radio>
                </el-radio-group>
              </el-form-item>

              <template v-if="currentServerForm.config.type === 'stdio'">
                <el-form-item :label="$t('settings.mcp.form.command')" prop="config.command" :rules="[
                  {
                    required: true,
                    message: $t('settings.mcp.validation.commandRequired')
                  }
                ]">
                  <el-input v-model="currentServerForm.config.command" />
                </el-form-item>
                <el-form-item :label="$t('settings.mcp.form.args')" prop="config.argsString" :rules="[
                  {
                    required: true,
                    message: $t('settings.mcp.validation.argsRequired')
                  }
                ]">
                  <el-input v-model="currentServerForm.config.argsString" type="textarea" :rows="1"
                    :autosize="{ minRows: 1, maxRows: 3 }" :placeholder="$t('settings.mcp.form.argsPlaceholder')" />
                </el-form-item>
                <el-form-item :label="$t('settings.mcp.form.env')" prop="config.envString">
                  <el-input v-model="currentServerForm.config.envString" type="textarea" :rows="2"
                    :autosize="{ minRows: 1, maxRows: 5 }" :placeholder="$t('settings.mcp.form.envPlaceholder')" />
                </el-form-item>
              </template>

              <template v-if="currentServerForm.config.type !== 'stdio'">
                <el-form-item :label="$t('settings.mcp.form.url')" prop="config.url" :rules="[
                  {
                    required: true,
                    message: $t('settings.mcp.validation.urlRequired')
                  }
                ]">
                  <el-input v-model="currentServerForm.config.url" />
                </el-form-item>
                <el-form-item :label="$t('settings.mcp.form.bearerToken')" prop="config.bearer_token">
                  <el-input v-model="currentServerForm.config.bearer_token" />
                </el-form-item>
                <el-form-item :label="$t('settings.general.proxyServer')" prop="config.proxy">
                  <el-input v-model="currentServerForm.config.proxy" />
                </el-form-item>
              </template>
              <el-form-item :label="$t('settings.mcp.form.timeout')" prop="config.timeout">
                <el-input v-model="currentServerForm.config.timeout" type="number"
                  :placeholder="$t('settings.mcp.form.timeoutPlaceholder')" />
              </el-form-item>
            </el-form>
          </el-tab-pane>

          <el-tab-pane :label="$t('settings.mcp.form.tabJson')" name="jsonEditor">
            <textarea v-model="jsonConfigString" class="json-editor"></textarea>
          </el-tab-pane>
        </el-tabs>
      </div>
      <template #footer>
        <span class="dialog-footer">
          <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
          <el-button type="primary" @click="handleSubmit" :loading="formLoading">
            {{ $t('common.confirm') }}
          </el-button>
        </span>
      </template>
    </el-dialog>

    <!-- Preset MCPs Dialog -->
    <el-dialog v-model="presetMcpsVisible" :title="$t('settings.mcp.presetTitle')" width="600px" align-center
      class="preset-mcps-dialog">
      <div class="preset-mcps-container">
        <div class="search-bar">
          <el-row :gutter="10">
            <el-col :span="16">
              <el-input v-model="presetSearchQuery" :placeholder="$t('common.search')" clearable class="search-input" />
            </el-col>
            <el-col :span="8">
              <el-button type="primary" plain @click="handleManualAddFromPresetDialog" style="width: 100%">
                <cs name="add" /> {{ $t('settings.mcp.addManually') }}
              </el-button>
            </el-col>
          </el-row>
        </div>

        <div class="preset-mcps-list" v-loading="loadingPresets">
          <el-card v-for="preset in filteredPresetMcps" :key="preset.name" class="preset-mcp-card" shadow="hover">
            <div class="mcp-item">
              <avatar :text="preset.name" :size="40" class="mcp-logo" />
              <div class="mcp-details">
                <h3>{{ preset.name }}</h3>
                <p>{{ preset.description }}</p>
              </div>
              <el-space direction="vertical">
                <el-button type="success" @click="importPresetMcp(preset)" :disabled="isPresetAdded(preset.name)">
                  {{
                    isPresetAdded(preset.name)
                      ? $t('settings.mcp.added')
                      : $t('settings.mcp.addFromPreset')
                  }}
                </el-button>
                <el-button size="small" round @click="openUrl(preset.website)" v-if="preset.website">{{
                  $t('common.detail')
                  }}</el-button>
              </el-space>
            </div>
          </el-card>
          <div v-if="!loadingPresets && filteredPresetMcps.length === 0" class="empty-state">
            {{ $t('settings.mcp.noPresetsFound') }}
          </div>
        </div>
      </div>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, watch, reactive, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { useMcpStore } from '@/stores/mcp'
import { openUrl } from '@/libs/util'
import { ElMessageBox, ElTooltip } from 'element-plus'

import { showMessage, showMessageBox } from '@/libs/util'

const { t } = useI18n()
const mcpStore = useMcpStore()

const serverFormRef = ref(null)
const activeTabName = ref('formEditor') // Changed from activeTab to activeTabName

// 对话框状态
const dialogVisible = ref(false)
const formLoading = ref(false)
const isEditMode = ref(false)

const initialServerFormState = () => ({
  id: null,
  name: '',
  description: '',
  disabled: false,
  disabled_toolsString: '',
  config: {
    name: '',
    type: 'stdio',
    url: null,
    bearer_token: null,
    proxy: null,
    command: null,
    args: [],
    argsString: '',
    env: [],
    envString: '',
    disabled_tools: [],
    disabled_toolsString: '',
    timeout: null
  }
})

const currentServerForm = reactive(initialServerFormState())
const jsonConfigString = ref('')

const presetMcpsVisible = ref(false)
const presetMcps = ref([])
const presetSearchQuery = ref('')
const loadingPresets = ref(false)

const serverToOperateOn = ref(null) // For delete or other confirmations

// =================================================
// Coumputed Properties
// =================================================
watch(
  currentServerForm,
  newForm => {
    if (activeTabName.value === 'formEditor') {
      // Changed from activeTab
      try {
        const payload = preparePayloadFromForm(newForm)
        jsonConfigString.value = JSON.stringify(payload, null, 2)
      } catch (e) {
        console.warn('Error generating JSON from form for preview:', e)
      }
    }
  },
  { deep: true }
)

watch(jsonConfigString, newJson => {
  if (activeTabName.value === 'jsonEditor') {
    // Changed from activeTab
    try {
      const parsedPayload = JSON.parse(newJson)
      if (
        parsedPayload &&
        parsedPayload.mcpServers &&
        typeof parsedPayload.mcpServers === 'object'
      ) {
        const serverNames = Object.keys(parsedPayload.mcpServers)
        if (serverNames.length > 0) {
          const serverNameFromFile = serverNames[0] // Assume the first key is the server name
          const serverConfigFromFile = parsedPayload.mcpServers[serverNameFromFile]

          if (serverConfigFromFile && typeof serverConfigFromFile.type === 'string') {
            // Update currentServerForm.name
            currentServerForm.name = serverNameFromFile
            // Note: description, disabled, and top-level disabled_tools are not in jsonConfigString,
            // so they retain their values from the form editor or defaults.

            // Update currentServerForm.config
            currentServerForm.config.name = serverNameFromFile // Config name usually matches server name
            currentServerForm.config.type = serverConfigFromFile.type

            if (serverConfigFromFile.type === 'stdio') {
              currentServerForm.config.command = serverConfigFromFile.command || null
              currentServerForm.config.args = serverConfigFromFile.args || []
              currentServerForm.config.argsString = arrayToString(currentServerForm.config.args)
              // Clear SSE specific fields
              currentServerForm.config.url = null
              currentServerForm.config.bearer_token = null
              currentServerForm.config.proxy = null
              currentServerForm.config.env = serverConfigFromFile.env || []
            } else {
              currentServerForm.config.url = serverConfigFromFile.url || null
              currentServerForm.config.bearer_token = serverConfigFromFile.bearer_token || null
              currentServerForm.config.proxy = serverConfigFromFile.proxy || null
              // Clear stdio specific fields
              currentServerForm.config.command = null
              currentServerForm.config.args = []
              currentServerForm.config.argsString = ''
              currentServerForm.config.env = []
            }

            currentServerForm.config.envString = arrayToEnvString(currentServerForm.config.env)

            currentServerForm.config.disabled_tools = serverConfigFromFile.disabled_tools || []
            currentServerForm.config.disabled_toolsString = arrayToString(
              currentServerForm.config.disabled_tools
            )
          }
        }
      }
    } catch (e) { }
  }
})

// =================================================
// Form and Dialog Logic
// =================================================
const resetForm = () => {
  // Set activeTabName to 'formEditor' first to prevent the jsonConfigString watcher
  // from incorrectly updating currentServerForm if the tab was previously 'jsonEditor'.
  activeTabName.value = 'formEditor' // Changed from activeTab

  Object.assign(currentServerForm, initialServerFormState())

  // Update jsonConfigString to be consistent with the reset form's payload structure.
  // This ensures that if the user switches to the JSON tab, they see the correct empty/initial payload.
  // The watcher for currentServerForm will also update jsonConfigString, but setting it
  jsonConfigString.value = JSON.stringify(preparePayloadFromForm(currentServerForm), null, 2)

  if (serverFormRef.value) {
    serverFormRef.value.resetFields()
  }
  // Note: resetFields() might not be effective if called when the form is not yet visible/mounted.
  // The main call to resetFields() for opening a new/edit dialog is now in openServerFormDialog using nextTick.
}

const prepareFormForDisplay = serverData => {
  const form = JSON.parse(JSON.stringify(serverData)) // Deep clone
  form.config.argsString = arrayToString(form.config.args)
  form.config.envString = arrayToEnvString(form.config.env)
  form.disabled_toolsString = arrayToString(form.disabled_tools)
  form.config.disabled_toolsString = arrayToString(form.config.disabled_tools)
  return form
}

const preparePayloadFromForm = form => {
  const serverNameKey = (form.name || '').trim()
  if (!serverNameKey) {
    return { mcpServers: {} }
  }

  const serverConfigData = {}

  serverConfigData.type = form.config.type

  if (form.config.type === 'stdio') {
    serverConfigData.command = (form.config.command || '').trim()
    serverConfigData.args = parseStringToArray(form.config.argsString || '')
  } else {
    serverConfigData.url = (form.config.url || '').trim()
    if (form.config.bearer_token && form.config.bearer_token.trim()) {
      serverConfigData.bearer_token = form.config.bearer_token.trim()
    }
    if (form.config.proxy && form.config.proxy.trim()) {
      serverConfigData.proxy = form.config.proxy.trim()
    }
  }

  const envArray = parseEnvString(form.config.envString || '')
  if (envArray.length > 0) {
    serverConfigData.env = envArray
  }

  // disabled_tools from the config part of the form
  const configDisabledTools = parseStringToArray(form.config.disabled_toolsString || '')
  if (configDisabledTools.length > 0) {
    serverConfigData.disabled_tools = configDisabledTools
  }
  serverConfigData.timeout = form.config.timeout || null
  return { mcpServers: { [serverNameKey]: serverConfigData } }
}

const openServerFormDialog = (initialData = null) => {
  // 1. Clear previous validation states from the form if the ref exists.
  // Do this early, before data model changes might trigger premature validation or display old messages.
  if (serverFormRef.value) {
    serverFormRef.value.clearValidate()
  }

  activeTabName.value = 'formEditor' // Default to form editor
  // Watcher for name -> config.name sync will be handled specifically for add mode

  if (initialData) {
    // Editing existing server or populating from a preset
    isEditMode.value = !!initialData.id // True edit mode if initialData has an ID (i.e., existing server)

    // Construct the data structure expected by the form
    const serverDataForForm = {
      id: initialData.id || null,
      name: initialData.name || '',
      description: initialData.description || '',
      disabled: initialData.disabled || false,
      disabled_tools: initialData.disabled_tools || [],
      config: {
        name: initialData.config?.name || initialData.name || '',
        type: initialData.config?.type || 'stdio',
        url: initialData.config?.url || null,
        bearer_token: initialData.config?.bearer_token || null,
        proxy: initialData.config?.proxy || null,
        command: initialData.config?.command || null,
        args: initialData.config?.args || [],
        env: initialData.config?.env || [],
        disabled_tools: initialData.config?.disabled_tools || [],
        timeout: initialData.config?.timeout || null
      }
    }
    const preparedData = prepareFormForDisplay(serverDataForForm)
    Object.assign(currentServerForm, preparedData) // Populate reactive form data
    jsonConfigString.value = JSON.stringify(preparePayloadFromForm(currentServerForm), null, 2)
    dialogVisible.value = true // Show dialog after data is ready for editing
  } else {
    // Manual Add mode
    isEditMode.value = false

    Object.assign(currentServerForm, initialServerFormState())
    jsonConfigString.value = JSON.stringify(preparePayloadFromForm(currentServerForm), null, 2)

    // Set dialog visible first, then in nextTick, reset data and clear validation.
    // This ensures form elements are available when clearValidate is called.
    dialogVisible.value = true

    nextTick(() => {
      // Crucially, clear any validation messages that might have appeared
      // on initial render with empty data before user interaction.
      if (serverFormRef.value) {
        serverFormRef.value.clearValidate()
      }

      // Auto-fill config.name from server.name for new entries when adding manually.
      // This watcher is specific to manual add mode.
      const unwatchNameForConfigSync = watch(
        () => currentServerForm.name,
        newName => {
          // Only fill if in manual add mode and config.name is still empty
          if (!isEditMode.value && !currentServerForm.config.name) {
            currentServerForm.config.name = newName
          }
        }
      )

      // Watch for dialog visibility to stop the name sync watcher when dialog closes.
      // This prevents the watcher from persisting across dialog openings.
      const stopVisibilityWatcherForNameSync = watch(dialogVisible, isVisible => {
        if (!isVisible) {
          unwatchNameForConfigSync() // Stop the name to config.name watcher
          stopVisibilityWatcherForNameSync() // Stop this visibility watcher itself
        }
      })
    })
  }
}

const openEditDialog = server => {
  // No resetForm() here. openServerFormDialog handles its own state setup.
  openServerFormDialog(server)
}

const handleSubmit = async () => {
  if (!serverFormRef.value) {
    // Changed from activeTab
    showMessage(t('settings.mcp.form.error'), 'error')
    return
  }
  serverFormRef.value.validate(async valid => {
    if (!valid) {
      console.log('Form validation failed')
      showMessage(t('settings.mcp.form.validationFailed'), 'error') // Pass 'error' as type
      return false
    }
  })

  // 检查是否包含占位符
  const placeholderValidation = validatePlaceholders()
  if (!placeholderValidation.isValid) {
    showMessage(placeholderValidation.message, 'error')
    return
  }

  formLoading.value = true

  const payload = preparePayloadFromForm(currentServerForm)
  const formData = {
    name: currentServerForm.name,
    description: currentServerForm.description,
    config: {
      name: currentServerForm.name,
      ...payload['mcpServers'][currentServerForm.name]
    },
    disabled: currentServerForm.disabled
  }
  if (isEditMode.value && currentServerForm.id > 0) {
    formData.id = currentServerForm.id
    // add org disabled_tools
    formData.config.disabled_tools = currentServerForm.config.disabled_tools
  }

  try {
    await mcpStore.saveMcpServer(formData)
    dialogVisible.value = false
    const langKey = isEditMode.value ? 'settings.mcp.updateSuccess' : 'settings.mcp.addSuccess'
    showMessage(t(langKey), 'success')
  } catch (e) {
    let langKey = isEditMode.value ? 'settings.mcp.updateFailed' : 'settings.mcp.addFailed'
    showMessage(t(langKey, { error: e }), 'error')
  } finally {
    formLoading.value = false
  }
}

// =================================================
// Server List Actions
// =================================================
const handleDeleteServerConfirmation = server => {
  serverToOperateOn.value = server
  ElMessageBox.confirm(
    t('settings.mcp.confirmDelete', { name: server.name || '' }),
    t('settings.mcp.deleteConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  )
    .then(() => {
      executeDeleteServer()
    })
    .catch(() => {
      /* User canceled */ serverToOperateOn.value = null
    })
}

const executeDeleteServer = async () => {
  if (!serverToOperateOn.value) return
  try {
    await mcpStore.deleteMcpServer(serverToOperateOn.value.id)
    showMessage(t('settings.mcp.deleteSuccess'), 'success')
    serverToOperateOn.value = null
  } catch (e) {
    showMessage(t('settings.mcp.operationFailed', { error: e.message || String(e) }), 'error') // Pass 'error' as type
  }
}

const toggleServerStatus = async server => {
  const uiState = mcpStore.getOrInitServerUiState(server.id)
  if (uiState.loading) {
    return
  }
  uiState.loading = true
  const originalDisabled = server.disabled

  try {
    if (server.disabled) {
      await mcpStore.enableMcpServer(server.id)
      server.disabled = false
      showMessage(t('settings.mcp.enableSuccess', { name: server.name }), 'success')
    } else {
      await mcpStore.disableMcpServer(server.id)
      server.disabled = true
      showMessage(t('settings.mcp.disableSuccess', { name: server.name }), 'success')
    }
  } catch (e) {
    server.disabled = originalDisabled
    const langKey = originalDisabled ? 'settings.mcp.enableFailed' : 'settings.mcp.disableFailed'
    showMessageBox(t(langKey, { error: e.message || String(e), name: server.name }), 'error')
  } finally {
    uiState.loading = false
  }
}

const restartMcpServer = async server => {
  const uiState = mcpStore.getOrInitServerUiState(server.id)
  if (uiState.loading || server.disabled) return
  uiState.loading = true
  try {
    await mcpStore.restartMcpServer(server.id)
    showMessage(t('settings.mcp.restartSuccess', { name: server.name }), 'success')
  } catch (e) {
    showMessage(
      t('settings.mcp.restartFailed', {
        error: e.message || String(e),
        name: server.name
      }),
      'error'
    )
  } finally {
    uiState.loading = false
  }
}

const toggleServerToolsExpansion = async server => {
  if (server.status !== 'running') {
    return
  }
  // Initialize uiState if it doesn't exist
  // Get UI state from store, it will be initialized if not present
  const uiState = mcpStore.getOrInitServerUiState(server.id)

  uiState.expanded = !uiState.expanded

  if (uiState.expanded && !mcpStore.serverTools[server.id]) {
    // Fetch only if not already fetched
    uiState.loading = true
    try {
      await mcpStore.fetchMcpServerTools(server.id)
    } catch (e) {
      showMessage(t('settings.mcp.fetchToolsFailed', { error: e.message || String(e) }), 'error') // Pass 'error' as type
      uiState.expanded = false // Collapse on error
    } finally {
      uiState.loading = false
    }
  }
}

const getServerStatusClass = status => {
  if (typeof status === 'object' && status !== null && status.hasOwnProperty('error')) {
    return 'error'
  }
  if (typeof status === 'string') {
    return status.toLowerCase()
  }
  return 'stopped'
}

const getServerStatusText = status => {
  console.log('status text:', status)
  if (typeof status === 'object' && status !== null && status.hasOwnProperty('error')) {
    return t('settings.mcp.statusError', status)
  }

  if (typeof status === 'string' && status.length > 0) {
    const capitalizedStatus = status.charAt(0).toUpperCase() + status.slice(1)
    const i18nKey = `settings.mcp.status${capitalizedStatus}`
    return t(i18nKey, capitalizedStatus)
  }

  return t('settings.mcp.statusUnknown', 'Unknown')
}

const toggleDisableTool = (serverId, tool) => {
  try {
    mcpStore.toggleDisableTool(serverId, tool)
  } catch (e) {
    showMessage(t('settings.mcp.operationFailed', { error: e.message || String(e) }), 'error')
  }
}

// =================================================
// Preset MCPs Logic
// =================================================
const loadPresetMcps = async () => {
  if (presetMcps.value.length > 0 && !loadingPresets.value) return // Already loaded
  loadingPresets.value = true
  try {
    const response = await fetch('/presetMcp.json') // Assumes presetMcp.json is in public folder
    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }
    const data = await response.json()
    presetMcps.value = data.map(p => ({
      ...p,
      searchName: (p.name + p.description).toLowerCase()
    }))
  } catch (error) {
    console.error('Failed to load preset MCPs:', error)
    showMessage(t('settings.mcp.loadPresetError'), 'error')
    presetMcps.value = [] // Clear on error
  } finally {
    loadingPresets.value = false
  }
}

const showPresetMcpDialog = async () => {
  presetSearchQuery.value = '' // Reset search
  await loadPresetMcps() // Ensure presets are loaded
  presetMcpsVisible.value = true
}

const filteredPresetMcps = computed(() => {
  if (!presetSearchQuery.value) return presetMcps.value
  const search = presetSearchQuery.value.toLowerCase()
  return presetMcps.value.filter(preset => preset.searchName.includes(search))
})

const isPresetAdded = presetName => {
  return mcpStore.servers.some(server => server.name === presetName)
}

const importPresetMcp = preset => {
  presetMcpsVisible.value = false

  const mcpSpecificConfig = preset.config?.mcpServers?.[preset.name]
  if (!mcpSpecificConfig) {
    showMessage(
      t('settings.mcp.operationFailed', {
        error: `Preset configuration for '${preset.name}' is malformed.`
      }),
      'error'
    )
    openServerFormDialog() // Open empty form as fallback
    return
  }

  const serverData = {
    name: preset.name,
    description: preset.description || '',
    config: {
      // This is McpServerConfig
      name: preset.name, // McpServerConfig's name, defaults to server name
      type: mcpSpecificConfig.type || (mcpSpecificConfig.url ? 'sse' : 'stdio'), // Default to stdio if not specified
      command: mcpSpecificConfig.command || '',
      args: mcpSpecificConfig.args || [],
      env: mcpSpecificConfig.env || [], // Assuming env might be in preset
      // Add other fields from mcpSpecificConfig if they exist (url, bearer_token, proxy, disabled_tools for config)
      ...(mcpSpecificConfig.url && { url: mcpSpecificConfig.url }),
      ...(mcpSpecificConfig.bearer_token && {
        bearer_token: mcpSpecificConfig.bearer_token
      }),
      ...(mcpSpecificConfig.proxy && { proxy: mcpSpecificConfig.proxy }),
      ...(mcpSpecificConfig.disabled_tools && {
        disabled_tools: mcpSpecificConfig.disabled_tools
      })
    }
    // disabled_tools for the McpServer itself can also be added if defined in preset root
  }
  openServerFormDialog(serverData)
}

const handleManualAddFromPresetDialog = () => {
  presetMcpsVisible.value = false
  openServerFormDialog() // Open a blank form for manual addition
}

// =================================================
// Validation Functions
// =================================================
const validatePlaceholders = () => {
  const placeholderRegex = /\{[^}]+\}/g
  const errors = []

  // check Bearer Token, streamable_http and sse may has bearer_token
  if (currentServerForm.config.type !== 'stdio' && currentServerForm.config.bearer_token) {
    const bearerToken = currentServerForm.config.bearer_token.trim()
    if (bearerToken && placeholderRegex.test(bearerToken)) {
      const matches = bearerToken.match(placeholderRegex)
      errors.push(
        t('settings.mcp.validation.bearerTokenPlaceholder', {
          placeholders: matches.join(', ')
        })
      )
    }
  }

  // check the env
  if (currentServerForm.config.envString) {
    const envString = currentServerForm.config.envString.trim()
    if (envString && placeholderRegex.test(envString)) {
      const matches = envString.match(placeholderRegex)
      errors.push(
        t('settings.mcp.validation.envPlaceholder', {
          placeholders: matches.join(', ')
        })
      )
    }
  }

  // check the url, streamable_http and sse must be has url and may has placeholder in url
  if (currentServerForm.config.type !== 'stdio' && currentServerForm.config.url) {
    const url = currentServerForm.config.url.trim()
    if (url && placeholderRegex.test(url)) {
      const matches = url.match(placeholderRegex)
      errors.push(
        t('settings.mcp.validation.urlPlaceholder', {
          placeholders: matches.join(', ')
        })
      )
    }
  }

  // The stdio may has placeholder in command
  if (currentServerForm.config.type === 'stdio') {
    if (
      currentServerForm.config.command &&
      placeholderRegex.test(currentServerForm.config.command)
    ) {
      const matches = currentServerForm.config.command.match(placeholderRegex)
      errors.push(
        t('settings.mcp.validation.commandPlaceholder', {
          placeholders: matches.join(', ')
        })
      )
    }

    if (
      currentServerForm.config.argsString &&
      placeholderRegex.test(currentServerForm.config.argsString)
    ) {
      const matches = currentServerForm.config.argsString.match(placeholderRegex)
      errors.push(
        t('settings.mcp.validation.argsPlaceholder', {
          placeholders: matches.join(', ')
        })
      )
    }
  }

  if (errors.length > 0) {
    return {
      isValid: false,
      message: t('settings.mcp.validation.placeholderError') + '\n' + errors.join('\n')
    }
  }

  return { isValid: true }
}

// =================================================
// Utility Functions
// =================================================
const parseStringToArray = str => {
  if (!str || typeof str !== 'string') return []
  return str
    .split(',')
    .map(s => trimQuotes(s.trim()))
    .filter(s => s)
}

const arrayToString = arr => (arr || []).join(',')

const parseEnvString = str => {
  if (!str || typeof str !== 'string') return []
  return str
    .split('\n')
    .map(line => {
      const parts = line.split(':')
      if (parts.length >= 2) {
        return [trimQuotes(parts[0].trim()), trimQuotes(parts.slice(1).join(':').trim())]
      }
      return null
    })
    .filter(pair => pair && pair[0])
}

const arrayToEnvString = arr => {
  if (!arr || !Array.isArray(arr)) return ''
  return arr.map(pair => `${pair[0]}: ${pair[1]}`).join('\n')
}

const trimQuotes = str => {
  if (str.startsWith('"')) {
    str = str.substring(1).trim()
  }
  if (str.endsWith(',')) {
    str = str.substring(0, str.length - 1).trim()
  }
  return str.replace(/^['"]|['"]$/g, '').trim()
}
</script>

<style lang="scss" scoped>
.card {
  .list {
    .item-wrapper {
      border-bottom: 1px solid var(--cs-border-color);

      &:last-child {
        border: none;
      }

      .item {
        border-bottom: none;

        .server-left {
          display: flex;
          align-items: center;
          flex-grow: 1;

          .expand-btn {
            margin-right: var(--cs-space-sm);
            background: none;
            border: none;
            padding: 0;

            .cs {
              color: var(--cs-text-color-tertiary);
              cursor: default;

              &.running {
                cursor: pointer;
                color: var(--cs-text-color-primary);
              }
            }
          }

          .server-info {
            margin-left: var(--cs-space-sm);

            .server-name {
              font-weight: bold;
            }

            .server-status {
              font-size: var(--cs-font-size-xs);
              color: var(--cs-text-color-primary);
              display: flex;
              align-items: center;

              .status-dot {
                display: inline-block;
                width: 8px;
                height: 8px;
                border-radius: 8px;
                margin-right: var(--cs-space-xs);
                flex-shrink: 0;
              }

              &.starting {
                .status-dot {
                  background: var(--cs-info-color);
                  animation: pulse 1.5s ease-in-out infinite;
                }

                .status-text {
                  color: var(--cs-info-color);
                }
              }

              &.connected {
                .status-dot {
                  background: var(--cs-info-color);
                }

                .status-text {
                  color: var(--cs-info-color);
                }
              }

              &.running {
                .status-dot {
                  background: var(--cs-success-color);
                }

                .status-text {
                  color: var(--cs-success-color);
                }
              }

              &.stopped {
                .status-dot {
                  background: var(--cs-text-color-secondary);
                }

                .status-text {
                  color: var(--cs-text-color-secondary);
                }
              }

              &.error {
                .status-dot {
                  background: var(--cs-error-color);
                }

                .status-text {
                  color: var(--cs-error-color);
                }
              }

              &.unknown {
                .status-dot {
                  background: var(--cs-warning-color);
                }

                .status-text {
                  color: var(--cs-warning-color);
                }
              }
            }
          }
        }

        .value {
          display: flex;
          align-items: center;
          margin-left: 10px;

          .icon.disabled {
            cursor: not-allowed;
            background-color: rgba(0, 0, 0, 0);
          }
        }
      }

      .tools-list {
        list-style: none;

        ul {
          margin: var(--cs-space-sm) var(--cs-space);
          padding: 0;
        }

        .tool-item {
          display: flex;
          flex-direction: row;
          justify-content: space-between;
          margin-top: var(--cs-space-sm);
          padding-top: var(--cs-space-sm);
          border-top: 1px dotted var(--cs-border-color);

          .tool-info {
            display: flex;
            flex-direction: column;
            line-height: 1.5;

            .tool-name {
              font-weight: bold;
              font-size: var(--cs-font-size-md);
            }

            .tool-description {
              font-size: var(--cs-font-size-sm);
            }
          }
        }
      }
    }

    .empty-state {
      text-align: center;
      padding: 40px 0;

      button {
        background: none;
        border: none;
        color: var(--cs-color-primary);
        cursor: pointer;
        padding: 0;
        margin-left: 5px;
        font-size: var(--cs-font-size);
      }
    }
  }

  // Styles for icons within .title and .value
  .icon {
    cursor: pointer;
    padding: 5px; // Add some clickable area
    display: flex; // Center icon if needed
    align-items: center;
    justify-content: center;
    // Add hover effect if desired
  }

  // Styles for your custom Dialog's form container
  // These are from your original file, assuming they target elements within your custom Dialog/Tabs/Tab
  .form-container {
    // If using El-Form, it might have its own padding.
    // If your custom Tabs/Tab add padding, adjust this.
    // padding: 20px; // Add padding if the dialog content area doesn't have it

    .form-group {
      // This class was in your original HTML, but El-Form uses el-form-item
      margin-bottom: 15px;

      label {
        display: block;
        margin-bottom: 5px;
        font-weight: bold;
      }

      input[type="text"],
      // Target specific inputs if not using el-input
      input[type="url"],
      textarea,
      select {
        width: 100%;
        padding: 8px;
        border: 1px solid #ddd; // Theme border
        border-radius: 4px;
        box-sizing: border-box; // Important for width: 100%
      }

      textarea {
        min-height: 80px;
      }

      input[type='checkbox'] {
        width: auto; // Checkboxes shouldn't be 100% width
        margin-right: 5px;
      }
    }

    .json-editor {
      width: 100%;
      min-height: 300px;
      font-family: monospace;
      padding: 10px;
      border: 1px solid #ddd; // Theme border
      border-radius: 4px;
      box-sizing: border-box;
      margin-top: 10px; // Space if it's directly after a tab title
    }
  }

  // Styles for preset list (if you re-implement it)
  .preset-list {
    .preset-item {
      display: flex;
      align-items: center;
      padding: 10px;
      border-bottom: 1px solid #eee;

      .preset-info {
        flex-grow: 1;
        margin-left: 10px;

        .preset-title {
          font-weight: bold;
        }

        .preset-desc {
          font-size: 12px;
          color: #666;
        }
      }

      .add-preset-btn {
        background: none;
        border: none;
        cursor: pointer;
        padding: 5px;

        i {
          font-size: 16px;
        }
      }
    }
  }
}

.el-overlay {
  .model-edit-dialog {
    .el-dialog__header {
      display: none;
    }

    .el-tabs__nav-wrap:after {
      background-color: var(--cs-border-color);
    }
  }
}

.preset-mcps-dialog {
  :deep(.el-dialog__body) {
    padding: 0;
  }

  .preset-mcps-container {
    display: flex;
    flex-direction: column;
    height: 70vh; // Or a fixed height like 500px

    .search-bar {
      padding: var(--cs-space-sm) var(--cs-space) 0;
      background: var(--el-bg-color); // Match theme

      .search-input {
        margin-bottom: var(--cs-space-sm);
      }
    }

    .preset-mcps-list {
      flex: 1;
      overflow-y: auto;
      padding: 0 var(--cs-space) var(--cs-space-sm);

      .empty-state {
        text-align: center;
        padding: 40px 0;
        color: var(--el-text-color-placeholder);
      }
    }
  }

  .preset-mcp-card {
    margin-bottom: var(--cs-space-sm);

    .mcp-item {
      display: flex;
      align-items: center;
      gap: var(--cs-space);

      .mcp-logo {
        // Style for Avatar if needed, or remove if Avatar handles it
        flex-shrink: 0;
      }

      .mcp-details {
        flex: 1;
        min-width: 0; // For text ellipsis

        h3 {
          margin: 0 0 5px 0;
          font-size: 16px;
          line-height: 1.2;
        }

        p {
          margin: 0;
          font-size: 13px;
          color: var(--el-text-color-secondary);
        }
      }
    }
  }
}

@keyframes pulse {
  0% {
    transform: scale(0.8);
  }

  50% {
    transform: scale(1.2);
  }

  100% {
    transform: scale(0.8);
  }
}
</style>
