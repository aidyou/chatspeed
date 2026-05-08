<template>
  <el-dialog
    v-model="visible"
    :title="$t('workflow.skillsConfigTitle')"
    width="560px"
    custom-class="workflow-skills-selector-dialog"
    :before-close="handleClose">
    <div class="skills-selector-content">
      <el-form label-width="100px">
        <el-form-item :label="$t('settings.agent.skillEnabled')">
          <el-switch v-model="skillEnabled" />
        </el-form-item>

        <template v-if="skillEnabled">
          <el-form-item :label="$t('common.search')">
            <el-input
              v-model="searchKeyword"
              clearable
              :placeholder="$t('workflow.skillsSearchPlaceholder')" />
          </el-form-item>

          <el-form-item :label="$t('settings.agent.selectedSkills')">
            <div v-if="filteredSkills.length" class="skill-checklist">
              <el-checkbox-group v-model="selectedSkills" class="skill-checklist__group">
                <label v-for="skill in filteredSkills" :key="skill.name" class="skill-checklist__item">
                  <el-checkbox :value="skill.name">
                    <span class="skill-checklist__name">{{ skill.name }}</span>
                  </el-checkbox>
                  <span
                    v-if="skill.description"
                    class="skill-checklist__description"
                    :title="skill.description">
                    {{ skill.description }}
                  </span>
                </label>
              </el-checkbox-group>
            </div>
            <div class="form-tip">{{ $t('settings.agent.skillsHint') }}</div>
            <div v-if="!filteredSkills.length" class="form-tip">
              {{ emptyStateText }}
            </div>
          </el-form-item>
        </template>
      </el-form>
    </div>
    <template #footer>
      <div class="dialog-footer">
        <el-button size="small" @click="visible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" size="small" @click="handleSave">{{ $t('common.save') }}</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  modelValue: Boolean,
  currentWorkflow: {
    type: Object,
    default: null
  },
  agent: {
    type: Object,
    default: null
  },
  systemSkills: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['update:modelValue', 'save'])

const { t } = useI18n()
const ALWAYS_ENABLED_SKILL_NAMES = ['help']

const visible = computed({
  get: () => props.modelValue,
  set: value => emit('update:modelValue', value)
})

const skillEnabled = ref(true)
const selectedSkills = ref([])
const searchKeyword = ref('')

const selectableSkills = computed(() => {
  return [...props.systemSkills]
    .filter(skill => !ALWAYS_ENABLED_SKILL_NAMES.includes(skill.name))
    .sort((a, b) => {
      if (a.source !== b.source) {
        return a.source === 'user' ? -1 : 1
      }
      return a.name.localeCompare(b.name, 'zh-Hans')
    })
})

const filteredSkills = computed(() => {
  const query = searchKeyword.value.trim().toLowerCase()
  if (!query) return selectableSkills.value
  return selectableSkills.value.filter(skill => skill.name.toLowerCase().includes(query))
})

const emptyStateText = computed(() => {
  if (!selectableSkills.value.length) {
    return t('settings.agent.noSkillsAvailable')
  }
  return t('workflow.skillsSearchEmpty')
})

const sortSelectedSkillNames = (skillNames) => {
  if (!Array.isArray(skillNames)) return []
  const allowedNames = new Set(selectableSkills.value.map(skill => skill.name))
  return [...new Set(skillNames)]
    .filter(name => allowedNames.has(name))
    .sort((a, b) => a.localeCompare(b, 'zh-Hans'))
}

const defaultSelectedSkillNames = () => selectableSkills.value.map(skill => skill.name)

const initFromSource = () => {
  searchKeyword.value = ''

  const workflowConfig = props.currentWorkflow?.agentConfig || null
  const source = workflowConfig || props.agent || null

  if (!source) {
    skillEnabled.value = true
    selectedSkills.value = defaultSelectedSkillNames()
    return
  }

  skillEnabled.value = source.skillEnabled !== false

  if (Array.isArray(source.selectedSkills)) {
    selectedSkills.value = sortSelectedSkillNames(source.selectedSkills)
    return
  }

  selectedSkills.value = defaultSelectedSkillNames()
}

const handleClose = (done) => {
  visible.value = false
  done()
}

const handleSave = () => {
  emit('save', {
    skillEnabled: skillEnabled.value !== false,
    selectedSkills: skillEnabled.value ? sortSelectedSkillNames(selectedSkills.value) : []
  })
  visible.value = false
}

watch(visible, (isVisible) => {
  if (isVisible) {
    initFromSource()
  }
})

watch(selectableSkills, () => {
  if (visible.value) {
    initFromSource()
  }
})
</script>

<style lang="scss" scoped>
.skills-selector-content {
  padding: 4px;
}

.skill-checklist {
  width: 100%;
  max-height: 320px;
  overflow-y: auto;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  background: var(--cs-bg-color-light);
  padding: 8px 10px;
}

.skill-checklist__group {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
}

.skill-checklist__item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
}

.skill-checklist__name {
  font-family: Monaco, Menlo, Consolas, 'Courier New', monospace;
  font-size: 13px;
}

.skill-checklist__description {
  color: var(--cs-text-color-secondary);
  font-size: 12px;
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding-left: 24px;
}

.form-tip {
  color: var(--cs-text-color-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
