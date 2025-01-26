import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import i18n from '@/i18n/index.js'

import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'

import { csGetStorage, csSetStorage } from '@/libs/util'
import { csStorageKey } from '@/config/config'
import { getModelLogo } from '@/libs/logo'
import { isEmpty } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

/**
 * useModelStore defines a store for managing AI models.
 * It includes state for the list of models and related operations.
 */
export const useModelStore = defineStore('model', () => {
  /**
   * Get current window label
   * @type {string}
   */
  const label = getCurrentWebviewWindow().label

  /**
   * A reactive reference to store all AI models.
   * @type {import('vue').Ref<Array<Object>>}
   */
  const models = ref([]);

  /**
   * Returns the available models.
   * @returns {Array<Object>} The available models.
   */
  const availableModels = computed(() => models.value.filter(m => !m.disabled))

  /**
   * Set the models from the backend
   * @param {Array|null} value
   */
  const setModels = (value) => {
    models.value = isEmpty(value) ? [] : [...value]
    console.log('models', models.value)
  }

  let isModelLoading = false
  /**
   * Fetches all AI models from the backend and updates the state.
   * Uses Tauri's invoke method to call the backend command `get_all_ai_models`.
   * If the result is empty, it sets the models to an empty array.
   */
  const updateModelStore = () => {
    if (isModelLoading) {
      return
    }
    isModelLoading = true
    invoke('get_all_ai_models')
      .then((result) => {
        if (isEmpty(result)) {
          models.value = [];
          return;
        }

        models.value = result.map(model => {
          const processedModel = processModelData(model)
          if (model.id === defaultModel.value.id) {
            setDefaultModel(processedModel)
          }
          return processedModel
        });
        console.debug('models', models.value)

        initDefaultModel();
      })
      .catch((error) => {
        console.error('Failed to update model store:', error);
        // 可以考虑添加用户提示
      })
      .finally(() => {
        isModelLoading = false
      })
  };

  /**
   * Retrieves a model by its ID.
   * @param {number} id - The ID of the model to retrieve.
   * @returns {Object} The model object, or an empty object if not found.
   */
  const getModelById = (id) => {
    if (isEmpty(models.value)) return null;
    return models.value.find((model) => model.id === id) || null;
  }

  /**
   * A reactive reference to store the default model configuration.
   * @type {import('vue').Ref<Object>}
   */
  const defaultModel = ref(csGetStorage(csStorageKey.defaultModel) || {})

  /**
   * Sets the default model configuration.
   * @param {Object} value - The new default model configuration.
   */
  const setDefaultModel = (value) => {
    defaultModel.value = !isEmpty(value) ? processModelData(value) : {}
    csSetStorage(csStorageKey.defaultModel, defaultModel.value)
  }

  /**
   * Initializes the default model configuration and identifier.
   * It first looks for a model marked as default; if none is found,
   * it returns the first model that is not disabled.
   * @returns {Object} The default model object, or an empty object if none exists.
   */
  const initDefaultModel = () => {
    if (!isEmpty(defaultModel.value)) {
      console.debug('load defaultModel from localstorage', defaultModel.value)
      return;
    }
    if (isEmpty(models.value)) return;

    // 查找标记为默认的模型，如果没有则使用第一个可用模型
    const model = availableModels.value.find((model) => model.isDefault) ||
      (availableModels.value.length > 0 ? availableModels.value[0] : null);

    if (model) {
      setDefaultModel(model)
    }
  }

  /**
   * Add model data processing function
   * @param {Object} model
   * @returns {Object} Processed model data
   */
  const processModelData = (model) => ({
    ...model,
    logo: getModelLogo(model.defaultModel)
  })

  // =================================================
  // The following functions are related to interaction with the server
  // =================================================

  /**
   * Set a model
   * @param {Object} formData
   * @returns {Promise<string>} Promise that resolves to a success message
   */
  const setModel = (formData) => {
    if (!formData.metadata) {
      console.error('metadata is empty:', formData)
    }
    return new Promise((resolve, reject) => {
      const command = formData.id ? 'update_ai_model' : 'add_ai_model'
      invoke(command, formData)
        .then((modelData) => {
          sendSyncState('model', label)

          if (formData.id) {
            const modelIndex = models.value.findIndex(m => m.id === formData.id)
            if (modelIndex !== -1) {
              models.value[modelIndex] = processModelData(modelData)
            }
          } else {
            models.value.push(processModelData(modelData))
          }

          resolve(i18n.global.t(`settings.model.${formData.id ? 'updateSuccess' : 'addSuccess'}`))
        })
        .catch((err) => {
          console.error(`${command} error:`, err)
          reject(err)
        })
    })
  }

  /**
   * Delete a model from the server and update the model store
   * @param {number} id
   * @returns {Promise<void>}
   */
  const deleteModel = (id) => {
    return new Promise((resolve, reject) => {
      invoke('delete_ai_model', { id })
        .then(() => {
          sendSyncState('model', label)
          const index = models.value.findIndex(m => m.id === id);
          if (index !== -1) {
            models.value.splice(index, 1);
          }
          resolve()
        })
        .catch((err) => {
          console.error('delete_ai_model error', err)
          reject(err)
        })
    })
  }

  /**
   * Update the order of models on the server
   * @returns {Promise<void>}
   */
  const updateModelOrder = () => {
    return new Promise((resolve, reject) => {
      invoke('update_ai_model_order', { modelIds: models.value.map(model => model.id) })
        .then(() => {
          sendSyncState('model', label)
          resolve()
        })
        .catch(err => {
          console.error('settings.model.updateOrderFailed', err)
          reject(err)
        })
    })
  }

  // =================================================
  // Initialize the model store and export the functions
  // =================================================

  // Initialize the model store
  updateModelStore();

  return {
    models,
    availableModels,
    setModels,
    updateModelStore,
    defaultModel,
    setDefaultModel,
    getModelById,
    setModel,
    deleteModel,
    updateModelOrder
  };
})
