<template>
  <div class="card">
    <div class="title">
      <span>{{ t('settings.type.skill') }}</span>
      <el-tooltip :content="$t('settings.skill.add')" placement="left">
        <span class="icon" @click="showPresetSkills()">
          <cs name="add" />
        </span>
      </el-tooltip>
    </div>
    <Sortable
      v-if="skills.length > 0"
      class="list"
      item-key="id"
      :list="skills"
      :options="{
        animation: 150,
        ghostClass: 'ghost',
        dragClass: 'drag',
        draggable: '.draggable',
        forceFallback: true,
        bubbleScroll: true
      }"
      @update="onSortUpdate"
      @end="onDragEnd">
      <template #item="{ element }">
        <div class="item draggable" :key="element.id">
          <div class="label">
            <cs :name="element.icon" color="primary" size="18px" v-if="element.icon" />
            <avatar :text="element.name" :size="20" v-else />
            {{ element.name }}
          </div>

          <!-- manage icons -->
          <div class="value">
            <el-tooltip
              :content="$t('settings.skill.edit')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="editSkill(element.id)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.skill.copy')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="copySkill(element.id)" @mousedown.stop>
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.skill.delete')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="deleteSkill(element.id)" @mousedown.stop>
                <cs name="trash" size="16px" color="secondary" />
              </div>
            </el-tooltip>
          </div>
        </div>
      </template>
    </Sortable>
    <div class="list" v-else>
      <div class="item">
        <div class="label">{{ $t('settings.skill.noSkills') }}</div>
      </div>
    </div>
  </div>

  <!-- add/edit skill dialog -->
  <el-dialog
    v-model="skillDialogVisible"
    width="560px"
    class="skill-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    @closed="onSkillDialogClose">
    <el-form :model="skillForm" :rules="skillRules" ref="formRef">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="t('settings.skill.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.skill.name')" prop="name">
            <el-input v-model="skillForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.skill.icon')" prop="icon">
            <el-select
              v-model="skillForm.icon"
              :placeholder="$t('settings.skill.selectIcon')"
              filterable
              remote
              :remote-method="iconFilter"
              :loading="loading">
              <template #prefix>
                <cs v-if="skillForm.icon" :name="skillForm.icon" color="primary" size="18px" />
              </template>
              <el-option
                v-for="icon in filteredSkillIcons"
                :key="icon.value"
                :label="icon.label"
                :value="icon.value">
                <cs :name="icon.value" color="primary" size="20px" class="option-icon" />
                <span> {{ icon.label }}</span>
              </el-option>
            </el-select>
          </el-form-item>
          <!-- <el-form-item :label="$t('settings.skill.logo')" prop="logo">
            <fileSelector
              type="image"
              ref="fileSelectorRef"
              @fileChanged="onFileSelectorChange"
              :defaultPath="skillForm.logo" />
          </el-form-item> -->
          <el-form-item :label="$t('settings.skill.description')" prop="description">
            <el-input v-model="skillForm.description" type="textarea" :rows="3" />
          </el-form-item>
          <el-form-item :label="$t('settings.skill.selectType')" prop="type">
            <el-select v-model="skillForm.type" :placeholder="$t('settings.skill.selectType')">
              <el-option
                v-for="item in skillDropdown"
                :key="item.value"
                :label="item.label"
                :value="item.value" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.skill.toolsEnabled')" prop="disabled">
            <el-switch v-model="skillForm.toolsEnabled" />
          </el-form-item>
          <el-form-item :label="$t('settings.skill.disabled')" prop="disabled">
            <el-switch v-model="skillForm.disabled" />
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="t('settings.skill.promptInfo')" name="prompt">
          <el-form-item prop="prompt">
            <el-input
              v-model="skillForm.prompt"
              type="textarea"
              :rows="13"
              :placeholder="
                $t('settings.skill.promptPlaceholder', {
                  from: '{fromLang}',
                  to: '{toLang}',
                  content: '{content}'
                })
              " />
          </el-form-item>
          <el-form-item :label="$t('settings.skill.useSystemRole')" prop="useSystemRole">
            <el-switch v-model="skillForm.useSystemRole" />
          </el-form-item>
        </el-tab-pane>
      </el-tabs>
    </el-form>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="skillDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateSkill">{{ $t('common.save') }}</el-button>
      </span>
    </template>
  </el-dialog>

  <!-- preset skills -->
  <el-dialog
    v-model="presetSkillsVisible"
    width="560px"
    class="preset-skills-dialog"
    :title="$t('settings.skill.presetSkills')"
    :show-close="true"
    :close-on-click-modal="true"
    :close-on-press-escape="true">
    <div class="preset-skills-list">
      <div class="preset-skill-item manual-add" @click="manualAdd">
        <div class="preset-skill-icon">
          <cs name="add" size="32px" />
        </div>
        <div class="preset-skill-info">
          <div class="preset-skill-name">{{ $t('settings.skill.manualAdd') }}</div>
          <div class="preset-skill-desc">{{ $t('settings.skill.manualAddDesc') }}</div>
        </div>
      </div>
      <div
        v-for="item in presetSkills"
        :key="item.name"
        class="preset-skill-item"
        @click="importSkill(item)">
        <div class="preset-skill-icon">
          <cs :name="item.icon" size="32px" />
        </div>
        <div class="preset-skill-info">
          <div class="preset-skill-name">{{ item.name }}</div>
          <div class="preset-skill-desc">{{ item.description }}</div>
        </div>
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
const { t } = useI18n()

import { Sortable } from 'sortablejs-vue3'

import iconfonts from '@/components/icon/type.js'
import { showMessage } from '@/libs/util'

import { useSettingStore } from '@/stores/setting'
import { useSkillStore } from '@/stores/skill'

const settingStore = useSettingStore()
// skills
const skillStore = useSkillStore()
// Computed property to get and set models from the store
const skills = computed(() => skillStore.skills)

const formRef = ref(null)
const fileSelectorRef = ref(null)
const skillDialogVisible = ref(false)
const editId = ref(null)
const fileSelect = ref('')
const defaultFormData = {
  icon: '',
  logo: '',
  name: '',
  prompt: '',
  description: '',
  type: '',
  useSystemRole: false,
  disabled: false,
  toolsEnabled: true
}
const skillTypes = [
  'chat',
  'coding',
  'dataAnalysis',
  'imageRecognition',
  'languageDetection',
  'mindMapping',
  'questionAnswering',
  'recommendation',
  'sentimentAnalysis',
  'summarization',
  'textCompletion',
  'translation',
  'voiceRecognition',
  'writing'
]
const skillDropdown = computed(() =>
  skillTypes.map(type => ({
    label: t(`settings.skill.type.${type}`),
    value: type
  }))
)

// Reactive object to hold the form data for the skill
const skillForm = ref({ ...defaultFormData })

// Validation rules for the skill form
const skillRules = {
  name: [{ required: true, message: t('settings.skill.nameRequired') }],
  prompt: [{ required: true, message: t('settings.skill.promptRequired') }]
}

// Create an array of skill icons from the iconfonts object, sorted and filtered by prefix 'skill-'
const skillIcons = Object.keys(iconfonts)
  .sort((a, b) => a.localeCompare(b))
  .filter(key => key.startsWith('skill-'))
  .map(key => ({
    value: key,
    label: key
  }))

const loading = ref(false)
const filteredSkillIcons = ref(skillIcons)

/**
 * Filters the skill icons based on the user's query.
 * If the query is not empty, it sets the loading state to true,
 * waits for 200ms, and then filters the skill icons based on the query.
 * If the query is empty, it resets the filtered icons to the original list.
 * @param {string} query - The search term used to filter skill icons.
 */
const iconFilter = query => {
  if (query !== '') {
    loading.value = true
    setTimeout(() => {
      loading.value = false
      filteredSkillIcons.value = skillIcons.filter(item => {
        return item.label.toLowerCase().includes(query.toLowerCase())
      })
    }, 200)
  } else {
    filteredSkillIcons.value = skillIcons
  }
}

/**
 * Creates a new skill object from the skill data.
 * @param skillData Object - The skill data to create a new skill from.
 */
const createFromSkillData = skillData => {
  return {
    icon: skillData.icon,
    name: skillData.name,
    prompt: skillData.prompt,
    description: skillData.metadata?.description || '',
    type: skillData.metadata?.type || '',
    useSystemRole: skillData.metadata?.useSystemRole || false,
    disabled: skillData.disabled,
    toolsEnabled: skillData.metadata?.toolsEnabled || false
  }
}

/**
 * Opens the skill dialog for editing or creating a new skill.
 * @param {string|null} id - The ID of the skill to edit, or null to create a new skill.
 */
const editSkill = async id => {
  // reset form
  formRef.value?.resetFields()
  fileSelectorRef.value?.reset()
  activeTab.value = 'basic'

  if (id) {
    const skillData = skillStore.getSkillById(id)
    if (!skillData) {
      showMessage(t('settings.skill.notFound'), 'error')
      return
    }
    editId.value = id
    skillForm.value = createFromSkillData(skillData)
  } else {
    editId.value = null
    skillForm.value = { ...defaultFormData }
  }

  skillDialogVisible.value = true
}

const onSkillDialogClose = () => {
  editId.value = null
  skillForm.value = { ...defaultFormData }
  formRef.value?.resetFields()
  fileSelectorRef.value?.reset()
}

/**
 * Creates a copy of the specified skill and opens the dialog for editing.
 * @param {string} id - The ID of the skill to copy.
 */
const copySkill = id => {
  const skillData = skillStore.getSkillById(id)
  if (!skillData) {
    showMessage(t('settings.skill.notFound'), 'error')
    return
  }
  fileSelectorRef.value?.reset()
  editId.value = null
  skillForm.value = createFromSkillData(skillData)
  skillDialogVisible.value = true
}

/**
 * Validates the form and updates or adds a skill based on the current form data.
 */
const updateSkill = () => {
  formRef.value.validate(valid => {
    if (valid) {
      const formData = {
        id: editId.value,
        name: skillForm.value.name,
        icon: skillForm.value.icon,
        logo: fileSelect.value || skillForm.value.logo || '',
        prompt: skillForm.value.prompt,
        disabled: skillForm.value.disabled,
        metadata: {
          description: skillForm.value.description || '',
          type: skillForm.value.type || '',
          useSystemRole: skillForm.value.useSystemRole || false,
          toolsEnabled: skillForm.value.toolsEnabled || false
        }
      }

      skillStore
        .setSkill(formData)
        .then(() => {
          showMessage(
            editId.value ? t('settings.skill.updateSuccess') : t('settings.skill.addSuccess'),
            'success'
          )
          skillDialogVisible.value = false
        })
        .catch(err => {
          if (err instanceof FrontendAppError) {
            showMessage(t('settings.skill.saveFailed', { error: err.toFormattedString() }), 'error')
            console.error(`Error updating skill: ${err.toFormattedString()}`, err.originalError)
          } else {
            showMessage(
              t('settings.skill.saveFailed', { error: err.message || String(err) }),
              'error'
            )
            console.error('Error updating skill:', err)
          }
        })
    } else {
      console.log('error submit!')
      return false
    }
  })
}

/**
 * Confirms and deletes the specified skill.
 * @param {string} id - The ID of the skill to delete.
 */
const deleteSkill = id => {
  ElMessageBox.confirm(t('settings.skill.deleteConfirm'), t('settings.skill.deleteTitle'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning'
  }).then(() => {
    // User confirmed deletion
    skillStore
      .deleteSkill(id)
      .then(() => {
        showMessage(t('settings.skill.deleteSuccess'), 'success')
      })
      .catch(err => {
        if (err instanceof FrontendAppError) {
          showMessage(t('settings.skill.deleteFailed', { error: err.toFormattedString() }), 'error')
          console.error(`Error deleting skill: ${err.toFormattedString()}`, err.originalError)
        } else {
          showMessage(
            t('settings.skill.deleteFailed', { error: err.message || String(err) }),
            'error'
          )
          console.error('Error deleting skill:', err)
        }
      })
  })
}

/**
 * Handles the end of a drag event and updates the skill order.
 */
const onDragEnd = () => {
  skillStore.updateSkillOrder().catch(err => {
    if (err instanceof FrontendAppError) {
      console.error(
        `settings.skill.updateOrderFailed: ${err.toFormattedString()}`,
        err.originalError
      )
      showMessage(
        t('settings.skill.updateOrderFailed', { error: err.toFormattedString() }),
        'error'
      )
    } else {
      console.error('settings.skill.updateOrderFailed', err)
      showMessage(
        t('settings.skill.updateOrderFailed', { error: err.message || String(err) }),
        'error'
      )
    }
  })
}

/**
 * Handles the update event of the Sortable component.
 * @param {Object} e - The event object containing oldIndex and newIndex.
 */
const onSortUpdate = e => {
  const { oldIndex, newIndex } = e
  if (oldIndex === null || newIndex === null) return
  const skillsCopy = [...skills.value]
  const item = skillsCopy.splice(oldIndex, 1)[0]
  skillsCopy.splice(newIndex, 0, item)
  skillStore.setSkills(skillsCopy)
}

/**
 * Handles the change of the logo file.
 * @param {Array} file - The array of selected file paths.
 */
const onFileSelectorChange = file => {
  if (file.length > 0) {
    fileSelect.value = file[0] || ''
  }
}

// Preset skills related
const presetSkillsVisible = ref(false)
const presetSkills = ref([])

/**
 * Shows the preset skills dialog and loads the preset skills data
 */
const showPresetSkills = async () => {
  presetSkillsVisible.value = true
  try {
    const response = await fetch('/presetPrompts.json')
    const data = await response.json()
    presetSkills.value =
      data.prompts[settingStore.settings.interfaceLanguage] || data.prompts['English']
  } catch (error) {
    if (error instanceof FrontendAppError) {
      console.error(
        `Failed to load preset skills: ${error.toFormattedString()}`,
        error.originalError
      )
      ElMessage.error(t('settings.skill.loadPresetError', { error: error.toFormattedString() }))
    } else {
      console.error('Failed to load preset skills:', error)
      ElMessage.error(
        t('settings.skill.loadPresetError', { error: error.message || String(error) })
      )
    }
  }
}

/**
 * Closes the preset skills dialog and opens the edit skill dialog
 */
const manualAdd = () => {
  presetSkillsVisible.value = false
  editSkill()
}

/**
 * Imports a preset skill and opens the edit skill dialog
 * @param {Object} skill - The preset skill data to import
 */
const importSkill = skill => {
  const formData = {
    name: skill.name,
    icon: skill.icon,
    logo: '',
    prompt: skill.prompt,
    disabled: false,
    metadata: {
      description: skill.description || '',
      type: skill.type || '',
      useSystemRole: false
    }
  }

  skillStore
    .setSkill(formData)
    .then(() => {
      showMessage(t('settings.skill.importSuccess'), 'success')
    })
    .catch(err => {
      if (err instanceof FrontendAppError) {
        showMessage(t('settings.skill.importFailed', { error: err.toFormattedString() }), 'error')
        console.error(`Error importing skill: ${err.toFormattedString()}`, err.originalError)
      } else {
        showMessage(
          t('settings.skill.importFailed', { error: err.message || String(err) }),
          'error'
        )
        console.error('Error importing skill:', err)
      }
    })
}

// Add tab activation state control
const activeTab = ref('basic')
</script>

<style lang="scss">
.ghost {
  background: rgba(255, 255, 255, 0.1);
}

.option-icon {
  margin-right: var(--cs-space-sm);
}

.skill-logo {
  width: 18px;
  height: 18px;
  border-radius: var(--cs-border-radius-round);
  margin-right: var(--cs-space-xxs);
}

.el-overlay {
  .skill-edit-dialog {
    .el-dialog__header {
      display: none;
    }

    .el-tabs__nav-wrap:after {
      background-color: var(--cs-border-color);
    }
  }
}

.preset-skills-dialog {
  .preset-skills-list {
    max-height: 400px;
    overflow-y: auto;
    padding: 0 var(--cs-space-md);

    .preset-skill-item {
      display: flex;
      align-items: center;
      padding: var(--cs-space);
      margin-bottom: var(--cs-space-sm);
      border-radius: var(--cs-border-radius-md);
      cursor: pointer;
      transition: all 0.3s;
      border: 1px solid var(--el-border-color);

      &:hover {
        border-color: var(--el-color-primary);
        background-color: var(--el-color-primary-light-9);
      }

      &.manual-add {
        position: sticky;
        top: 0;
        z-index: 1;
        background-color: var(--el-bg-color);
        border-style: dashed;
        margin-bottom: var(--cs-space-md);

        &:hover {
          background-color: var(--el-color-primary-light-9);
        }
      }
    }

    .preset-skill-icon {
      margin-right: var(--cs-space);
      color: var(--el-color-primary);
    }

    .preset-skill-info {
      flex: 1;
    }

    .preset-skill-name {
      font-size: var(--cs-font-size);
      font-weight: 500;
      margin-bottom: var(--cs-space-xxs);
      color: var(--el-text-color-primary);
    }

    .preset-skill-desc {
      font-size: var(--cs-font-size-xs);
      color: var(--el-text-color-secondary);
    }
  }
}
</style>
