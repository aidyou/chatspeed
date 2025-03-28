import { defineStore } from 'pinia';
import { ref, computed, provide } from 'vue';
import i18n from '@/i18n/index.js'

import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'

import { csGetStorage, csSetStorage } from '@/libs/util'
import { csStorageKey } from '@/config/config'
import { getModelLogo } from '@/libs/logo'
import { isEmpty } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

/**
 * useModelStore defines a store for managing AI model providers.
 * It includes state for the list of model providers and related operations.
 */
export const useModelStore = defineStore('modelProvider', () => {
  /**
   * Get current window label
   * @type {string}
   */
  const label = getCurrentWebviewWindow().label

  /**
   * A reactive reference to store all AI model providers.
   * @type {import('vue').Ref<Array<Object>>}
   */
  const providers = ref([]);

  /**
   * Returns the available model providers.
   * @returns {Array<Object>} The available model providers.
   */
  const getAvailableProviders = computed(() => providers.value.filter(m => !m.disabled))

  /**
   * Set the model providers from the backend
   * @param {Array|null} value
   */
  const setModelProviders = (value) => {
    providers.value = isEmpty(value) ? [] : [...value]
    console.log('providers', providers.value)
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
          providers.value = [];
          return;
        }

        providers.value = result.map(model => {
          const processedModel = processModelLogo(model)
          if (model.id === defaultModelProvider.value.id) {
            setDefaultModelProvider(processedModel)
          }
          return processedModel
        });
        console.debug('models', providers.value)

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
   * Retrieves a model provider by its ID.
   * @param {number} id - The ID of the model provider to retrieve.
   * @returns {Object} The model provider object, or an empty object if not found.
   */
  const getModelProviderById = (id) => {
    if (isEmpty(providers.value)) return null;
    return providers.value.find((model) => model.id === id) || null;
  }

  /**
   * A reactive reference to store the default model configuration.
   * @type {import('vue').Ref<Object>}
   */
  const defaultModelProvider = ref(csGetStorage(csStorageKey.defaultProvider, {}))

  /**
   * Sets the default model configuration.
   * @param {Object} value - The new default model configuration.
   */
  const setDefaultModelProvider = (value) => {
    defaultModelProvider.value = !isEmpty(value) ? processModelLogo(value) : {}
    csSetStorage(csStorageKey.defaultProvider, defaultModelProvider.value)
  }

  /**
   * Initializes the default model configuration and identifier.
   * It first looks for a model marked as default; if none is found,
   * it returns the first model that is not disabled.
   * @returns {Object} The default model object, or an empty object if none exists.
   */
  const initDefaultModel = () => {
    if (!isEmpty(defaultModelProvider.value)) {
      console.debug('load defaultModel from localstorage', defaultModelProvider.value)
      return;
    }
    if (isEmpty(providers.value)) return;

    // 查找标记为默认的模型，如果没有则使用第一个可用模型
    const model = getAvailableProviders.value.find((model) => model.isDefault) ||
      (getAvailableProviders.value.length > 0 ? getAvailableProviders.value[0] : null);

    if (model) {
      setDefaultModelProvider(model)
    }
  }

  /**
   * Add model data processing function
   * @param {Object} model
   * @returns {Object} Processed model data
   */
  const processModelLogo = (model) => {
    return {
      ...model,
      logo: getModelLogo(model.defaultModel),
      providerLogo: model?.metadata?.logo || ''
    }
  }

  // =================================================
  // The following functions are related to interaction with the server
  // =================================================

  /**
   * Set a model provider
   * @param {Object} formData
   * @returns {Promise<string>} Promise that resolves to a success message
   */
  const setModelProvider = (formData) => {
    if (!formData.metadata) {
      console.error('metadata is empty:', formData)
    }
    return new Promise((resolve, reject) => {
      const command = formData.id ? 'update_ai_model' : 'add_ai_model'
      invoke(command, formData)
        .then((modelData) => {
          sendSyncState('model', label)

          if (formData.id) {
            const modelIndex = providers.value.findIndex(m => m.id === formData.id)
            if (modelIndex !== -1) {
              providers.value[modelIndex] = processModelLogo(modelData)
            }
          } else {
            providers.value.push(processModelLogo(modelData))
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
   * Delete a model provider from the server and update the model store
   * @param {number} id
   * @returns {Promise<void>}
   */
  const deleteModelProvider = (id) => {
    return new Promise((resolve, reject) => {
      invoke('delete_ai_model', { id })
        .then(() => {
          sendSyncState('model', label)
          const index = providers.value.findIndex(m => m.id === id);
          if (index !== -1) {
            providers.value.splice(index, 1);
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
   * Update the order of model providers on the server
   * @returns {Promise<void>}
   */
  const updateModelProviderOrder = () => {
    return new Promise((resolve, reject) => {
      invoke('update_ai_model_order', { modelIds: providers.value.map(model => model.id) })
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
    providers,
    getAvailableProviders,
    setModelProviders,
    updateModelStore,
    defaultModelProvider,
    setDefaultModelProvider,
    getModelProviderById,
    setModelProvider,
    deleteModelProvider,
    updateModelProviderOrder
  };
})
