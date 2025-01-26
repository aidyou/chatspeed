import { defineStore } from 'pinia';
import { nextTick, ref, computed } from 'vue';

import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'

import { isEmpty } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'
import { useSettingStore } from '@/stores/setting'

/**
 * useSkillStore defines a store for managing AI skills.
 * It includes state for the list of skills and related operations.
 */
export const useSkillStore = defineStore('skill', () => {
  // current window label
  const label = getCurrentWebviewWindow().label
  const settingStore = ref(null)

  /**
   * Returns the setting store.
   * @returns {Object} The setting store.
   */
  const getSettingStore = () => {
    if (!settingStore.value) {
      settingStore.value = useSettingStore()
    }
    return settingStore.value
  }

  /**
   * A reactive reference to store all AI skills.
   * @type {import('vue').Ref<Array<Object>>}
   */
  const skills = ref([]);

  /**
   * Returns the available skills.
   * @returns {Array<Object>} The available skills.
   */
  const availableSkills = computed(() => {
    return skills.value.filter(s => !s.disabled)
  })

  /**
   * Sets the skills from the backend.
   * @param {Array|null} value - The skills to set, or null to clear.
   */
  const setSkills = (value) => {
    skills.value = isEmpty(value) ? [] : [...value]
  }

  /**
   * Sets a skill. If the formData has an id, it will update the skill.
   * Otherwise, it will add a new skill.
   * This method will submit data to the backend and update the skills in the local store.
   * @param {Object} formData - The skill data to set.
   * @returns {Promise<void>} A promise that resolves when the skill is successfully updated.
   */
  const setSkill = (formData) => {
    return new Promise((resolve, reject) => {
      try {
        const processedFormData = {
          ...formData,
          logo: processLogoUrl(formData.logo)
        }

        const command = formData.id ? 'update_ai_skill' : 'add_ai_skill'
        invoke(command, processedFormData)
          .then((updatedSkill) => {
            sendSyncState('skill', label);

            if (formData.id) {
              const index = skills.value.findIndex(x => x.id === formData.id);
              if (index !== -1) {
                skills.value[index] = processSkillData(updatedSkill);
              }
              console.log('update skill:', updatedSkill)
            } else {
              console.log('add new skill:', updatedSkill)
              skills.value.push(processSkillData(updatedSkill));
            }

            resolve();
          })
          .catch(err => {
            console.error(`${command} error:`, err);
            reject(err);
          })
      } catch (err) {
        console.error('setSkill error:', err);
        reject(err);
      }
    })
  }

  /**
   * Deletes a skill by its ID.
   * This method will submit data to the backend and update the skills in the local store.
   * @param {number} id - The ID of the skill to delete.
   * @returns {Promise<void>} A promise that resolves when the skill is successfully deleted.
   */
  const deleteSkill = (id) => {
    return new Promise((resolve, reject) => {
      invoke('delete_ai_skill', { id })
        .then(() => {
          const index = skills.value.findIndex(s => s.id === id);
          if (index !== -1) {
            skills.value.splice(index, 1);
          }
          sendSyncState('skill', label);
          resolve();
        })
        .catch((err) => {
          console.error('deleteSkill error:', err);
          reject(err);
        })
    })
  }

  /**
   * Updates the order of skills.
   * @returns {Promise<void>} A promise that resolves when the skill order is successfully updated.
   */
  const updateSkillOrder = () => {
    return new Promise((resolve, reject) => {
      const skillIds = skills.value.map(x => x.id);
      invoke('update_ai_skill_order', { skillIds })
        .then(() => {
          sendSyncState('skill', label);
          resolve();
        })
        .catch((err) => {
          console.error('updateSkillOrder error:', err);
          reject(err);
        })
    })
  }

  let isSkillLoading = false
  /**
   * Fetches all AI skills from the backend and updates the state.
   * Uses Tauri's invoke method to call the backend command `get_all_ai_skills`.
   * If the result is empty, it sets the skills to an empty array.
   */
  const updateSkillStore = () => {
    if (isSkillLoading) {
      return
    }
    isSkillLoading = true
    invoke('get_all_ai_skills')
      .then((result) => {
        if (isEmpty(result)) {
          skills.value = [];
          return;
        }

        skills.value = result.map(processSkillData);
        console.debug('skills', skills.value)
      })
      .catch((error) => {
        console.error('Failed to update skill store:', error);
      })
      .finally(() => {
        isSkillLoading = false
      })
  };


  /**
   * Retrieves a skill by its ID.
   * @param {number} id - The ID of the skill to retrieve.
   * @returns {Object} The skill object, or an empty object if not found.
   */
  const getSkillById = (id) => {
    if (isEmpty(skills.value)) return null;
    return skills.value.find(x => x.id === id) || null;
  }

  /**
   * Processes the logo URL by handling HTTP server prefix
   * @param {string} logo - The logo URL to process
   * @returns {string} The processed logo URL
   */
  const processLogoUrl = (logo) => {
    if (!logo) return '';

    const httpServer = getSettingStore()?.settings?.httpServer

    if (!httpServer) return logo;

    if (logo.startsWith('http://') || logo.startsWith('https://')) {
      const regex = new RegExp(httpServer, 'g');
      return logo.replace(regex, '');
    }

    return logo;
  }

  /**
   * Processes skill data by adding HTTP server prefix to logo URLs
   * @param {Object} skill - The skill data to process
   * @returns {Object} The processed skill data
   */
  const processSkillData = (skill) => {
    return {
      ...skill,
      logo: skill.logo ? getSettingStore()?.settings.httpServer + skill.logo : ''
    }
  }

  // Initialize the skill store by fetching the skills
  nextTick(() => {
    updateSkillStore();
  })

  return {
    skills,
    availableSkills,
    setSkill,
    setSkills,
    deleteSkill,
    updateSkillOrder,
    updateSkillStore,
    getSkillById,
  };
})