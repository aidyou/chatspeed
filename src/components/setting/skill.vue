<template>
  <div class="card">
    <div class="title">
      <span>{{ t('settings.type.skill') }}</span>
      <el-tooltip :content="$t('settings.skill.add')" placement="top">
        <span class="icon" @click="editSkill()"><cs name="add" /></span>
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
        bubbleScroll: true,
      }"
      @update="onSortUpdate"
      @end="onDragEnd">
      <template #item="{ element }">
        <div class="item draggable" :key="element.id">
          <div class="label">
            <cs :name="element.icon" color="primary" size="18px" v-if="!element.logo" />
            <img :src="element.logo" v-else class="skill-logo" />
            {{ element.name }}
          </div>

          <!-- manage icons -->
          <div class="value">
            <el-tooltip
              :content="$t('settings.skill.edit')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="editSkill(element.id)">
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.skill.copy')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="copySkill(element.id)">
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.skill.delete')"
              placement="top"
              :hide-after="0"
              transition="none">
              <div class="icon" @click="deleteSkill(element.id)">
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

  <el-dialog
    v-model="skillDialogVisible"
    width="560px"
    class="skill-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
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
          <el-form-item :label="$t('settings.skill.logo')" prop="logo">
            <fileSelector
              type="image"
              ref="fileSelectorRef"
              @fileChanged="onFileSelectorChange"
              :defaultPath="skillForm.logo" />
          </el-form-item>
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
                  content: '{content}',
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
</template>

<script setup>
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
const { t } = useI18n()

import { Sortable } from 'sortablejs-vue3'

import fileSelector from '@/components/common/fileSelector.vue'
import iconfonts from '@/components/icon/type.js'
import { isEmpty, showMessage } from '@/libs/util'
import { useSkillStore } from '@/stores/skill'

// models
const skillStore = useSkillStore()
// Computed property to get and set models from the store
const skills = computed(() => skillStore.skills)

const formRef = ref(null)
const fileSelectorRef = ref(null)
const skillDialogVisible = ref(false)
const editId = ref(null)
const fileSelect = ref('')

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
  'writing',
]
const skillDropdown = computed(() =>
  skillTypes.map(type => ({
    label: t(`settings.skill.type.${type}`),
    value: type,
  }))
)

// Reactive object to hold the form data for the skill
const skillForm = ref({
  icon: '',
  logo: '',
  name: '',
  prompt: '',
  description: '',
  type: '',
  useSystemRole: false,
  disabled: false,
})

// Validation rules for the skill form
const skillRules = {
  // icon: [{ required: true, message: t('settings.skill.iconRequired') }],
  name: [{ required: true, message: t('settings.skill.nameRequired') }],
  prompt: [{ required: true, message: t('settings.skill.promptRequired') }],
}

// Create an array of skill icons from the iconfonts object, sorted and filtered by prefix 'skill-'
const skillIcons = Object.keys(iconfonts)
  .sort((a, b) => a.localeCompare(b))
  .filter(key => key.startsWith('skill-'))
  .map(key => ({
    value: key,
    label: key,
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
 * Opens the skill dialog for editing or creating a new skill.
 * @param {string|null} id - The ID of the skill to edit, or null to create a new skill.
 */
const editSkill = async id => {
  fileSelectorRef.value?.reset()
  activeTab.value = 'basic'

  if (id) {
    const skillData = skillStore.getSkillById(id)
    if (!skillData) {
      showMessage(t('settings.skill.notFound'), 'error')
      return
    }
    editId.value = id
    skillForm.value = {
      icon: skillData.icon,
      logo: skillData.logo,
      name: skillData.name,
      prompt: skillData.prompt,
      description: skillData.metadata?.description || '',
      type: skillData.metadata?.type || '',
      useSystemRole: skillData.metadata?.useSystemRole || false,
      disabled: skillData.disabled,
    }
  } else {
    editId.value = null
    skillForm.value = {
      icon: '',
      name: '',
      prompt: '',
      description: '',
      type: '',
      useSystemRole: false,
      disabled: false,
    }
  }
  skillDialogVisible.value = true
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
  skillForm.value = {
    icon: skillData.icon,
    logo: skillData.logo,
    name: skillData.name + '-Copy',
    prompt: skillData.prompt,
    description: skillData.metadata?.description || '',
    type: skillData.metadata?.type || '',
    useSystemRole: skillData.metadata?.useSystemRole || false,
    disabled: skillData.disabled,
  }
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
        },
      }
      if (isEmpty(formData.logo) && isEmpty(formData.icon)) {
        showMessage(t('settings.skill.iconOrLogoRequired'), 'error')
        return
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
          showMessage(err, 'error')
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
    type: 'warning',
  }).then(() => {
    // User confirmed deletion
    skillStore
      .deleteSkill(id)
      .then(() => {
        showMessage(t('settings.skill.deleteSuccess'), 'success')
      })
      .catch(err => {
        showMessage(err, 'error')
      })
  })
}

/**
 * Handles the end of a drag event and updates the skill order.
 */
const onDragEnd = () => {
  skillStore.updateSkillOrder().catch(err => {
    console.error('settings.skill.updateOrderFailed', err)
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

// 添加 tab 激活状态控制
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
</style>
