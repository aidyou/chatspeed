import { defineStore } from 'pinia';
import { ref, computed, nextTick } from 'vue';
import i18n from '@/i18n/index.js'

import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'

import { csGetStorage, csSetStorage } from '@/libs/util'
import { csStorageKey } from '@/config/config'
import { getModelLogo } from '@/libs/logo'
import { isEmpty } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

/**
 * @typedef {Object} ModelInfo
 * @property {string} id - The unique identifier of the model.
 * @property {string} name - The display name of the model.
 * @property {string} group - The group this model belongs to.
 * @property {boolean} reasoning - Indicates if the model supports reasoning.
 * @property {boolean} functionCall - Indicates if the model supports function calling.
 */

/**
 * @typedef {Object} ModelMetadata
 * @property {number|null} [frequencyPenalty] - The frequency penalty setting.
 * @property {string|null} [logo] - URL or path to the model provider's logo.
 * @property {number|null} [n] - Corresponds to OpenAI's 'n' parameter (number of choices to generate).
 * @property {number|null} [presencePenalty] - The presence penalty setting.
 * @property {string|null} [proxyPassword] - Password for the proxy server.
 * @property {string|null} [proxyServer] - Address of the proxy server.
 * @property {string|null} [proxyType] - Type of proxy to use (e.g., 'bySetting', 'none').
 * @property {string|null} [proxyUsername] - Username for the proxy server.
 * @property {string|null} [responseFormat] - The response format (e.g., 'text', 'json_object').
 * @property {string|null} [stop] - Stop sequences for the model.
 */

/**
 * @typedef {Object} ModelProvider
 * @property {number} id - The unique identifier of the model provider record.
 * @property {string} name - The human-readable name of the model provider.
 * @property {ModelInfo[]} models - An array of model information objects supported by this provider.
 * @property {string} defaultModel - The ID of the default model for this provider.
 * @property {string} apiProtocol - The API protocol (e.g., 'openai', 'gemini', 'claude').
 * @property {string} baseUrl - The base URL for the API.
 * @property {string} apiKey - The API key for authentication.
 * @property {number} maxTokens - Default maximum tokens for responses.
 * @property {number} temperature - Default temperature for responses.
 * @property {number} topP - Default top_p for responses.
 * @property {number} topK - Default top_k for responses.
 * @property {number} sortIndex - The sort order index for display.
 * @property {boolean} isDefault - Whether this provider is the default for the application.
 * @property {boolean} disabled - Whether this provider is currently disabled.
 * @property {boolean} isOfficial - Whether this is an official/pre-configured provider.
 * @property {string} officialId - Identifier for official models.
 * @property {ModelMetadata|string|null} metadata - Additional metadata, can be a JSON string or an object.
 * @property {string} [logo] - Processed logo for the default model of this provider. (Added by processModelLogo)
 * @property {string} [providerLogo] - Processed logo specifically for the provider. (Added by processModelLogo from metadata.logo)
 */

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
   * @type {import('vue').Ref<ModelProvider[]>}
   */
  const providers = ref([]);

  /**
   * Returns the available model providers.
   * @returns {ModelProvider[]} The available model providers.
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
          return processModelLogo(model)
        });
        console.debug('models', providers.value)

        initDefaultModel();
      })
      .catch((error) => {
        console.error('Failed to update model store:', error);
      })
      .finally(() => {
        isModelLoading = false
      })
  };

  /**
   * Retrieves a model provider by its ID.
   * @param {number} id - The ID of the model provider to retrieve.
   * @returns {ModelProvider|null} The model provider object, or null if not found.
   */
  const getModelProviderById = (id) => {
    if (isEmpty(providers.value)) return null;
    return providers.value.find((model) => model.id === id) || null;
  }
  /**
   * A reactive reference to store the default model configuration.
   * @type {import('vue').Ref<ModelProvider|{}>}
   */
  const defaultModelProvider = ref({})

  /**
   * Sets the default model configuration.
   * @param {Object} value - The new default model configuration.
   */
  const setDefaultModelProvider = (value) => {
    console.log('setDefaultModelProvider', value)
    defaultModelProvider.value = !isEmpty(value) ? processModelLogo(value) : {}
    csSetStorage(csStorageKey.defaultProvider, defaultModelProvider.value)
  }

  /**
   * Initializes the default model configuration and identifier.
   * It first looks for a model marked as default; if none is found,
   * it returns the first model that is not disabled.
   * @returns {void}
   */
  const initDefaultModel = () => {
    if (isEmpty(providers.value)) return;

    defaultModelProvider.value = csGetStorage(csStorageKey.defaultProvider, {})
    console.debug('load defaultModel from localstorage', defaultModelProvider.value)

    if (!isEmpty(defaultModelProvider.value)) {
      // check if the default model is available
      const currentDefaultModel = getModelProviderById(defaultModelProvider.value.id)
      // current default model has existed, update and return
      if (currentDefaultModel) {
        setDefaultModelProvider(currentDefaultModel)
        return;
      }
    }


    // 查找标记为默认的模型，如果没有则使用第一个可用模型
    const model = getAvailableProviders.value.find((model) => model.isDefault)
      ?? getAvailableProviders.value[0] ?? null;

    if (model) {
      setDefaultModelProvider(model)
    }
  }

  /**
   * Add model data processing function
   * @param {ModelProvider} model
   * @returns {ModelProvider} Processed model data
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
   * @param {ModelProvider & {metadata: ModelMetadata|string}} formData - The model provider data to set. Metadata might be a string initially.
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
          if (formData.id) {
            const modelIndex = providers.value.findIndex(m => m.id === formData.id)
            if (modelIndex !== -1) {
              providers.value[modelIndex] = { ...processModelLogo(modelData) }
            }
          } else {
            providers.value.push(processModelLogo(modelData))
          }

          // Update default model
          if (formData.id === defaultModelProvider.value.id) {
            setDefaultModelProvider(modelData)
          }

          nextTick(() => {
            sendSyncState('model', label)
          })

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
          const index = providers.value.findIndex(m => m.id === id);
          if (index !== -1) {
            providers.value.splice(index, 1);
          }
          nextTick(() => {
            sendSyncState('model', label)
          })
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
          nextTick(() => {
            sendSyncState('model', label)
          })
          resolve()
        })
        .catch(err => {
          console.error('settings.model.updateOrderFailed', err)
          reject(err)
        })
    })
  }

  /**
   * List all models
   * @returns {Promise<Array>}
   */
  const listModels = (apiProtocol, apiUrl, apiKey) => {
    return new Promise((resolve, reject) => {
      invoke('list_models', { apiProtocol, apiUrl, apiKey })
        .then((models) => {
          console.log(models)
          resolve(models)
        })
        .catch(err => {
          console.error('list_models error', err)
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
    updateModelStore,
    defaultModelProvider,
    getModelProviderById,
    setModelProvider,
    setModelProviders,
    setDefaultModelProvider,
    deleteModelProvider,
    updateModelProviderOrder,
    listModels
  };
})
