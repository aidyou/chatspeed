<template>
  <div class="card">
    <div class="title">
      <span>{{ t('settings.type.model') }}</span>
      <el-tooltip :content="$t('settings.model.add')" placement="top">
        <span class="icon" @click="showPresetModels()"><cs name="add" /></span>
      </el-tooltip>
    </div>
    <Sortable
      v-if="models.length > 0"
      class="list"
      item-key="id"
      :list="models"
      :options="{
        animation: 150,
        ghostClass: 'ghost',
        dragClass: 'drag',
        draggable: '.draggable',
        forceFallback: true,
        bubbleScroll: true
      }"
      @update="onUpdate"
      @end="onDragEnd">
      <template #item="{ element }">
        <div class="item draggable" :key="element.id">
          <div class="label">
            <img
              v-if="element.providerLogo !== ''"
              :src="element.providerLogo"
              class="provider-logo" />
            <logo :name="element.logo" color="primary" size="18px" v-else />
            {{ element.name }}
          </div>
          <div class="value">
            <el-tooltip
              :content="$t('settings.model.edit')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="editModel(element.id)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.model.copy')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="copyModel(element.id)" @mousedown.stop>
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.model.delete')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="deleteModel(element.id)" @mousedown.stop>
                <cs name="trash" size="16px" color="secondary" />
              </div>
            </el-tooltip>
          </div>
        </div>
      </template>
    </Sortable>
    <div class="list" v-else>
      <div class="item">
        <div class="label">{{ $t('settings.model.noModels') }}</div>
      </div>
    </div>
  </div>

  <el-dialog
    v-model="modelDialogVisible"
    width="560px"
    class="model-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
    <el-form :model="modelForm" :rules="modelRules" ref="formRef">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="$t('settings.model.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.model.apiProtocol')" prop="apiProtocol">
            <el-select v-model="modelForm.apiProtocol">
              <el-option v-for="(k, v) in apiProtocolOptions" :key="k" :label="v" :value="k" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.model.name')" prop="name">
            <el-input v-model="modelForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.logo')" prop="logo">
            <el-input
              v-model="modelForm.logo"
              :placeholder="$t('settings.model.logoPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.models')" prop="models">
            <el-select
              v-model="modelForm.modelList"
              multiple
              filterable
              allow-create
              default-first-option
              collapse-tags
              collapse-tags-tooltip
              :placeholder="$t('settings.model.modelsPlaceholder')"
              @change="handleModelChange"
              @paste.native="handlePaste">
              <el-option
                v-for="item in modelForm.modelList"
                :key="item"
                :label="item"
                :value="item" />
            </el-select>
          </el-form-item>
          <el-form-item
            :label="$t('settings.model.defaultModel')"
            prop="defaultModel"
            v-if="defaultModels.length > 0">
            <el-select v-model="modelForm.defaultModel">
              <el-option
                v-for="model in defaultModels"
                :key="model"
                :label="model"
                :value="model" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.model.apiUrl')" prop="baseUrl">
            <el-input v-model="modelForm.baseUrl" :placeholder="baseUrlPlaceholder" />
          </el-form-item>
          <el-form-item
            :label="$t('settings.model.apiKey')"
            prop="apiKey"
            :required="modelForm.apiProtocol !== 'ollama'">
            <el-input
              v-model="modelForm.apiKey"
              type="textarea"
              :autosize="{ minRows: 1, maxRows: 5 }"
              show-password />
          </el-form-item>
        </el-tab-pane>
        <el-tab-pane :label="$t('settings.model.additionalInfo')" name="additional">
          <el-form-item :label="$t('settings.model.maxTokens')" prop="maxTokens">
            <el-input-number
              v-model="modelForm.maxTokens"
              :min="64"
              :max="128000"
              :step="1024"
              :step-strictly="false"
              controls-position="right"
              :placeholder="$t('settings.model.maxTokensPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.temperature')" prop="temperature">
            <el-slider
              v-model="modelForm.temperature"
              :min="0"
              :max="2"
              :step="0.1"
              show-input
              :format-tooltip="value => value.toFixed(1)"
              input-size="small" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.topP')" prop="topP">
            <el-slider
              v-model="modelForm.topP"
              :min="0"
              :max="1"
              :step="0.1"
              show-input
              :format-tooltip="value => value.toFixed(1)"
              input-size="small" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.topK')" prop="topK">
            <el-tooltip
              :content="$t('settings.model.topKPlaceholder')"
              placement="top"
              :hide-after="0"
              transition="none">
              <el-slider
                v-model="modelForm.topK"
                :min="0"
                :max="100"
                :step="1"
                show-input
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.proxyType')" prop="proxyType">
            <el-radio-group v-model="modelForm.proxyType">
              <el-radio
                :label="proxyType.value"
                :value="proxyType.value"
                v-for="proxyType in proxyTypeOptions"
                :key="proxyType.value"
                >{{ proxyType.label }}</el-radio
              >
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="$t('settings.model.disabled')" prop="disabled">
            <el-switch v-model="modelForm.disabled" />
          </el-form-item>
        </el-tab-pane>
      </el-tabs>
    </el-form>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="modelDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateModel">{{ $t('common.save') }}</el-button>
      </span>
    </template>
  </el-dialog>

  <!-- 预设模型列表弹窗 -->
  <el-dialog
    v-model="presetModelsVisible"
    :title="$t('settings.model.presetModels')"
    width="600px"
    align-center
    class="preset-models-dialog">
    <div class="preset-models-container">
      <div class="search-bar">
        <el-row :gutter="10">
          <el-col :span="16">
            <el-input
              v-model="searchQuery"
              :placeholder="$t('common.search')"
              clearable
              class="search-input" />
          </el-col>
          <el-col :span="8">
            <el-button type="primary" plain @click="manualAdd()" style="width: 100%"
              ><cs name="add" /> {{ $t('settings.model.addDirectly') }}</el-button
            >
          </el-col>
        </el-row>
      </div>

      <div class="preset-models-list">
        <el-card
          v-for="model in filteredModels"
          :key="model.name"
          class="preset-model-card"
          shadow="hover">
          <div class="model-item">
            <div class="model-info">
              <img :src="model.logo" class="model-logo" />
              <div class="model-details">
                <h3>{{ model.name }}</h3>
                <p>{{ model.desc }}</p>
              </div>
              <el-button type="success" @click="importPresetModel(model)">{{
                $t('settings.model.addFromPreset')
              }}</el-button>
            </div>
            <div class="links">
              <el-link
                v-if="model.documentationUrl"
                type="primary"
                @click="invokeOpenUrl(model.documentationUrl)">
                {{ $t('settings.model.documentation') }}
              </el-link>
              <el-link
                v-if="model.modelListUrl"
                type="primary"
                @click="invokeOpenUrl(model.modelListUrl)">
                {{ $t('settings.model.modelInfo') }}
              </el-link>
              <el-link
                v-if="model.keyApplyUrl"
                type="primary"
                @click="invokeOpenUrl(model.keyApplyUrl)">
                {{ $t('settings.model.applyKey') }}
              </el-link>
            </div>
          </div>
        </el-card>
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { openUrl } from '@tauri-apps/plugin-opener'
const { t } = useI18n()

import { Sortable } from 'sortablejs-vue3'

import { isEmpty, showMessage, toInt, toFloat } from '@/libs/util'
import { useModelStore } from '@/stores/model'

// models
const modelStore = useModelStore()

// Computed property to get and set models from the store
const models = computed(() => modelStore.models)

const activeTab = ref('basic')
const formRef = ref(null)
const modelDialogVisible = ref(false)
const editId = ref(null)

// Computed property to generate API type options for the select input
const apiProtocolOptions = {
  OpenAI: 'openai',
  Ollama: 'ollama',
  Gemini: 'gemini',
  Claude: 'claude'
}

/**
 * Computed property to generate proxy type options for the select input
 */
const proxyTypeOptions = computed(() => {
  return ['bySetting', 'none'].map(key => ({
    label: t(`settings.model.proxyTypes.${key}`),
    value: key
  }))
})

const defaultFormData = {
  apiProtocol: 'openai',
  name: '',
  logo: '',
  models: '',
  modelList: [],
  defaultModel: '',
  baseUrl: '',
  apiKey: '',
  maxTokens: 4096,
  temperature: 1.0,
  topP: 1.0,
  topK: 40,
  proxyType: 'bySetting',
  disabled: false
}
// Reactive object to hold the form data for the model
const modelForm = ref({ ...defaultFormData })

// Computed property to get the base URL placeholder based on the API type
const baseUrlPlaceholder = computed(() => {
  if (modelForm.value.apiProtocol === 'openai') {
    return 'https://api.openai.com/v1'
  } else if (modelForm.value.apiProtocol === 'ollama') {
    return 'http://localhost:11434/v1'
  } else if (modelForm.value.apiProtocol === 'huggingface') {
    return 'https://api-inference.huggingface.co/models'
  } else if (modelForm.value.apiProtocol === 'anthropic') {
    return 'https://api.anthropic.com'
  } else if (modelForm.value.apiProtocol === 'gemini') {
    return 'https://generativelanguage.googleapis.com/v1beta/models'
  }
  return ''
})

// Validation rules for the model form
const modelRules = {
  apiProtocol: [{ required: true, message: t('settings.model.apiProtocolRequired') }],
  name: [{ required: true, message: t('settings.model.nameRequired') }],
  models: [{ required: true, message: t('settings.model.modelsRequired') }],
  defaultModel: [{ required: true, message: t('settings.model.defaultModelRequired') }],
  // baseUrl: [{ required: true, message: t('settings.model.apiUrlRequired') }],
  apiKey: [
    {
      validator: (_rule, value, callback) => {
        if (modelForm.value.apiProtocol === 'ollama') {
          callback()
        } else if (isEmpty(value)) {
          callback(new Error(t('settings.model.apiKeyRequired')))
        } else {
          callback()
        }
      },
      trigger: 'blur'
    }
  ]
}

// Computed property to derive default models from the input
const defaultModels = computed(() => {
  return modelInit(modelForm.value.models)
})

// Watcher to update the default model when models change
watch(
  () => modelForm.value.models,
  () => {
    if (
      modelForm.value.defaultModel == '' ||
      !defaultModels.value.includes(modelForm.value.defaultModel)
    ) {
      modelForm.value.defaultModel = defaultModels.value.length > 0 ? defaultModels.value[0] : ''
    }
  }
)

/**
 * Initialize models
 * @param {string} models - The models string
 * @returns {Array} - The initialized models array
 */
const modelInit = models => {
  return models
    .trim()
    .replace(/，/g, ',') // Replace Chinese comma with English comma
    .replace(/\n/g, ',') // Replace newline with comma
    .split(',')
    .map(model => model.trim())
    .filter(m => m !== '')
}

/**
 * Opens the model dialog for editing or creating a new model.
 * @param {string|null} id - The ID of the model to edit, or null to create a new model.
 */
const editModel = async (id, model) => {
  formRef.value?.resetFields()
  activeTab.value = 'basic' // 重置为基础信息标签页

  if (id) {
    const modelData = modelStore.getModelById(id)
    if (!modelData) {
      showMessage(t('settings.model.notFound'), 'error')
      return
    }
    editId.value = id
    modelForm.value = {
      apiProtocol: modelData.apiProtocol,
      name: modelData.name,
      logo: modelData?.metadata?.logo || '',
      models: modelData.models.join(','),
      modelList: [...modelData.models],
      defaultModel: modelData.defaultModel,
      baseUrl: modelData.baseUrl,
      apiKey: modelData.apiKey,
      maxTokens: modelData.maxTokens,
      temperature: modelData.temperature,
      topP: modelData.topP,
      topK: modelData.topK,
      disabled: modelData.disabled,
      proxyType: modelData?.metadata?.proxyType
    }
  } else if (model) {
    modelForm.value = { ...defaultFormData }
    modelForm.value.modelList = [...model.models]
    modelForm.value.models = modelForm.value.modelList.join(',')
    modelForm.value.apiProtocol = model.protocol

    const keys = ['name', 'logo', 'baseUrl', 'maxTokens', 'temperature', 'topP', 'topK']
    keys.forEach(key => {
      modelForm.value[key] = model[key]
    })
  } else {
    editId.value = null
    modelForm.value = { ...defaultFormData }
  }
  modelDialogVisible.value = true
}

/**
 * Creates a copy of the specified model and opens the dialog for editing.
 * @param {string} id - The ID of the model to copy.
 */
const copyModel = id => {
  const modelData = modelStore.getModelById(id)
  if (!modelData) {
    showMessage(t('settings.model.notFound'), 'error')
    return
  }
  editId.value = null
  modelForm.value = {
    apiProtocol: modelData.apiProtocol,
    name: modelData.name + '-Copy',
    logo: modelData?.metadata?.logo || '',
    models: modelData.models.join(','),
    modelList: modelData.models,
    defaultModel: modelData.defaultModel,
    baseUrl: modelData.baseUrl,
    apiKey: modelData.apiKey,
    maxTokens: modelData.maxTokens,
    temperature: modelData.temperature,
    topP: modelData.topP,
    topK: modelData.topK,
    disabled: modelData.disabled,
    proxyType: modelData?.metadata?.proxyType
  }
  modelDialogVisible.value = true
}

/**
 * Validates the form and updates or adds a model based on the current form data.
 */
const updateModel = () => {
  formRef.value.validate(valid => {
    console.log(modelForm.value)
    if (valid) {
      const allModels = [...new Set(modelInit(modelForm.value.models))]

      const formData = {
        id: editId.value,
        apiProtocol: modelForm.value.apiProtocol.trim(),
        name: modelForm.value.name.trim(),
        models: allModels,
        defaultModel: modelForm.value.defaultModel.trim(),
        baseUrl: modelForm.value.baseUrl.trim(),
        apiKey: modelForm.value.apiKey.trim(),
        maxTokens: toInt(modelForm.value.maxTokens),
        temperature: toFloat(modelForm.value.temperature),
        topP: toFloat(modelForm.value.topP),
        topK: toInt(modelForm.value.topK),
        disabled: modelForm.value.disabled,
        metadata: {
          proxyType: modelForm.value.proxyType || 'bySetting',
          logo: modelForm.value.logo || ''
        }
      }

      modelStore
        .setModel(formData)
        .then(msg => {
          showMessage(msg, 'success')
          modelDialogVisible.value = false
        })
        .catch(err => {
          showMessage(err, 'error')
        })
    } else {
      console.log('error submit!')
      return false
    }
  })
}

/**
 * Confirms and deletes the specified model.
 * @param {string} id - The ID of the model to delete.
 */
const deleteModel = id => {
  ElMessageBox.confirm(t('settings.model.deleteConfirm'), t('settings.model.deleteTitle'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning'
  }).then(() => {
    // User confirmed deletion
    modelStore
      .deleteModel(id)
      .then(() => {
        showMessage(t('settings.model.deleteSuccess'), 'success')
      })
      .catch(err => {
        showMessage(err, 'error')
      })
  })
}

/**
 * Handles the end of a drag event and updates the model order.
 */
const onDragEnd = () => {
  modelStore.updateModelOrder().catch(err => {
    showMessage(err, 'error')
    console.error('settings.model.updateOrderFailed', err)
  })
}

/**
 * Handles the update event of the Sortable component.
 * @param {Object} e - The event object containing oldIndex and newIndex.
 */
const onUpdate = e => {
  const { oldIndex, newIndex } = e
  if (oldIndex === null || newIndex === null) return
  const modelsCopy = [...models.value]
  const item = modelsCopy.splice(oldIndex, 1)[0]
  modelsCopy.splice(newIndex, 0, item)
  modelStore.setModels(modelsCopy)
}

/**
 * Handles changes in the model list and updates the models string
 * @param {Array} value - Array of selected models
 */
const handleModelChange = value => {
  if (value.length > 0) {
    const lastItem = value[value.length - 1]
    if (lastItem && lastItem.includes(',')) {
      value.pop()
      const newItems = modelInit(lastItem)
      modelForm.value.modelList = [...new Set([...value, ...newItems])]
    }
  }
  modelForm.value.models = modelForm.value.modelList.join(',')
}

const handlePaste = e => {
  e.preventDefault()
  const pastedText = e.clipboardData.getData('text')
  const newItems = modelInit(pastedText)
  modelForm.value.modelList = [...new Set([...modelForm.value.modelList, ...newItems])]
  modelForm.value.models = modelForm.value.modelList.join(',')
}

// preset models
const presetModelsVisible = ref(false)
const presetModels = ref([])
const searchQuery = ref('')

const filteredModels = computed(() => {
  if (!searchQuery.value) return presetModels.value
  const search = searchQuery.value.toLowerCase()
  return presetModels.value.filter(model => model.searchName.includes(search))
})

/**
 * Opens the given URL in the default web browser
 */
const invokeOpenUrl = async url => {
  try {
    await openUrl(url)
  } catch (error) {
    console.log(error)
    showMessage(t('common.openUrlError'), 'error')
  }
}

/**
 * Shows the preset models dialog and loads the preset models data
 */
const showPresetModels = async () => {
  if (!presetModels.value.length) {
    try {
      const response = await fetch('/presetTextAiProvider.json')
      const data = await response.json()
      presetModels.value = data.models.map(x => {
        x.searchName = x.name.toLowerCase()
        return x
      })
    } catch (error) {
      return showMessage(t('settings.model.loadPresetError'), 'error')
    }
  }
  presetModelsVisible.value = true
}
/**
 * closes the preset models dialog and opens the edit model dialog
 */
const manualAdd = () => {
  presetModelsVisible.value = false
  editModel(null)
}

/**
 * Imports a preset model and opens the edit model dialog
 * @param {Object} model - The preset model data to import
 */
const importPresetModel = model => {
  presetModelsVisible.value = false
  editModel(null, model)
}
</script>

<style lang="scss">
.ghost {
  background: rgba(255, 255, 255, 0.1);
}

.el-overlay {
  .model-edit-dialog {
    .el-dialog__header {
      display: none;
    }
    .el-tabs__nav-wrap:after {
      background-color: var(--cs-border-color);
    }

    .el-select {
      .el-select__tags {
        max-height: 52px; // 约等于两行的高度
        overflow-y: auto;

        &::-webkit-scrollbar {
          width: 6px;
        }
        &::-webkit-scrollbar-thumb {
          background: var(--el-border-color);
          border-radius: 3px;
        }
      }
    }
  }
}
.provider-logo {
  width: 18px;
  height: 18px;
  border-radius: 18px;
  margin-right: var(--cs-space-xs);
}

.preset-models-dialog {
  :deep(.el-dialog__body) {
    padding: 0;
  }

  .preset-models-container {
    display: flex;
    flex-direction: column;
    height: 70vh;

    .search-bar {
      padding: var(--cs-space-sm) var(--cs-space) 0;
      background: var(--el-bg-color);
    }

    .preset-models-list {
      flex: 1;
      overflow-y: auto;
      padding: 0 var(--cs-space) var(--cs-space-sm);
    }
  }

  .preset-model-card {
    margin-bottom: var(--cs-space-sm);
    .model-item {
      .model-info {
        display: flex;
        align-items: center;
        gap: var(--cs-space);
        margin-bottom: var(--cs-space-sm);

        .model-logo {
          width: 40px;
          height: 40px;
          flex-shrink: 0;
          border-radius: 40px;

          img {
            width: 100%;
            height: 100%;
            border-radius: 40px;
          }
        }
      }
      .links {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: var(--cs-space);
      }
    }

    .model-details {
      flex: 1;
      min-width: 0;

      h3 {
        margin: 0;
        font-size: 16px;
        line-height: 24px;
      }

      p {
        margin: 5px 0 0;
        font-size: 14px;
        color: var(--el-text-color-secondary);
        overflow: hidden;
        text-overflow: ellipsis;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
      }
    }
  }
}

.search-input {
  margin-bottom: 16px;
}
.el-popper {
  max-width: 550px;
}
</style>
