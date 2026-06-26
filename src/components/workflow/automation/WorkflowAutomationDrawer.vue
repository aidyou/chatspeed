<template>
  <el-drawer
    v-model="visible"
    :title="drawerTitle"
    direction="ltr"
    size="500px"
    class="workflow-automation-drawer"
    @open="prepareForm">
    <div class="automation-form-shell">
      <el-form
        :model="form"
        label-position="top"
        class="automation-form">
        <el-form-item :label="$t('workflow.automation.taskTitle')" required>
          <el-input v-model="form.title" />
        </el-form-item>

        <el-form-item :label="$t('workflow.automation.prompt')" required>
          <el-input
            v-model="form.prompt"
            type="textarea"
            :autosize="{ minRows: 4, maxRows: 8 }" />
          <div class="prompt-file-row">
            <el-button size="small" @click="selectPromptFile">
              <cs name="folder" />
              {{ $t('workflow.automation.promptFile') }}
            </el-button>
            <span v-if="form.promptFilePath" class="path-value" :title="form.promptFilePath">
              {{ form.promptFilePath }}
            </span>
          </div>
        </el-form-item>

        <el-form-item :label="$t('workflow.automation.agent')" required>
          <AgentSelector v-model="selectedAgent" />
        </el-form-item>

        <el-form-item :label="$t('workflow.automation.model')">
          <el-button @click="modelSelectorVisible = true">
            <cs name="setting" />
            {{ activeModelName }}
          </el-button>
        </el-form-item>

        <el-form-item :label="$t('workflow.automation.allowedDirectories')">
          <div class="paths-field">
            <el-button size="small" @click="selectAllowedDirectory">
              <cs name="folder" />
              {{ $t('workflow.automation.addDirectory') }}
            </el-button>
            <div v-if="form.allowedPaths.length > 0" class="path-list">
              <el-tag
                v-for="path in form.allowedPaths"
                :key="path"
                closable
                @close="removeAllowedPath(path)">
                {{ path }}
              </el-tag>
            </div>
          </div>
        </el-form-item>

        <el-form-item :label="$t('workflow.automation.frequency')" required>
          <el-segmented
            v-model="form.scheduleKind"
            :options="frequencyOptions" />
        </el-form-item>

        <template v-if="form.scheduleKind === 'daily'">
          <el-form-item :label="$t('workflow.automation.executionTime')">
            <el-time-picker v-model="dailyTime" format="HH:mm" value-format="HH:mm" />
          </el-form-item>
          <WeekdayPicker v-model="form.weekdays" />
          <el-form-item :label="$t('workflow.automation.effectiveRange')">
            <el-date-picker
              v-model="effectiveDateRange"
              type="daterange"
              format="YYYY-MM-DD"
              value-format="YYYY-MM-DD"
              :start-placeholder="$t('workflow.automation.startDate')"
              :end-placeholder="$t('workflow.automation.endDate')"
              clearable />
          </el-form-item>
        </template>

        <template v-else-if="form.scheduleKind === 'interval'">
          <el-form-item :label="$t('workflow.automation.intervalHours')">
            <el-input-number v-model="form.intervalHours" :min="1" :max="720" />
          </el-form-item>
          <WeekdayPicker v-model="form.weekdays" />
          <el-form-item :label="$t('workflow.automation.effectiveRange')">
            <el-date-picker
              v-model="effectiveDateRange"
              type="daterange"
              format="YYYY-MM-DD"
              value-format="YYYY-MM-DD"
              :start-placeholder="$t('workflow.automation.startDate')"
              :end-placeholder="$t('workflow.automation.endDate')"
              clearable />
          </el-form-item>
        </template>

        <template v-else>
          <el-form-item :label="$t('workflow.automation.executionDate')">
            <el-date-picker
              v-model="onceRunAt"
              type="datetime"
              format="YYYY-MM-DD HH:mm"
              value-format="YYYY-MM-DD HH:mm" />
          </el-form-item>
        </template>

        <el-form-item>
          <el-checkbox v-model="form.selfReview">
            {{ $t('workflow.automation.selfReview') }}
          </el-checkbox>
        </el-form-item>
      </el-form>
    </div>

    <template #footer>
      <div class="automation-actions">
        <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
        <el-button
          v-if="form.id"
          type="danger"
          plain
          @click="deleteAutomation">
          {{ $t('common.delete') }}
        </el-button>
        <el-button
          v-if="form.id"
          @click="runNow"
          :loading="runningNow">
          <cs name="play" />
          {{ $t('workflow.automation.runNow') }}
        </el-button>
        <el-button
          type="primary"
          :loading="saving"
          :disabled="!canSaveAutomation"
          @click="saveAutomation">
          {{ $t('common.save') }}
        </el-button>
      </div>
    </template>

    <WorkflowModelSelector
      v-model="modelSelectorVisible"
      :agent="selectedAgent"
      :initial-models="form.agentConfig?.models"
      @save="onModelConfigSave" />
  </el-drawer>
</template>

<script setup>
import { computed, defineComponent, h, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAgentStore } from '@/stores/agent'
import { useWorkflowAutomationStore } from '@/stores/workflowAutomation'
import AgentSelector from '@/components/workflow/AgentSelector.vue'
import WorkflowModelSelector from '@/components/workflow/WorkflowModelSelector.vue'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['update:modelValue', 'started-workflow', 'saved'])
const { t } = useI18n()
const agentStore = useAgentStore()
const automationStore = useWorkflowAutomationStore()

const visible = computed({
  get: () => props.modelValue,
  set: value => emit('update:modelValue', value)
})

const allWeekdays = () => [1, 2, 3, 4, 5, 6, 7]

const defaultForm = () => ({
  id: null,
  title: '',
  prompt: '',
  promptFilePath: '',
  agentConfig: null,
  allowedPaths: [],
  scheduleKind: 'daily',
  weekdays: allWeekdays(),
  startDate: '',
  endDate: '',
  intervalHours: 1,
  selfReview: false,
  enabled: true
})

const form = reactive(defaultForm())
const selectedAgent = ref(null)
const dailyTime = ref('09:00')
const onceRunAt = ref('')
const saving = ref(false)
const runningNow = ref(false)
const modelSelectorVisible = ref(false)

const drawerTitle = computed(() =>
  form.id ? t('workflow.automation.edit') : t('workflow.automation.create')
)

const frequencyOptions = computed(() => [
  { label: t('workflow.automation.daily'), value: 'daily' },
  { label: t('workflow.automation.interval'), value: 'interval' },
  { label: t('workflow.automation.once'), value: 'once' }
])

const weekdayOptions = computed(() => [
  { label: t('workflow.automation.weekday.mon'), value: 1 },
  { label: t('workflow.automation.weekday.tue'), value: 2 },
  { label: t('workflow.automation.weekday.wed'), value: 3 },
  { label: t('workflow.automation.weekday.thu'), value: 4 },
  { label: t('workflow.automation.weekday.fri'), value: 5 },
  { label: t('workflow.automation.weekday.sat'), value: 6 },
  { label: t('workflow.automation.weekday.sun'), value: 7 }
])

const WeekdayPicker = defineComponent({
  props: {
    modelValue: {
      type: Array,
      default: () => []
    }
  },
  emits: ['update:modelValue'],
  setup(componentProps, { emit: componentEmit }) {
    const toggle = value => {
      const next = new Set(componentProps.modelValue)
      if (next.has(value)) next.delete(value)
      else next.add(value)
      componentEmit('update:modelValue', Array.from(next).sort())
    }
    return () =>
      h('div', { class: 'weekday-picker' }, [
        h('div', { class: 'field-label' }, t('workflow.automation.weekdays')),
        h(
          'div',
          { class: 'weekday-buttons' },
          weekdayOptions.value.map(option =>
            h(
              'button',
              {
                type: 'button',
                class: {
                  'weekday-button': true,
                  active: componentProps.modelValue.includes(option.value)
                },
                onClick: () => toggle(option.value)
              },
              option.label
            )
          )
        )
      ])
  }
})

const activeModelName = computed(() => {
  const model = form.agentConfig?.models?.act || selectedAgent.value?.actModel
  return model?.model || t('workflow.automation.model')
})

const effectiveDateRange = computed({
  get: () => {
    if (!form.startDate && !form.endDate) return []
    return [form.startDate || '', form.endDate || '']
  },
  set: value => {
    const [start = '', end = ''] = Array.isArray(value) ? value : []
    form.startDate = start || ''
    form.endDate = end || ''
  }
})

const hasPromptSource = computed(() =>
  Boolean(form.prompt.trim() || form.promptFilePath.trim())
)

const hasCompleteEffectiveRange = computed(() =>
  (!form.startDate && !form.endDate) || Boolean(form.startDate && form.endDate)
)

const canSaveAutomation = computed(() => {
  if (!form.title.trim()) return false
  if (!hasPromptSource.value) return false
  if (!selectedAgent.value?.id) return false
  if (!hasCompleteEffectiveRange.value) return false

  if (form.scheduleKind === 'daily') {
    return Boolean(dailyTime.value && form.weekdays.length > 0)
  }
  if (form.scheduleKind === 'interval') {
    return Boolean(form.intervalHours > 0 && form.weekdays.length > 0)
  }
  return Boolean(onceRunAt.value)
})

const resetForm = () => {
  Object.assign(form, defaultForm())
  selectedAgent.value = agentStore.primaryAgents[0] || null
  dailyTime.value = '09:00'
  onceRunAt.value = ''
}

const prepareForm = async () => {
  if (agentStore.agents.length === 0) {
    await agentStore.fetchAgents()
  }
  if (!selectedAgent.value) {
    selectedAgent.value = agentStore.primaryAgents[0] || null
  }
  await automationStore.fetchAutomations()
  if (automationStore.selectedAutomationId) {
    const selected = automationStore.automations.find(
      item => item.id === automationStore.selectedAutomationId
    )
    if (selected) {
      applyAutomationToForm(selected)
    }
  } else {
    resetForm()
  }
}

const applyAutomationToForm = automation => {
  Object.assign(form, {
    id: automation.id,
    title: automation.title || '',
    prompt: automation.prompt || '',
    promptFilePath: automation.promptFilePath || '',
    agentConfig: automation.agentConfig || null,
    allowedPaths: Array.isArray(automation.allowedPaths) ? [...automation.allowedPaths] : [],
    scheduleKind: automation.scheduleKind || 'daily',
    weekdays: Array.isArray(automation.scheduleConfig?.weekdays) &&
      automation.scheduleConfig.weekdays.length > 0
      ? [...automation.scheduleConfig.weekdays]
      : allWeekdays(),
    startDate: automation.scheduleConfig?.start_date || automation.scheduleConfig?.startDate || '',
    endDate: automation.scheduleConfig?.end_date || automation.scheduleConfig?.endDate || '',
    intervalHours:
      automation.scheduleConfig?.interval_hours || automation.scheduleConfig?.intervalHours || 1,
    selfReview: Boolean(automation.selfReview),
    enabled: automation.enabled !== false
  })
  selectedAgent.value =
    agentStore.agents.find(agent => agent.id === automation.agentId) ||
    agentStore.primaryAgents[0] ||
    null
  dailyTime.value = automation.scheduleConfig?.time || '09:00'
  onceRunAt.value = automation.scheduleConfig?.run_at || automation.scheduleConfig?.runAt || ''
}

const scheduleConfig = () => {
  if (form.scheduleKind === 'daily') {
    return {
      time: dailyTime.value || '09:00',
      weekdays: form.weekdays,
      start_date: form.startDate || null,
      end_date: form.endDate || null
    }
  }
  if (form.scheduleKind === 'interval') {
    return {
      interval_hours: form.intervalHours || 1,
      weekdays: form.weekdays,
      start_date: form.startDate || null,
      end_date: form.endDate || null,
      anchor_time: dailyTime.value || '09:00'
    }
  }
  return {
    run_at: onceRunAt.value
  }
}

const automationRequest = () => ({
  id: form.id,
  title: form.title,
  prompt: form.prompt,
  promptFilePath: form.promptFilePath,
  agentId: selectedAgent.value?.id || '',
  agentConfig: form.agentConfig,
  allowedPaths: form.allowedPaths,
  scheduleKind: form.scheduleKind,
  scheduleConfig: scheduleConfig(),
  selfReview: form.selfReview,
  enabled: form.enabled
})

const saveAutomation = async () => {
  if (!canSaveAutomation.value) return
  saving.value = true
  try {
    const saved = await automationStore.saveAutomation(automationRequest())
    applyAutomationToForm(saved)
    ElMessage.success(t('workflow.automation.saveSuccess'))
    emit('saved', saved)
    visible.value = false
  } catch (error) {
    ElMessage.error(error?.message || String(error))
  } finally {
    saving.value = false
  }
}

const deleteAutomation = async () => {
  try {
    await ElMessageBox.confirm(
      t('workflow.automation.deleteConfirm'),
      t('workflow.automation.delete'),
      { type: 'warning' }
    )
  } catch {
    return
  }

  try {
    await automationStore.deleteAutomation(form.id)
    resetForm()
    visible.value = false
  } catch (error) {
    ElMessage.error(error?.message || String(error))
  }
}

const runNow = async () => {
  if (!form.id) return
  runningNow.value = true
  try {
    const result = await automationStore.runAutomationNow(form.id)
    emit('started-workflow', result.workflowSessionId)
    ElMessage.success(t('workflow.automation.runStarted'))
    visible.value = false
  } catch (error) {
    ElMessage.error(error?.message || String(error))
  } finally {
    runningNow.value = false
  }
}

const selectPromptFile = async () => {
  const selected = await open({
    multiple: false,
    directory: false
  })
  if (typeof selected === 'string') {
    form.promptFilePath = selected
  }
}

const selectAllowedDirectory = async () => {
  const selected = await open({
    multiple: true,
    directory: true
  })
  const paths = Array.isArray(selected) ? selected : selected ? [selected] : []
  for (const path of paths) {
    if (!form.allowedPaths.includes(path)) {
      form.allowedPaths.push(path)
    }
  }
}

const removeAllowedPath = path => {
  form.allowedPaths = form.allowedPaths.filter(item => item !== path)
}

const onModelConfigSave = models => {
  form.agentConfig = {
    ...(form.agentConfig || {}),
    models
  }
}

watch(
  () => agentStore.primaryAgents,
  agents => {
    if (!selectedAgent.value && agents.length > 0) {
      selectedAgent.value = agents[0]
    }
  },
  { immediate: true }
)

watch(
  () => automationStore.selectedAutomationId,
  id => {
    if (!visible.value) return
    if (!id) {
      resetForm()
      return
    }
    const selected = automationStore.automations.find(item => item.id === id)
    if (selected) {
      applyAutomationToForm(selected)
    }
  }
)
</script>

<style lang="scss" scoped>
.automation-form-shell {
  display: flex;
  flex-direction: column;
  min-height: 100%;
}

.automation-actions {
  display: flex;
  align-items: center;
  gap: var(--cs-space-sm);
}

.path-value {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.path-value,
.field-label {
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
}

.prompt-file-row,
.paths-field,
.path-list,
.weekday-picker {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-xs);
  width: 100%;
}

.prompt-file-row {
  margin-top: var(--cs-space-xs);
}

.automation-form {
  display: flex;
  flex: 1;
  flex-direction: column;
}

.path-list {
  align-items: flex-start;
}

.weekday-buttons {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
}

:deep(.el-date-editor.el-input__wrapper) {
  width: 100%;
}

.weekday-button {
  width: 32px;
  height: 32px;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-round);
  background: var(--cs-bg-color);
  color: var(--cs-text-color-secondary);
  cursor: pointer;

  &.active {
    background: var(--cs-color-primary);
    color: var(--cs-text-color-primary);
    border-color: var(--cs-color-primary);
  }
}

.automation-actions {
  justify-content: flex-end;
}
</style>
