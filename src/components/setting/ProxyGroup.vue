<template>
  <div class="proxy-group-container">
    <div class="card">
      <div class="title">
        <span>{{ $t('settings.proxyGroup.title') }}</span>
        <div class="actions">
          <el-tooltip placement="top" :content="$t('settings.proxyGroup.batchUpdate')" :hide-after="0"
            :enterable="false">
            <span class="icon" @click="openBatchUpdateDialog">
              <cs name="skill-format1" />
            </span>
          </el-tooltip>
          <el-tooltip placement="left" :content="$t('settings.proxyGroup.addGroup')" :hide-after="0" :enterable="false">
            <span class="icon" @click="openAddDialog">
              <cs name="add" />
            </span>
          </el-tooltip>
        </div>
      </div>

      <div class="list">
        <template v-if="sortedProxyGroupList.length > 0">
          <div v-for="group in sortedProxyGroupList" :key="group.id" class="item">
            <div class="label">
              <Avatar :size="36" :text="group.name" />
              <div class="label-text">
                <div class="name-row">
                  {{ group.name }}
                  <el-tag v-if="proxyGroupStore.activeGroup === group.name" type="success" size="small" effect="dark"
                    round class="active-tag">
                    {{ $t('settings.proxyGroup.activeGroup') }}
                  </el-tag>
                </div>
                <small>{{ group.description }}</small>
              </div>
            </div>

            <div class="value">
              <el-tooltip placement="top" :hide-after="0" :enterable="false">
                <template #content>
                  {{ $t('settings.proxyGroup.toolCompatMode') }}: {{ $t(`settings.proxyGroup.toolCompatModes.${group.metadata?.toolCompatMode || 'auto'}`) }}
                </template>
                <span class="icon" @click="handleToggleToolCompatMode(group)">
                  <cs :name="getToolCompatModeIcon(group.metadata?.toolCompatMode || 'auto')" size="16px" color="secondary" :active="(group.metadata?.toolCompatMode || 'auto') !== 'auto'" />
                </span>
              </el-tooltip>
              <el-tooltip placement="top" :content="$t('settings.proxyGroup.activateGroup')" :hide-after="0"
                :enterable="false" v-if="proxyGroupStore.activeGroup !== group.name">
                <span class="icon" @click="handleActivateGroup(group.name)">
                  <cs name="check-circle" size="16px" color="secondary" />
                </span>
              </el-tooltip>
              <span class="icon active" v-else>
                <cs name="check-circle" size="16px" color="secondary" :active="true" />
              </span>
              <el-tooltip placement="top" :content="$t('settings.proxyGroup.copyGroup')" :hide-after="0"
                :enterable="false">
                <span class="icon" @click="openCopyDialog(group)">
                  <cs name="copy" size="16px" color="secondary" />
                </span>
              </el-tooltip>
              <el-tooltip placement="top" :content="$t('settings.proxyGroup.editGroup')" :hide-after="0"
                :enterable="false">
                <span class="icon" @click="openEditDialog(group)">
                  <cs name="edit" size="16px" color="secondary" />
                </span>
              </el-tooltip>
              <el-tooltip placement="top" :content="$t('settings.proxyGroup.deleteGroup')" :hide-after="0"
                :enterable="false">
                <span class="icon" @click="handleDeleteGroup(group.id)">
                  <cs name="trash" size="16px" color="secondary" />
                </span>
              </el-tooltip>
            </div>
          </div>
        </template>
        <template v-else>
          <div class="empty-state">
            {{ $t('settings.proxyGroup.noGroupsFound') }}
            <el-button type="primary" @click="openAddDialog" size="small">
              <cs name="add" />{{ $t('settings.proxyGroup.addNow') }}
            </el-button>
          </div>
        </template>
      </div>

      <div class="switch-guide" v-if="sortedProxyGroupList.length > 0">
        <h4>{{ $t('settings.proxyGroup.switchGuideTitle') }}</h4>
        <p v-html="$t('settings.proxyGroup.switchGuideDesc')"></p>
        <div class="url-example">
          <span>{{ $t('settings.proxyGroup.switchGuideUrl') }}</span>
          <code>{{ baseUrl }}/switch/v1/chat/completions</code>
        </div>
      </div>

      <!-- proxy group editor -->
      <el-dialog v-model="dialogVisible" width="560px" class="model-edit-dialog" :show-close="false"
        :close-on-click-modal="false" :close-on-press-escape="false" @closed="resetForm">
        <el-form :model="currentGroup" label-width="120px" ref="proxyGroupFormRef">
          <el-tabs v-model="activeTab">
            <!-- basic info -->
                        <el-tab-pane :label="$t('settings.model.basicInfo')" name="basic">
                          <el-form-item :label="$t('settings.proxyGroup.form.name')" prop="name" :rules="[
                            { validator: validateGroupName, trigger: 'blur' }
                          ]">
                            <el-input v-model="currentGroup.name" :placeholder="$t('settings.proxyGroup.form.namePlaceholder')" />
                          </el-form-item>              <el-form-item :label="$t('settings.proxyGroup.form.description')" prop="description">
                <el-input v-model="currentGroup.description" type="textarea" :rows="2"
                  :placeholder="$t('settings.proxyGroup.form.descriptionPlaceholder')" />
              </el-form-item>
              <el-form-item :label="$t('settings.proxyGroup.form.toolCompatMode')"
                prop="metadata.toolCompatMode">
                <el-select v-model="currentGroup.metadata.toolCompatMode"
                  :placeholder="$t('settings.proxyGroup.form.toolCompatModePlaceholder')">
                  <el-option :label="$t('settings.proxyGroup.toolCompatModes.auto')" value="auto" />
                  <el-option :label="$t('settings.proxyGroup.toolCompatModes.compat')" value="compat" />
                  <el-option :label="$t('settings.proxyGroup.toolCompatModes.native')" value="native" />
                </el-select>
              </el-form-item>
              <el-form-item :label="$t('settings.proxyGroup.form.temperatureRatio')" prop="temperature">
                <el-tooltip :content="$t('settings.proxyGroup.form.temperatureRatioPlaceholder')" placement="top">
                  <el-slider v-model="currentGroup.temperature" :min="0" :max="1.0" :step="0.1" show-input
                    :show-tooltip="false" input-size="small" />
                </el-tooltip>
              </el-form-item>
              <el-form-item :label="$t('settings.proxyGroup.form.disabled')" prop="disabled">
                <el-switch v-model="currentGroup.disabled" />
              </el-form-item>
            </el-tab-pane>

            <!-- prompt info -->
            <el-tab-pane :label="$t('settings.skill.promptInfo')" name="prompt">
              <div class="tab-content-scroll">
                <el-form-item :label="$t('settings.proxyGroup.form.promptInjection')" prop="prompt_injection">
                  <el-select v-model="currentGroup.promptInjection"
                    :placeholder="$t('settings.proxyGroup.form.promptInjectionPlaceholder')">
                    <el-option :label="$t('settings.proxyGroup.promptInjection.off')" value="off" />
                    <el-option :label="$t('settings.proxyGroup.promptInjection.enhance')" value="enhance" />
                    <el-option :label="$t('settings.proxyGroup.promptInjection.replace')" value="replace" />
                  </el-select>
                </el-form-item>
                <el-form-item :label="$t('settings.proxyGroup.form.promptInjectionPosition')"
                  prop="metadata.prompt_injection_position">
                  <el-select v-model="currentGroup.metadata.promptInjectionPosition"
                    :placeholder="$t('settings.proxyGroup.form.promptInjectionPositionPlaceholder')">
                    <el-option :label="$t('settings.proxyGroup.promptInjectionPosition.system')" value="system" />
                    <el-option :label="$t('settings.proxyGroup.promptInjectionPosition.user')" value="user" />
                  </el-select>
                </el-form-item>
                <el-form-item :label="$t('settings.proxyGroup.form.modelInjectionCondition')"
                  prop="metadata.model_injection_condition">
                  <el-input v-model="currentGroup.metadata.modelInjectionCondition" type="textarea" :rows="2"
                    :autosize="{ minRows: 2, maxRows: 5 }"
                    :placeholder="$t('settings.proxyGroup.form.modelInjectionConditionPlaceholder')" />
                </el-form-item>
                <el-form-item :label="$t('settings.proxyGroup.form.promptText')" prop="prompt_text">
                  <el-input v-model="currentGroup.promptText" type="textarea" :rows="4"
                    :placeholder="$t('settings.proxyGroup.form.promptTextPlaceholder')" />
                </el-form-item>
                <el-form-item :label="$t('settings.proxyGroup.form.toolFilter')" prop="tool_filter">
                  <el-input v-model="currentGroup.toolFilter" type="textarea" :rows="3"
                    :placeholder="$t('settings.proxyGroup.form.toolFilterPlaceholder')" />
                </el-form-item>

                <div class="custom-headers-section">
                  <div class="header-title">
                    <span>{{ $t('settings.proxyGroup.form.promptReplace') }}</span>
                    <el-tooltip :content="$t('settings.proxyGroup.form.promptReplaceTip')" placement="top">
                      <cs name="help-circle" size="14px" color="secondary" style="margin-left: 4px" />
                    </el-tooltip>
                  </div>

                  <div v-for="(item, index) in currentGroup.metadata.promptReplace" :key="index" class="header-row"
                    style="display: flex; gap: 10px; margin-bottom: 10px">
                    <el-input v-model="item.key" :placeholder="$t('settings.model.headerKey')" style="flex: 1" />
                    <el-input v-model="item.value" :placeholder="$t('settings.model.headerValue')" style="flex: 2" />
                    <el-button type="danger" link @click="removePromptReplace(index)"
                      style="padding: 0; min-width: 24px">
                      <cs name="trash" size="16px" />
                    </el-button>
                  </div>

                  <el-button type="primary" plain size="small" @click="addPromptReplace" style="width: 100%">
                    <cs name="add" /> {{ $t('settings.model.addHeader') }}
                  </el-button>
                </div>
              </div>
            </el-tab-pane>
          </el-tabs>
        </el-form>
        <template #footer>
          <span class="dialog-footer">
            <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
            <el-button type="primary" @click="handleGroupConfigSubmit" :loading="formLoading">
              {{ $t('common.confirm') }}
            </el-button>
          </span>
        </template>
      </el-dialog>

      <!-- batch update dialog -->
      <el-dialog v-model="batchUpdateDialogVisible" :title="$t('settings.proxyGroup.batchUpdateTitle')" width="600px"
        align-center class="proxy-group-edit-dialog" :show-close="false" :close-on-click-modal="false"
        :close-on-press-escape="false" style="--el-dialog-margin-top: 5vh; min-height: 600px">
        <div class="form-container">
          <el-form :model="batchUpdateForm" label-width="auto" style="padding-top: 10px">
            <el-form-item :label="$t('settings.proxyGroup.selectGroups')">
              <el-select v-model="batchUpdateForm.selectedIds" multiple collapse-tags collapse-tags-indicator
                style="width: 100%">
                <el-option v-for="item in sortedProxyGroupList" :key="item.id" :label="item.name" :value="item.id" />
              </el-select>
            </el-form-item>

            <el-divider border-style="dashed">{{ $t('settings.proxyGroup.selectFieldsToUpdate') }}</el-divider>

            <div class="tab-content-scroll" style="max-height: 400px">
              <el-form-item :label="$t('settings.proxyGroup.form.loadFromTemplate')">
                <el-select v-model="batchUpdateForm.templateGroupId" style="width: 100%" :placeholder="$t('settings.proxyGroup.form.selectTemplate')" @change="loadTemplate">
                  <el-option v-for="item in sortedProxyGroupList" :key="item.id" :label="item.name" :value="item.id" />
                </el-select>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.promptInjection')">
                <el-row :gutter="10" style="width: 100%; align-items: center">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.promptInjection" />
                  </el-col>
                  <el-col :span="22">
                    <el-select v-model="batchUpdateForm.promptInjection" :disabled="!batchUpdateFields.promptInjection"
                      style="width: 100%">
                      <el-option :label="$t('settings.proxyGroup.promptInjection.off')" value="off" />
                      <el-option :label="$t('settings.proxyGroup.promptInjection.enhance')" value="enhance" />
                      <el-option :label="$t('settings.proxyGroup.promptInjection.replace')" value="replace" />
                    </el-select>
                  </el-col>
                </el-row>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.promptInjectionPosition')">
                <el-row :gutter="10" style="width: 100%; align-items: center">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.promptInjectionPosition" />
                  </el-col>
                  <el-col :span="22">
                    <el-select v-model="batchUpdateForm.promptInjectionPosition"
                      :disabled="!batchUpdateFields.promptInjectionPosition" style="width: 100%">
                      <el-option :label="$t('settings.proxyGroup.promptInjectionPosition.system')" value="system" />
                      <el-option :label="$t('settings.proxyGroup.promptInjectionPosition.user')" value="user" />
                    </el-select>
                  </el-col>
                </el-row>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.modelInjectionCondition')">
                <el-row :gutter="10" style="width: 100%; align-items: flex-start">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.modelInjectionCondition" style="margin-top: 4px" />
                  </el-col>
                  <el-col :span="22">
                    <el-input v-model="batchUpdateForm.modelInjectionCondition"
                      :disabled="!batchUpdateFields.modelInjectionCondition" type="textarea" :rows="2"
                      :autosize="{ minRows: 2, maxRows: 5 }" />
                  </el-col>
                </el-row>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.promptText')">
                <el-row :gutter="10" style="width: 100%; align-items: flex-start">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.promptText" style="margin-top: 4px" />
                  </el-col>
                  <el-col :span="22">
                    <el-input v-model="batchUpdateForm.promptText" :disabled="!batchUpdateFields.promptText"
                      type="textarea" :rows="4" />
                  </el-col>
                </el-row>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.toolFilter')">
                <el-row :gutter="10" style="width: 100%; align-items: flex-start">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.toolFilter" style="margin-top: 4px" />
                  </el-col>
                  <el-col :span="22">
                    <el-input v-model="batchUpdateForm.toolFilter" :disabled="!batchUpdateFields.toolFilter"
                      type="textarea" :rows="3" />
                  </el-col>
                </el-row>
              </el-form-item>

              <el-form-item :label="$t('settings.proxyGroup.form.promptReplace')">
                <el-row :gutter="10" style="width: 100%; align-items: flex-start">
                  <el-col :span="2">
                    <el-checkbox v-model="batchUpdateFields.promptReplace" style="margin-top: 4px" />
                  </el-col>
                  <el-col :span="22">
                    <div v-for="(item, index) in batchUpdateForm.promptReplace" :key="index" class="header-row"
                      style="display: flex; gap: 10px; margin-bottom: 10px">
                      <el-input v-model="item.key" :placeholder="$t('settings.model.headerKey')" style="flex: 1"
                        :disabled="!batchUpdateFields.promptReplace" />
                      <el-input v-model="item.value" :placeholder="$t('settings.model.headerValue')" style="flex: 2"
                        :disabled="!batchUpdateFields.promptReplace" />
                      <el-button type="danger" link @click="removeBatchPromptReplace(index)"
                        :disabled="!batchUpdateFields.promptReplace" style="padding: 0; min-width: 24px">
                        <cs name="trash" size="16px" />
                      </el-button>
                    </div>
                    <el-button type="primary" plain size="small" @click="addBatchPromptReplace"
                      :disabled="!batchUpdateFields.promptReplace" style="width: 100%">
                      <cs name="add" /> {{ $t('settings.model.addHeader') }}
                    </el-button>
                  </el-col>
                </el-row>
              </el-form-item>
            </div>
          </el-form>
        </div>
        <template #footer>
          <span class="dialog-footer">
            <el-button @click="batchUpdateDialogVisible = false">{{ $t('common.cancel') }}</el-button>
            <el-button type="primary" @click="handleBatchUpdateSubmit" :loading="formLoading">
              {{ $t('common.confirm') }}
            </el-button>
          </span>
        </template>
      </el-dialog>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'
import { ElMessageBox } from 'element-plus'

import { useProxyGroupStore } from '@/stores/proxy_group'
import { showMessage } from '@/libs/util'
import { useSettingStore } from '@/stores/setting'
import { FrontendAppError } from '@/libs/tauri'

const settingStore = useSettingStore()
const { settings, env } = storeToRefs(settingStore)

const { t } = useI18n()
const proxyGroupStore = useProxyGroupStore()

const dialogVisible = ref(false)
const isEditing = ref(false)
const formLoading = ref(false)
const proxyGroupFormRef = ref(null)
const activeTab = ref('basic')

const validateGroupName = (rule, value, callback) => {
  if (!value) {
    callback(new Error(t('settings.proxyGroup.validation.nameRequired')))
  } else if (value.toLowerCase() === 'switch') {
    callback(new Error(t('settings.proxyGroup.validation.nameReserved')))
  } else {
    callback()
  }
}

const initialGroupState = () => ({
  id: null,
  name: '',
  description: '',
  promptInjection: 'off',
  promptText: '',
  toolFilter: '',
  temperature: 1.0,
  metadata: {
    maxContext: 0,
    promptInjectionPosition: 'system',
    modelInjectionCondition: '',
    promptReplace: [],
    toolCompatMode: 'auto'
  },
  disabled: false
})

const currentGroup = ref(initialGroupState())

onMounted(() => {
  proxyGroupStore.getList()
})

const baseUrl = computed(() => {
  return (
    env.value.chatCompletionProxy || 'http://127.0.0.1:' + settings.value.chatCompletionProxyPort
  )
})

const sortedProxyGroupList = computed(() => {
  return [...proxyGroupStore.list].sort((a, b) => {
    return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })
  })
})

const openAddDialog = () => {
  isEditing.value = false
  currentGroup.value = initialGroupState()
  activeTab.value = 'basic'
  dialogVisible.value = true
}

const openCopyDialog = group => {
  isEditing.value = false
  const { id, ...groupWithoutId } = group
  const newGroupName = generateUniqueName(group.name)
  currentGroup.value = {
    ...groupWithoutId,
    id: null,
    name: newGroupName
  }
  if (!currentGroup.value.metadata) {
    currentGroup.value.metadata = { maxContext: 0, modelInjectionCondition: '', promptReplace: [], toolCompatMode: 'auto' }
  }
  if (!currentGroup.value.metadata.modelInjectionCondition) {
    currentGroup.value.metadata.modelInjectionCondition = ''
  }
  if (!currentGroup.value.metadata.promptReplace) {
    currentGroup.value.metadata.promptReplace = []
  }
  if (!currentGroup.value.metadata.toolCompatMode) {
    currentGroup.value.metadata.toolCompatMode = 'auto'
  }
  console.log('Copied group:', currentGroup.value)
  activeTab.value = 'basic'
  dialogVisible.value = true
}

const generateUniqueName = baseName => {
  const existingNames = proxyGroupStore.list.map(g => g.name)

  if (!existingNames.includes(baseName)) {
    return baseName
  }

  let counter = 2
  let newName = `${baseName}${counter}`

  while (existingNames.includes(newName)) {
    counter++
    newName = `${baseName}${counter}`
  }

  return newName
}

const openEditDialog = group => {
  isEditing.value = true
  currentGroup.value = { ...group }
  if (!currentGroup.value.metadata) {
    currentGroup.value.metadata = { maxContext: 0, modelInjectionCondition: '', promptReplace: [], toolCompatMode: 'auto' }
  }
  // 确保在编辑现有分组时包含modelInjectionCondition字段
  if (!currentGroup.value.metadata.modelInjectionCondition) {
    currentGroup.value.metadata.modelInjectionCondition = ''
  }
  if (!currentGroup.value.metadata.promptReplace) {
    currentGroup.value.metadata.promptReplace = []
  }
  if (!currentGroup.value.metadata.toolCompatMode) {
    currentGroup.value.metadata.toolCompatMode = 'auto'
  }
  console.log(currentGroup.value)
  activeTab.value = 'basic'
  dialogVisible.value = true
}

const resetForm = () => {
  currentGroup.value = initialGroupState()
  isEditing.value = false
  if (proxyGroupFormRef.value) {
    proxyGroupFormRef.value.resetFields()
  }
  formLoading.value = false
}

const handleGroupConfigSubmit = async () => {
  if (!proxyGroupFormRef.value) return
  await proxyGroupFormRef.value.validate(async valid => {
    if (valid) {
      formLoading.value = true
      const newGroup = { ...currentGroup.value }
      // Filter out empty prompt replacement keys
      if (newGroup.metadata && newGroup.metadata.promptReplace) {
        newGroup.metadata.promptReplace = newGroup.metadata.promptReplace.filter(item => item.key.trim() !== '')
      }
      try {
        if (isEditing.value) {
          await proxyGroupStore.update(newGroup)
          showMessage(t('settings.proxyGroup.updateSuccess'), 'success')
        } else {
          await proxyGroupStore.add(newGroup)
          showMessage(t('settings.proxyGroup.addSuccess'), 'success')
        }
        dialogVisible.value = false
      } catch (error) {
        if (error instanceof FrontendAppError) {
          showMessage(
            t('settings.proxyGroup.saveFailed', { error: error.toFormattedString() }),
            'error'
          )
          console.error('Error saving proxy group:', error.originalError)
        } else {
          showMessage(
            t('settings.proxyGroup.saveFailed', { error: error.message || String(error) }),
            'error'
          )
          console.error('Error saving proxy group:', error)
        }
      } finally {
        formLoading.value = false
      }
    }
  })
}

const handleDeleteGroup = id => {
  ElMessageBox.confirm(
    t('settings.proxyGroup.deleteConfirmText'),
    t('settings.proxyGroup.deleteConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  )
    .then(async () => {
      try {
        await proxyGroupStore.remove(id)
        showMessage(t('settings.proxyGroup.deleteSuccess'), 'success')
      } catch (error) {
        if (error instanceof FrontendAppError) {
          showMessage(
            t('settings.proxyGroup.deleteFailed', { error: error.toFormattedString() }),
            'error'
          )
          console.error('Error deleting proxy group:', error.originalError)
        } else {
          showMessage(
            t('settings.proxyGroup.deleteFailed', { error: error.message || String(error) }),
            'error'
          )
          console.error('Error deleting proxy group:', error)
        }
      }
    })
    .catch(() => { })
}

const handleActivateGroup = async name => {
  try {
    await proxyGroupStore.setActiveGroup(name)
    showMessage(t('settings.proxyGroup.updateSuccess'), 'success')
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: String(error) }), 'error')
  }
}

const addPromptReplace = () => {
  if (!currentGroup.value.metadata.promptReplace) {
    currentGroup.value.metadata.promptReplace = []
  }
  currentGroup.value.metadata.promptReplace.push({ key: '', value: '' })
}

const removePromptReplace = index => {
  currentGroup.value.metadata.promptReplace.splice(index, 1)
}

// =================================================
// batch update
// =================================================
const batchUpdateDialogVisible = ref(false)
const batchUpdateForm = ref({
  selectedIds: [],
  templateGroupId: null,
  promptInjection: 'enhance',
  promptInjectionPosition: 'system',
  modelInjectionCondition: '',
  promptText: '',
  toolFilter: '',
  promptReplace: []
})
const batchUpdateFields = ref({
  promptInjection: false,
  promptInjectionPosition: false,
  modelInjectionCondition: false,
  promptText: false,
  toolFilter: false,
  promptReplace: false
})

const openBatchUpdateDialog = () => {
  batchUpdateForm.value = {
    selectedIds: [],
    templateGroupId: null,
    promptInjection: 'enhance',
    promptInjectionPosition: 'system',
    modelInjectionCondition: '',
    promptText: '',
    toolFilter: '',
    promptReplace: []
  }
  batchUpdateFields.value = {
    promptInjection: false,
    promptInjectionPosition: false,
    modelInjectionCondition: false,
    promptText: false,
    toolFilter: false,
    promptReplace: false
  }
  batchUpdateDialogVisible.value = true
}

const handleBatchUpdateSubmit = async () => {
  if (batchUpdateForm.value.selectedIds.length === 0) {
    showMessage(t('settings.proxyGroup.selectGroups'), 'error')
    return
  }

  const anyFieldSelected = Object.values(batchUpdateFields.value).some(v => v)
  if (!anyFieldSelected) {
    showMessage(t('settings.proxyGroup.selectFieldsToUpdate'), 'error')
    return
  }

  try {
    formLoading.value = true
    const payload = {
      ids: batchUpdateForm.value.selectedIds,
      promptInjection: batchUpdateFields.value.promptInjection ? batchUpdateForm.value.promptInjection : null,
      promptText: batchUpdateFields.value.promptText ? batchUpdateForm.value.promptText : null,
      toolFilter: batchUpdateFields.value.toolFilter ? batchUpdateForm.value.toolFilter : null,
      injectionPosition: batchUpdateFields.value.promptInjectionPosition ? batchUpdateForm.value.promptInjectionPosition : null,
      injectionCondition: batchUpdateFields.value.modelInjectionCondition ? batchUpdateForm.value.modelInjectionCondition : null,
      promptReplace: batchUpdateFields.value.promptReplace ? batchUpdateForm.value.promptReplace.filter(item => item.key.trim() !== '') : null
    }

    await proxyGroupStore.batchUpdate(payload)
    showMessage(t('settings.proxyGroup.updateSuccess'), 'success')
    batchUpdateDialogVisible.value = false
  } catch (error) {
    console.error('Batch update failed:', error)
    showMessage(t('settings.proxyGroup.saveFailed', { error: String(error) }), 'error')
  } finally {
    formLoading.value = false
  }
}

const loadTemplate = () => {
  if (!batchUpdateForm.value.templateGroupId) {
    return
  }
  const templateGroup = sortedProxyGroupList.value.find(g => g.id === batchUpdateForm.value.templateGroupId)
  if (!templateGroup) {
    return
  }
  batchUpdateForm.value.promptInjection = templateGroup.promptInjection || 'off'
  batchUpdateForm.value.promptInjectionPosition = templateGroup.metadata?.promptInjectionPosition || 'system'
  batchUpdateForm.value.modelInjectionCondition = templateGroup.metadata?.modelInjectionCondition || ''
  batchUpdateForm.value.promptText = templateGroup.promptText || ''
  batchUpdateForm.value.toolFilter = templateGroup.toolFilter || ''
  batchUpdateForm.value.promptReplace = templateGroup.metadata?.promptReplace ? JSON.parse(JSON.stringify(templateGroup.metadata.promptReplace)) : []
}

const addBatchPromptReplace = () => {
  if (!batchUpdateForm.value.promptReplace) {
    batchUpdateForm.value.promptReplace = []
  }
  batchUpdateForm.value.promptReplace.push({ key: '', value: '' })
}

const removeBatchPromptReplace = index => {
  batchUpdateForm.value.promptReplace.splice(index, 1)
}

// =================================================
// tool compatibility mode
// =================================================
const getToolCompatModeIcon = mode => {
  switch (mode) {
    case 'compat':
      return 'xml'
    case 'native':
      return 'hammer'
    case 'auto':
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
      metadata: {
        ...group.metadata,
        toolCompatMode: newMode
      }
    }
    await proxyGroupStore.update(updatedGroup)
    showMessage(t('settings.proxyGroup.toolCompatModeChanged', { mode: t(`settings.proxyGroup.toolCompatModes.${newMode}`) }), 'success')
  } catch (error) {
    showMessage(t('settings.proxyGroup.saveFailed', { error: String(error) }), 'error')
  }
}
</script>

<style lang="scss">
.el-overlay {
  .model-edit-dialog {
    .el-dialog__header {
      display: none;
    }

    .el-tabs__nav-wrap:after {
      background-color: var(--cs-border-color);
    }

    .el-tabs__header {
      background-color: transparent !important;
      // margin-bottom: 0;
    }
  }
}
</style>

<style lang="scss" scoped>
.proxy-group-container {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-lg);
}

.proxy-group-edit-dialog {
  :deep(.el-form-item) {
    align-items: flex-start;
  }
}

.card {
  .title {
    display: flex;
    justify-content: space-between;
    align-items: center;

    .actions {
      display: flex;
      align-items: center;
      gap: var(--cs-space-sm);
    }
  }
}

.label-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-weight: 500;
  color: var(--cs-text-color);

  .name-row {
    display: flex;
    align-items: center;
    gap: 8px;

    .active-tag {
      font-size: 10px;
      height: 18px;
      padding: 0 6px;
      line-height: 16px;
    }
  }

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
    display: flex;
    align-items: center;
    gap: var(--cs-space-xxs);

    .cs {
      font-size: 1.1em;
    }
  }
}

.form-container {
  max-height: calc(80vh - 120px);
  overflow-y: auto;
}

.proxy-group-tabs {
  :deep(.el-tabs__content) {
    max-height: calc(70vh - 160px);
    overflow-y: auto;
    padding: var(--cs-space-sm) 4px;
    margin-bottom: var(--cs-space-sm);
  }
}

.custom-headers-section {
  padding: 0 var(--cs-space-sm);

  .header-title {
    font-size: 14px;
    color: var(--el-text-color-regular);
    margin-bottom: 12px;
    display: flex;
    align-items: center;
  }

  .header-row {
    .el-input__inner {
      font-family: var(--cs-font-family-mono);
      font-size: 12px;
    }
  }
}

.switch-guide {
  margin: var(--cs-space) auto;
  padding: var(--cs-space);
  background: var(--cs-bg-color-light);
  border-radius: var(--cs-border-radius-md);
  border: 1px solid var(--cs-border-color);

  h4 {
    margin: 0 0 8px;
    font-size: 14px;
    color: var(--cs-color-primary);
  }

  p {
    margin: 0 0 12px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--cs-text-color-secondary);

    :deep(code) {
      background: var(--cs-bg-color);
      padding: 2px 4px;
      border-radius: 4px;
      font-family: var(--cs-font-family-mono);
    }
  }

  .url-example {
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    display: flex;
    flex-direction: column;
    gap: 4px;

    code {
      background: var(--cs-bg-color);
      padding: 6px 10px;
      border-radius: 4px;
      color: var(--cs-text-color);
      word-break: break-all;
      border: 1px solid var(--cs-border-color);
    }
  }
}
</style>