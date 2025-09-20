<template>
  <div class="card">
    <div class="title">
      <span>{{ t('settings.type.model') }}</span>
      <el-tooltip :content="$t('settings.model.add')" placement="left">
        <span class="icon" @click="showPresetModels()">
          <cs name="add" />
        </span>
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
            <avatar :text="element.name" color="primary" size="20px" v-else />
            {{ element.name }}
          </div>
          <div class="value">
            <el-tooltip
              :content="$t('settings.model.edit')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="editModel(element.id)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.model.copy')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="copyModel(element.id)" @mousedown.stop>
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.model.delete')"
              placement="top"
              :hide-after="0"
              :enterable="false"
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

  <!-- model editor -->
  <el-dialog
    v-model="modelDialogVisible"
    width="560px"
    class="model-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
    <el-form :model="modelForm" :rules="modelRules" ref="formRef" label-width="120px">
      <el-tabs v-model="activeTab">
        <!-- basic info -->
        <el-tab-pane :label="$t('settings.model.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.model.apiProtocol')" prop="apiProtocol">
            <el-select v-model="modelForm.apiProtocol" @change="onApiProtocolChange">
              <el-option v-for="(k, v) in apiProtocolOptions" :key="k" :label="v" :value="k" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.model.provider')" prop="name">
            <el-input v-model="modelForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.logo')" prop="logo">
            <el-input
              v-model="modelForm.logo"
              :placeholder="$t('settings.model.logoPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.apiUrl')" prop="baseUrl">
            <el-input v-model="modelForm.baseUrl" :placeholder="baseUrlPlaceholder" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.apiKey')" prop="apiKey">
            <el-input
              v-model="modelForm.apiKey"
              type="textarea"
              :autosize="{ minRows: 2, maxRows: 5 }"
              :placeholder="$t('settings.model.apiKeyPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.general.proxyType')" prop="proxyType">
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
          <el-form-item
            :label="$t('settings.general.proxyServer')"
            prop="proxyServer"
            v-show="modelForm.proxyType === 'http'">
            <el-input
              v-model="modelForm.proxyServer"
              type="text"
              :placeholder="$t('settings.general.proxyServerPlaceholder')" />
          </el-form-item>
          <el-form-item
            :label="$t('settings.general.proxyUsername')"
            prop="proxyUsername"
            v-show="modelForm.proxyType === 'http'">
            <el-input
              v-model="modelForm.proxyUsername"
              type="text"
              :placeholder="$t('settings.general.proxyUsernamePlaceholder')" />
          </el-form-item>
          <el-form-item
            :label="$t('settings.general.proxyPassword')"
            prop="proxyPassword"
            v-show="modelForm.proxyType === 'http'">
            <el-input
              v-model="modelForm.proxyPassword"
              type="text"
              :placeholder="$t('settings.general.proxyPasswordPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.model.disabled')" prop="disabled">
            <el-switch v-model="modelForm.disabled" />
          </el-form-item>
        </el-tab-pane>
        <!-- /end basic info -->

        <!-- model info -->
        <el-tab-pane :label="$t('settings.model.modelInfo')" name="modelInfo">
          <div class="card card-col-list">
            <div v-if="Object.keys(modelGroups).length > 0" class="card-container">
              <el-card
                v-for="(models, group) in modelGroups"
                :key="group"
                body-class="edit-card-body">
                <template #header>
                  <span>{{ group }}</span>
                </template>
                <div class="list opacity">
                  <div class="item" v-for="model in models" :key="model.id">
                    <div class="label">
                      <span>{{ model.name || model.id }}</span>
                      <span v-if="model.reasoning" class="model-icon">
                        <cs name="reasoning" color="var(--cs-color-primary)" />
                      </span>
                      <!-- <span v-if="model.functionCall" class="model-icon">
                        <cs name="function" color="var(--cs-color-primary)" />
                      </span> -->
                      <span v-if="model?.imageInput" class="model-icon">
                        <cs name="image-add" color="var(--cs-color-primary)" />
                      </span>
                    </div>
                    <div class="value model-action">
                      <el-tooltip
                        :content="$t('settings.model.defaultModel')"
                        placement="top"
                        :hide-after="0"
                        :enterable="false"
                        transition="none">
                        <cs
                          :name="model.id == modelForm.defaultModel ? 'check-circle' : 'uncheck'"
                          @click="onDefaultModelChange(model.id)" />
                      </el-tooltip>

                      <cs name="edit" @click="onModelConfig(model)" />
                      <cs
                        name="trash"
                        color="var(--el-color-danger)"
                        @click="removeModelConfig(model.id)" />
                    </div>
                  </div>
                </div>
              </el-card>
            </div>
            <div
              v-else
              style="
                text-align: center;
                font-size: var(--cs-font-size-lg);
                padding: var(--cs-space-lg);
              ">
              {{ $t('settings.model.noModels') }}
            </div>
            <div class="footer">
              <el-button
                type="success"
                round
                @click="onProviderModelImportShow()"
                :loading="isLoadingProviderModels"
                v-if="isLoadingProviderModels || Object.keys(providerModelToShow).length > 0">
                <cs name="import" />{{ $t('settings.model.import') }}
              </el-button>
              <el-button type="success" round @click="onModelConfig()">
                <cs name="add" />{{ $t('settings.model.add') }}
              </el-button>
            </div>
          </div>
        </el-tab-pane>
        <!-- /end model info -->

        <!-- additional info -->
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
            <el-tooltip
              :content="$t('settings.model.temperaturePlaceholder')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <el-slider
                v-model="modelForm.temperature"
                :min="0"
                :max="2"
                :step="0.1"
                show-input
                :show-tooltip="false"
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.topP')" prop="topP">
            <el-tooltip
              :content="$t('settings.model.topPPlaceholder')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <el-slider
                v-model="modelForm.topP"
                :min="0"
                :max="1"
                :step="0.1"
                show-input
                :format-tooltip="value => value.toFixed(1)"
                :show-tooltip="false"
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.topK')" prop="topK">
            <el-tooltip
              :content="$t('settings.model.topKPlaceholder')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <el-slider
                v-model="modelForm.topK"
                :min="0"
                :max="100"
                :step="1"
                show-input
                :show-tooltip="false"
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.frequencyPenalty')" prop="frequencyPenalty">
            <el-tooltip
              :content="$t('settings.model.frequencyPenaltyPlaceholder')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <el-slider
                v-model="modelForm.frequencyPenalty"
                :min="-2"
                :max="2"
                :step="1"
                show-input
                :show-tooltip="false"
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.presencePenalty')" prop="presencePenalty">
            <el-tooltip
              :content="$t('settings.model.presencePenaltyPlaceholder')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <el-slider
                v-model="modelForm.presencePenalty"
                :min="-2"
                :max="2"
                :step="1"
                show-input
                :show-tooltip="false"
                input-size="small" />
            </el-tooltip>
          </el-form-item>
          <el-form-item :label="$t('settings.model.stop')" prop="stop">
            <el-input
              v-model="modelForm.stop"
              type="textarea"
              :autosize="{ minRows: 2, maxRows: 5 }"
              :placeholder="$t('settings.model.stopPlaceholder')" />
          </el-form-item>
        </el-tab-pane>
        <!-- /end additional info -->
      </el-tabs>
    </el-form>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="modelDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateModel">{{ $t('common.save') }}</el-button>
      </span>
    </template>
  </el-dialog>

  <el-dialog
    v-model="modelImportDialogVisible"
    align-center
    width="500px"
    :title="$t('settings.model.modelConfig')"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
    <el-input
      v-model="providerModelKeyword"
      :placeholder="$t('settings.model.searchByIdOrName')"
      clearable
      style="margin-bottom: 15px" />
    <div class="card card-col-list card-model-import">
      <div v-if="Object.keys(providerModelToShow).length > 0" class="card-container">
        <el-card
          v-for="(providerModels, group) in providerModelToShow"
          :key="group"
          body-class="edit-card-body">
          <template #header>
            <span>{{ group }}</span>
          </template>
          <div class="list opacity">
            <div class="item" v-for="model in providerModels" :key="model.id">
              <div class="label">
                <el-tooltip
                  v-if="model.id"
                  :content="model.id"
                  placement="top"
                  :hide-after="0"
                  :enterable="false"
                  transition="none">
                  <span>{{ model.name || model.id }}</span>
                </el-tooltip>
                <span v-if="model.reasoning" class="model-icon">
                  <cs name="reasoning" color="var(--cs-color-primary)" />
                </span>
                <!-- <span v-if="model.functionCall" class="model-icon">
                  <cs name="function" color="var(--cs-color-primary)" />
                </span> -->
                <span v-if="model.imageInput" class="model-icon">
                  <cs name="image-add" color="var(--cs-color-primary)" />
                </span>
              </div>
              <div class="value model-action">
                <el-tooltip
                  v-if="!formModelIds.includes(model.id)"
                  :content="$t('settings.model.add')"
                  placement="top"
                  :hide-after="0"
                  :enterable="false"
                  transition="none">
                  <cs
                    :name="providerModelSelected[model.id] ? 'check-circle' : 'uncheck'"
                    :active="providerModelSelected[model.id]"
                    @click="onProviderModelSelected(model.id)" />
                </el-tooltip>
              </div>
            </div>
          </div>
        </el-card>
      </div>
      <div v-else class="model-not-found">
        {{ $t('settings.model.noModelsFound') }}
      </div>
    </div>
    <template #footer>
      <div class="dialog-footer">
        <el-button @click="modelImportDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="onProviderModelSave">{{ $t('common.save') }}</el-button>
      </div>
    </template>
  </el-dialog>

  <!-- model config -->
  <el-dialog
    v-model="modelConfigDialogVisible"
    align-center
    width="500px"
    :title="$t('settings.model.modelConfig')"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
    <el-form
      :model="modelConfigForm"
      label-width="100px"
      :rules="modelConfigRules"
      ref="configFormRef">
      <el-form-item :label="$t('settings.model.modelId')" prop="id">
        <el-input v-model="modelConfigForm.id" />
      </el-form-item>
      <el-form-item :label="$t('settings.model.modelAlias')" prop="name">
        <el-input v-model="modelConfigForm.name" />
      </el-form-item>
      <el-form-item :label="$t('settings.model.modelGroup')" prop="group">
        <el-input v-model="modelConfigForm.group" />
      </el-form-item>
      <el-form-item :label="$t('settings.model.reasoning')" prop="reasoning">
        <el-switch v-model="modelConfigForm.reasoning" />
      </el-form-item>
      <!-- <el-form-item :label="$t('settings.model.functionCall')" prop="functionCall">
        <el-switch v-model="modelConfigForm.functionCall" />
      </el-form-item> -->
      <el-form-item :label="$t('settings.model.imageInput')" prop="imageInput">
        <el-switch v-model="modelConfigForm.imageInput" />
      </el-form-item>
    </el-form>
    <template #footer>
      <div class="dialog-footer">
        <el-button @click="modelConfigDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateModelConfig">{{ $t('common.save') }}</el-button>
      </div>
    </template>
  </el-dialog>

  <!-- preset models -->
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
            <el-button type="primary" plain @click="onManualAdd()" style="width: 100%">
              <cs name="add" /> {{ $t('settings.model.addDirectly') }}
            </el-button>
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
                @click="openUrl(model.documentationUrl)">
                {{ $t('settings.model.documentation') }}
              </el-link>
              <el-link
                v-if="model.modelListUrl"
                type="primary"
                @click="openUrl(model.modelListUrl)">
                {{ $t('settings.model.modelInfo') }}
              </el-link>
              <el-link v-if="model.keyApplyUrl" type="primary" @click="openUrl(model.keyApplyUrl)">
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
import { computed, ref, watchEffect } from 'vue'
import { useI18n } from 'vue-i18n'
const { t } = useI18n()

import { Sortable } from 'sortablejs-vue3'

import { showMessage, toInt, toFloat, openUrl } from '@/libs/util'
import { useModelStore } from '@/stores/model'

// models
const modelStore = useModelStore()

// Computed property to get and set models from the store
const models = computed(() => modelStore.providers)

const activeTab = ref('basic')
const formRef = ref(null)
const modelDialogVisible = ref(false)
const editId = ref(null)

// Computed property to generate API type options for the select input
const apiProtocolOptions = {
  OpenAI: 'openai',
  Ollama: 'ollama',
  Gemini: 'gemini',
  Claude: 'claude',
  HuggingFace: 'huggingface'
}

/**
 * Computed property to generate proxy type options for the select input
 */
const proxyTypeOptions = computed(() => {
  return ['bySetting', 'http', 'none'].map(key => ({
    label: t(`settings.model.proxyTypes.${key}`),
    value: key
  }))
})

const defaultFormData = {
  apiProtocol: 'openai',
  name: '',
  logo: '',
  models: [],
  defaultModel: '',
  baseUrl: '',
  apiKey: '',
  maxTokens: 4096,
  temperature: 0.8,
  topP: 0.9,
  topK: 40,
  presencePenalty: 0.0,
  frequencyPenalty: 0.0,
  responseFormat: 'text',
  stop: '',
  proxyType: 'bySetting',
  proxyServer: '',
  proxyUsername: '',
  proxyPassword: '',
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
    return 'https://router.huggingface.co/hf-inference/models'
  } else if (modelForm.value.apiProtocol === 'claude') {
    return 'https://api.anthropic.com/v1'
  } else if (modelForm.value.apiProtocol === 'gemini') {
    return 'https://generativelanguage.googleapis.com/v1beta'
  }
  return ''
})

// Validation rules for the model form
const modelRules = {
  apiProtocol: [{ required: true, message: t('settings.model.apiProtocolRequired') }],
  name: [{ required: true, message: t('settings.model.nameRequired') }],
  models: [{ required: true, message: t('settings.model.modelsRequired') }],
  baseUrl: [{ required: true, message: t('settings.model.baseUrlRequired') }]
}

// =================================================
// model utils
// =================================================
const formModelIds = computed(() => {
  return modelForm.value.models.map(model => model.id)
})

const createFromModel = srcModel => {
  return {
    apiProtocol: srcModel.apiProtocol,
    name: srcModel.name,
    logo: srcModel?.metadata?.logo || '',
    models: srcModel.models,
    defaultModel: srcModel.defaultModel,
    baseUrl: srcModel.baseUrl,
    apiKey: srcModel.apiKey,
    maxTokens: srcModel.maxTokens,
    temperature: srcModel.temperature,
    topP: srcModel.topP,
    topK: srcModel.topK,
    disabled: srcModel.disabled,
    // metadata
    presencePenalty: srcModel?.metadata?.presencePenalty || 0.0,
    frequencyPenalty: srcModel?.metadata?.frequencyPenalty || 0.0,
    responseFormat: srcModel?.metadata?.responseFormat || 'text',
    stop: srcModel?.metadata?.stop || '',
    proxyType: srcModel?.metadata?.proxyType,
    proxyServer: srcModel?.metadata?.proxyServer || '',
    proxyUsername: srcModel?.metadata?.proxyUsername || '',
    proxyPassword: srcModel?.metadata?.proxyPassword || ''
  }
}
/**
 * Opens the model dialog for editing or creating a new model.
 * @param {string|null} id - The ID of the model to edit, or null to create a new model.
 */
const editModel = async (id, model) => {
  formRef.value?.resetFields()
  activeTab.value = 'basic'

  if (id) {
    const modelData = modelStore.getModelProviderById(id)
    if (!modelData) {
      showMessage(t('settings.model.notFound'), 'error')
      return
    }
    editId.value = id
    modelForm.value = createFromModel(modelData)
  } else if (model) {
    modelForm.value = { ...defaultFormData }
    modelForm.value.models = [...model.models]
    modelForm.value.apiProtocol = model.protocol

    const keys = ['name', 'logo', 'baseUrl', 'maxTokens', 'temperature', 'topP', 'topK']
    keys.forEach(key => {
      modelForm.value[key] = model[key]
      console.log(key, model[key])
    })
    if (!modelForm.value.baseUrl) {
      modelForm.value.baseUrl = baseUrlPlaceholder
    }

    console.log(modelForm.value)
  } else {
    editId.value = null
    modelForm.value = { ...defaultFormData }
    if (!modelForm.baseUrl) {
      modelForm.value.baseUrl = baseUrlPlaceholder.value
    }
  }
  modelDialogVisible.value = true

  fetchedProviderModelsFromServer(
    modelForm.value.apiProtocol,
    modelForm.value.baseUrl,
    modelForm.value.apiKey
  )
}

/**
 * Creates a copy of the specified model and opens the dialog for editing.
 * @param {string} id - The ID of the model to copy.
 */
const copyModel = id => {
  const modelData = modelStore.getModelProviderById(id)
  if (!modelData) {
    showMessage(t('settings.model.notFound'), 'error')
    return
  }
  editId.value = null
  modelForm.value = createFromModel(modelData)
  modelForm.value.name = modelData.name + '-Copy'
  modelDialogVisible.value = true
}

/**
 * Validates the form and updates or adds a model based on the current form data.
 */
const updateModel = () => {
  formRef.value.validate(valid => {
    // console.log(modelForm.value)
    if (!modelForm.value.models.length) {
      showMessage(t('settings.model.modelsRequired'), 'error')
      return
    }
    if (modelForm.value.proxyType === 'http') {
      if (!modelForm.value.proxyServer) {
        showMessage(t('settings.model.proxyServerRequired'), 'error')
        return
      }
      if (
        modelForm.value.proxyServer.indexOf('http://') !== 0 &&
        modelForm.value.proxyServer.indexOf('https://') !== 0
      ) {
        showMessage(t('settings.general.proxyServerInvalid'), 'error')
        return
      }
    }
    if (modelForm.value.defaultModel === '') {
      modelForm.value.defaultModel = modelForm.value.models[0].id
    } else if (!modelForm.value.models.some(x => x.id === modelForm.value.defaultModel)) {
      modelForm.value.defaultModel = modelForm.value.models[0].id
    }

    if (valid) {
      const formData = {
        id: editId.value,
        apiProtocol: modelForm.value.apiProtocol.trim(),
        name: modelForm.value.name.trim(),
        models: modelForm.value.models,
        defaultModel: modelForm.value.defaultModel.trim(),
        baseUrl: modelForm.value.baseUrl.trim(),
        apiKey: modelForm.value.apiKey.trim(),
        maxTokens: toInt(modelForm.value.maxTokens),
        temperature: toFloat(modelForm.value.temperature),
        topP: toFloat(modelForm.value.topP),
        topK: toInt(modelForm.value.topK),
        disabled: modelForm.value.disabled,
        metadata: {
          logo: modelForm.value.logo || '',
          frequencyPenalty: modelForm.value.frequencyPenalty,
          presencePenalty: modelForm.value.presencePenalty,
          responseFormat: modelForm.value.responseFormat,
          n: Math.max(0, modelForm.value.n),
          stop: modelForm.value.stop.trim() || '',
          proxyType: modelForm.value.proxyType || 'bySetting',
          proxyServer: modelForm.value.proxyServer.trim() || '',
          proxyUsername: modelForm.value.proxyUsername.trim() || '',
          proxyPassword: modelForm.value.proxyPassword.trim() || ''
        }
      }

      modelStore
        .setModelProvider(formData)
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
      .deleteModelProvider(id)
      .then(() => {
        showMessage(t('settings.model.deleteSuccess'), 'success')
      })
      .catch(err => {
        showMessage(err, 'error')
      })
  })
}

// =================================================
// Model Config area
// =================================================
// Reactive object to hold the form data for the model config
const defaultModelConfig = {
  id: '',
  name: '',
  group: '',
  // functionCall: false,
  reasoning: false,
  imageInput: false
}
const modelConfigRules = {
  id: [{ required: true, message: t('settings.model.modelIdRequired') }]
}
const prevModelConfigId = ref('')
const modelConfigForm = ref({ ...defaultModelConfig })
const modelConfigDialogVisible = ref(false)
const modelGroups = computed(() => {
  return modelForm.value.models.reduce((groups, x) => {
    if (!x.group) {
      x.group = t('settings.model.ungrouped')
    }
    groups[x.group] = groups[x.group] || []
    groups[x.group].push(x)
    return groups
  }, {})
})
/**
 * Edits the model config: set the model config to form and open the dialog
 * @param {Object} model - The model config to edit.
 */
const onModelConfig = model => {
  if (model) {
    prevModelConfigId.value = model.id
    model.group = model.group === t('settings.model.ungrouped') ? '' : model.group
    modelConfigForm.value = { ...model }
  } else {
    prevModelConfigId.value = ''
    modelConfigForm.value = { ...defaultModelConfig }
  }
  modelConfigDialogVisible.value = true
}

/**
 * setup the provider base url
 */
const onApiProtocolChange = () => {
  if (!editId.value) {
    modelForm.value.baseUrl = baseUrlPlaceholder
  }
}

/**
 * Changes the default model for the current model provider.
 * @param {string} id - The ID of the model to set as the default model.
 */
const onDefaultModelChange = id => {
  modelForm.value.defaultModel = id
}

/**
 * Updates the model config: update the model config in the form and close the dialog
 */
const updateModelConfig = () => {
  if (!modelConfigForm.value.id) return
  const idToUpdate = prevModelConfigId.value ?? modelConfigForm.value.id
  const index = modelForm.value.models.findIndex(item => item.id === idToUpdate)

  if (index !== -1) {
    if (prevModelConfigId.value && prevModelConfigId.value === modelForm.value.defaultModel) {
      modelForm.value.defaultModel = modelConfigForm.value.id
    }
    modelForm.value.models.splice(index, 1, { ...modelConfigForm.value })
  } else {
    modelForm.value.models.push({ ...modelConfigForm.value })
  }
  modelConfigDialogVisible.value = false
}
/**
 * Remove the model config from the form and close the dialog
 * @param {string} id - The ID of the model config to remove.
 */
const removeModelConfig = id => {
  const index = modelForm.value.models.findIndex(item => item.id === id)
  if (index !== -1) {
    modelForm.value.models.splice(index, 1)

    if (modelForm.value.defaultModel === id) {
      modelForm.value.defaultModel =
        modelForm.value.models.length > 0 ? modelForm.value.models[0].id : ''
    }
  }
}

// =================================================
// model import
// =================================================
const modelImportDialogVisible = ref(false)
const fetchedProviderModels = ref([])
const providerModelSelected = ref({})
const providerModelKeyword = ref('')
const isLoadingProviderModels = ref(false)

watchEffect(async () => {
  const protocol = modelForm.value.apiProtocol
  const baseUrl = modelForm.value.baseUrl
  const apiKey = modelForm.value.apiKey

  let apiKeyIsOptional = false
  if (protocol === 'ollama') {
    apiKeyIsOptional = true
  } else if (protocol === 'openai' && baseUrl && baseUrl.includes('text.pollinations.ai')) {
    apiKeyIsOptional = true
  }

  const essentialParamsPresentAndDialogVisible = modelDialogVisible.value && protocol && baseUrl
  const canFetch = essentialParamsPresentAndDialogVisible && (apiKey || apiKeyIsOptional)

  if (canFetch) {
    fetchedProviderModelsFromServer(protocol, baseUrl, apiKey, {
      proxyType: modelForm.value.proxyType,
      proxyUsername: modelForm.value.proxyUsername,
      proxyPassword: modelForm.value.proxyPassword
    })
  } else {
    fetchedProviderModels.value = []
  }
})

const fetchedProviderModelsFromServer = async (protocol, baseUrl, apiKey, metadata) => {
  isLoadingProviderModels.value = true
  fetchedProviderModels.value = []
  try {
    fetchedProviderModels.value =
      (await modelStore.listModels(protocol, baseUrl, apiKey, metadata)) || []
  } catch (error) {
    console.error('Failed to fetch provider models:', error)
    fetchedProviderModels.value = []
  } finally {
    isLoadingProviderModels.value = false
  }
}

const onProviderModelImportShow = () => {
  modelImportDialogVisible.value = true
  providerModelSelected.value = {}
  providerModelKeyword.value = ''
}
/**
 * Computed property to get the provider models to show in the dialog.
 */
const providerModelToShow = computed(() => {
  // Ensure fetchedProviderModels.value is always an array before spreading
  const baseModels = Array.isArray(fetchedProviderModels.value) ? fetchedProviderModels.value : []
  let modelsToProcess = [...baseModels]

  if (providerModelKeyword.value && providerModelKeyword.value.trim() !== '') {
    const kw = providerModelKeyword.value.toLowerCase().trim()
    modelsToProcess = modelsToProcess.filter(model => {
      if (!model) return false // Skip null/undefined models in the array
      // Safely access id and name, convert to string, then toLowerCase
      const modelId = model.id ? String(model.id).toLowerCase() : ''
      const modelName = model.name ? String(model.name).toLowerCase() : ''
      return modelId.includes(kw) || modelName.includes(kw)
    })
  }

  if (modelsToProcess.length === 0) {
    return {}
  }

  // Group by family
  const groupedModels = {}
  modelsToProcess.forEach(model => {
    if (model && typeof model === 'object') {
      const family = model.family || t('settings.model.ungrouped')
      if (!groupedModels[family]) {
        groupedModels[family] = []
      }
      groupedModels[family].push(model)
    } else {
      // This should ideally not be reached if fetchedProviderModels and filtering are correct
      console.warn('Skipping invalid model data during grouping:', model)
    }
  })
  console.log('groupedModels', groupedModels)
  return groupedModels
})

/**
 * select the provider model to import
 * @param id - The ID of the model to toggle.
 */
const onProviderModelSelected = id => {
  if (providerModelSelected.value[id]) {
    delete providerModelSelected.value[id]
  } else {
    providerModelSelected.value[id] = true
  }
}

const onProviderModelSave = () => {
  if (!fetchedProviderModels.value.length) return
  const selectedModels = Object.keys(providerModelSelected.value)
  if (!selectedModels.length) {
    showMessage(t('settings.model.noModelSelected'), 'error')
    return
  }
  const modelsToAdd = fetchedProviderModels.value
    .filter(model => providerModelSelected.value[model.id])
    .map(model => ({
      id: model.id,
      name: model.name,
      group: model.family || t('settings.model.ungrouped'),
      reasoning: model.reasoning || false,
      // functionCall: model.functionCall || false,
      imageInput: model.imageInput || false
    }))
  console.log('modelsToAdd', modelsToAdd)
  if (modelsToAdd.length) {
    modelForm.value.models.push(...modelsToAdd)
  }
  modelImportDialogVisible.value = false
}

// =================================================
// events
// =================================================

/**
 * Handles the end of a drag event and updates the model order.
 */
const onDragEnd = () => {
  modelStore.updateModelProviderOrder().catch(err => {
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
  modelStore.setModelProviders(modelsCopy)
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
 * Shows the preset models dialog and loads the preset models data
 */
const showPresetModels = async () => {
  if (!presetModels.value.length) {
    try {
      const response = await fetch('/presetTextAiProvider.json')
      const data = await response.json()
      presetModels.value = data.models.map(x => {
        x.searchName = x.name.toLowerCase()
        return { ...x }
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
const onManualAdd = () => {
  presetModelsVisible.value = false
  editModel(null)
}

/**
 * Imports a preset model and opens the edit model dialog
 * @param {Object} model - The preset model data to import
 */
const importPresetModel = model => {
  presetModelsVisible.value = false
  console.log(model)
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
        line-clamp: 2;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
      }
    }
  }
}

.card {
  .el-card__header {
    padding: var(--cs-space-sm) var(--cs-space);
  }

  .edit-card-body {
    padding: var(--cs-space-sm) var(--cs-space);

    .model-action {
      gap: var(--cs-space-sm) !important;

      .cs {
        cursor: pointer;

        &:hover {
          color: var(--el-color-primary) !important;
        }
      }
    }
  }

  &.card-col-list {
    max-height: 550px;
    overflow-y: auto;
    position: relative;

    .card-container {
      display: flex;
      flex-direction: column;
      gap: var(--cs-space-sm);
    }

    .footer {
      position: sticky;
      bottom: 0;
      width: 100%;
      justify-content: flex-start;
      box-sizing: border-box;
      background: var(--cs-bg-color-light);
      padding: var(--cs-space-sm) 0 var(--cs-space-xs);
    }

    &.card-model-import {
      max-height: 600px;

      .model-not-found {
        text-align: center;
        padding: 20px;
        color: var(--el-text-color-secondary);
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

.el-overlay-dialog {
  overflow: hidden;
}
</style>
