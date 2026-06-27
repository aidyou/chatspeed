<template>
  <el-dialog
    v-model="visible"
    :close-on-press-escape="false"
    :close-on-click-modal="false"
    :title="dialogTitle"
    @open="prepareForm"
    width="600px"
    class="workflow-automation-dialog"
    destroy-on-close>
    <div class="automation-form-shell">
      <el-tabs v-model="activeTab" class="automation-tabs">
        <el-tab-pane :label="$t('workflow.automation.basicInfo')" name="basic">
          <el-form :model="form" label-position="left" label-width="120px" class="automation-form">
            <el-form-item :label="$t('workflow.automation.taskTitle')" required>
              <el-input v-model="form.title" />
            </el-form-item>

            <el-form-item
              :label="$t('workflow.automation.prompt')"
              :required="!form.promptFilePath.trim()">
              <el-input
                v-model="form.prompt"
                type="textarea"
                :autosize="{ minRows: 4, maxRows: 8 }" />
            </el-form-item>

            <el-form-item
              :label="$t('workflow.automation.promptFileLabel')"
              :required="!form.prompt.trim()">
              <div class="selection-field">
                <div class="selection-row">
                  <el-button class="selection-button" @click="selectPromptFile" round>
                    <cs name="folder" />
                    {{ $t('chat.selectFile') }}
                  </el-button>
                </div>
                <span v-if="form.promptFilePath" class="path-value" :title="form.promptFilePath">
                  {{ form.promptFilePath }}
                </span>
                <div class="field-hint">
                  {{ $t('workflow.automation.promptFileHint') }}
                </div>
              </div>
            </el-form-item>

            <el-form-item :label="$t('workflow.automation.agent')" required>
              <AgentSelector v-model="selectedAgent" class="button-like-selector" />
            </el-form-item>

            <el-form-item :label="$t('workflow.automation.model')">
              <el-button class="selection-button" @click="modelSelectorVisible = true" round>
                <cs name="setting" />
                {{ activeModelName }}
              </el-button>
            </el-form-item>

            <el-form-item :label="$t('workflow.automation.frequency')" required>
              <el-segmented v-model="form.scheduleKind" :options="frequencyOptions" />
            </el-form-item>

            <template v-if="form.scheduleKind === 'daily'">
              <el-form-item :label="$t('workflow.automation.executionTime')">
                <div class="daily-time-list">
                  <div v-for="(time, index) in dailyTimes" :key="index" class="daily-time-row">
                    <el-time-picker
                      v-model="dailyTimes[index]"
                      format="HH:mm"
                      value-format="HH:mm" />
                    <el-button
                      circle
                      plain
                      type="primary"
                      @click="addDailyTime"
                      :disabled="dailyTimes.length >= 24">
                      +
                    </el-button>
                    <el-button
                      circle
                      plain
                      type="danger"
                      @click="removeDailyTime(index)"
                      :disabled="dailyTimes.length <= 1">
                      -
                    </el-button>
                  </div>
                </div>
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
              <el-form-item :label="$t('workflow.automation.intervalMinutes')">
                <el-input-number v-model="form.intervalMinutes" :min="5" :max="259200" :step="5" />
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
          </el-form>
        </el-tab-pane>

        <el-tab-pane :label="$t('workflow.automation.additionalInfo')" name="advanced">
          <el-form :model="form" label-position="left" label-width="120px" class="automation-form">
            <el-form-item :label="$t('workflow.automation.allowedDirectories')">
              <div class="selection-field paths-field">
                <div class="selection-row">
                  <el-button class="selection-button" @click="selectAllowedDirectory" round>
                    <cs name="folder" />
                    {{ $t('workflow.automation.addDirectory') }}
                  </el-button>
                </div>
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

            <el-form-item :label="$t('workflow.automation.shellCommand')">
              <div class="selection-field">
                <el-input
                  v-model="form.shellCommand"
                  :placeholder="$t('workflow.automation.shellCommandPlaceholder')" />
                <div class="field-hint">
                  {{ $t('workflow.automation.shellCommandHint') }}
                </div>
              </div>
            </el-form-item>

            <el-form-item
              v-if="showContinuousContext"
              :label="$t('workflow.automation.continuousContext')">
              <div class="selection-field">
                <el-switch v-model="form.continuousContext" />
                <div class="field-hint">
                  {{ $t('workflow.automation.continuousContextHint') }}
                </div>
              </div>
            </el-form-item>

            <el-form-item :label="$t('workflow.automation.selfReview')">
              <el-switch v-model="form.selfReview" />
            </el-form-item>

            <el-form-item :label="$t('workflow.automation.start')">
              <el-switch v-model="form.enabled" />
            </el-form-item>
          </el-form>
        </el-tab-pane>
      </el-tabs>
    </div>

    <template #footer>
      <div class="automation-actions">
        <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
        <el-button v-if="form.id" type="danger" plain @click="deleteAutomation">
          {{ $t('common.delete') }}
        </el-button>
        <el-button v-if="form.id" @click="runNow" :loading="runningNow">
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
  </el-dialog>
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
  shellCommand: '',
  scheduleKind: 'daily',
  weekdays: allWeekdays(),
  startDate: '',
  endDate: '',
  intervalMinutes: 5,
  continuousContext: false,
  selfReview: false,
  enabled: true
})

const form = reactive(defaultForm())
const selectedAgent = ref(null)
const defaultDailyTime = () => '09:00'
const normalizeDailyTimes = values => {
  const normalized = Array.isArray(values)
    ? values.map(value => (typeof value === 'string' ? value : '')).filter(Boolean)
    : []
  return normalized.length > 0 ? Array.from(new Set(normalized)).sort() : [defaultDailyTime()]
}
const dailyTimes = ref([defaultDailyTime()])
const onceRunAt = ref('')
const activeTab = ref('basic')
const saving = ref(false)
const runningNow = ref(false)
const modelSelectorVisible = ref(false)
const showContinuousContext = computed(() => form.scheduleKind !== 'once')

const dialogTitle = computed(() =>
  form.id ? t('workflow.automation.edit') : t('workflow.automation.createTitle')
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
      h('div', { class: 'weekday-picker-with-label' }, [
        h('div', { class: 'field-label' }, t('workflow.automation.weekdays')),
        h(
          'div',
          { class: 'weekday-picker-content' },
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

const hasPromptSource = computed(() => Boolean(form.prompt.trim() || form.promptFilePath.trim()))

const hasValidDailyTimes = computed(
  () => normalizeDailyTimes(dailyTimes.value).length > 0 && dailyTimes.value.every(Boolean)
)

const hasCompleteEffectiveRange = computed(
  () => (!form.startDate && !form.endDate) || Boolean(form.startDate && form.endDate)
)

const canSaveAutomation = computed(() => {
  if (!form.title.trim()) return false
  if (!hasPromptSource.value) return false
  if (!selectedAgent.value?.id) return false
  if (!hasCompleteEffectiveRange.value) return false

  if (form.scheduleKind === 'daily') {
    return Boolean(hasValidDailyTimes.value && form.weekdays.length > 0)
  }
  if (form.scheduleKind === 'interval') {
    return Boolean(form.intervalMinutes >= 5 && form.weekdays.length > 0)
  }
  return Boolean(onceRunAt.value)
})

const addDailyTime = () => {
  if (dailyTimes.value.length >= 24) return
  dailyTimes.value = [...dailyTimes.value, defaultDailyTime()]
}

const removeDailyTime = index => {
  if (dailyTimes.value.length <= 1) return
  dailyTimes.value = dailyTimes.value.filter((_, currentIndex) => currentIndex !== index)
}

const resetForm = () => {
  Object.assign(form, defaultForm())
  selectedAgent.value = agentStore.primaryAgents[0] || null
  dailyTimes.value = [defaultDailyTime()]
  onceRunAt.value = ''
  activeTab.value = 'basic'
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
    shellCommand:
      automation.shellConfig?.command ||
      [automation.shellConfig?.filePath, automation.shellConfig?.args].filter(Boolean).join(' '),
    scheduleKind: automation.scheduleKind || 'daily',
    weekdays:
      Array.isArray(automation.scheduleConfig?.weekdays) &&
      automation.scheduleConfig.weekdays.length > 0
        ? [...automation.scheduleConfig.weekdays]
        : allWeekdays(),
    startDate: automation.scheduleConfig?.start_date || automation.scheduleConfig?.startDate || '',
    endDate: automation.scheduleConfig?.end_date || automation.scheduleConfig?.endDate || '',
    intervalMinutes:
      automation.scheduleConfig?.interval_minutes ||
      automation.scheduleConfig?.intervalMinutes ||
      (automation.scheduleConfig?.interval_hours || automation.scheduleConfig?.intervalHours || 1) *
        60,
    continuousContext: Boolean(automation.continuousContext),
    selfReview: Boolean(automation.selfReview),
    enabled: automation.enabled !== false
  })
  selectedAgent.value =
    agentStore.agents.find(agent => agent.id === automation.agentId) ||
    agentStore.primaryAgents[0] ||
    null
  dailyTimes.value = normalizeDailyTimes(
    automation.scheduleConfig?.times || [automation.scheduleConfig?.time || defaultDailyTime()]
  )
  onceRunAt.value = automation.scheduleConfig?.run_at || automation.scheduleConfig?.runAt || ''
}

const scheduleConfig = () => {
  if (form.scheduleKind === 'daily') {
    const times = normalizeDailyTimes(dailyTimes.value)
    return {
      time: times[0] || defaultDailyTime(),
      times,
      weekdays: form.weekdays,
      start_date: form.startDate || null,
      end_date: form.endDate || null
    }
  }
  if (form.scheduleKind === 'interval') {
    return {
      interval_minutes: Math.max(5, form.intervalMinutes || 5),
      weekdays: form.weekdays,
      start_date: form.startDate || null,
      end_date: form.endDate || null,
      anchor_time: dailyTimes.value[0] || defaultDailyTime()
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
  shellConfig: form.shellCommand.trim()
    ? {
        command: form.shellCommand.trim()
      }
    : null,
  scheduleKind: form.scheduleKind,
  scheduleConfig: scheduleConfig(),
  continuousContext: form.scheduleKind === 'once' ? false : form.continuousContext,
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

watch(
  () => form.scheduleKind,
  scheduleKind => {
    if (scheduleKind === 'once') {
      form.continuousContext = false
    }
  }
)
</script>

<style lang="scss" scoped>
.automation-form-shell {
  display: flex;
  flex-direction: column;
  max-height: 60vh;
  overflow-y: auto;
}

.automation-tabs {
  :deep(.el-tabs__header) {
    margin-bottom: var(--cs-space-md);
  }
}

.automation-form {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-md);

  :deep(.el-form-item) {
    margin-bottom: 0;
    align-items: flex-start;
  }

  :deep(.el-form-item__label) {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    min-height: 36px;
    padding-right: var(--cs-space-md);
    font-size: var(--cs-font-size-sm);
    font-weight: 500;
    color: var(--cs-text-primary);
    line-height: 1.4;
  }

  :deep(.el-form-item__label::before) {
    margin-right: 2px;
  }

  :deep(.el-form-item__content) {
    flex: 1;
    line-height: 1.4;
  }

  :deep(.el-input__wrapper),
  :deep(.el-textarea__inner) {
    border-radius: var(--cs-border-radius);
  }

  :deep(.el-textarea__inner) {
    padding: var(--cs-space-sm);
    line-height: 1.5;
  }

  :deep(.el-button) {
    min-height: 36px;
  }

  :deep(.el-segmented) {
    --el-segmented-item-selected-color: var(--cs-color-primary);
    --el-segmented-item-selected-bg-color: var(--cs-bg-color);
    border-radius: var(--cs-border-radius);
  }

  :deep(.el-date-editor),
  :deep(.el-input-number) {
    width: 100%;
  }

  :deep(.el-tag) {
    border-radius: var(--cs-border-radius-sm);
  }

  :deep(.el-switch) {
    --el-switch-height: 26px;
    --el-switch-on-color: var(--cs-color-primary);
  }
}

.daily-time-list {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm);
  width: 100%;
}

.daily-time-row {
  display: flex;
  align-items: center;
  gap: var(--cs-space-sm);
  width: 100%;

  :deep(.el-date-editor) {
    flex: 1;
  }

  :deep(.el-button) {
    flex-shrink: 0;
    width: 30px;
    height: 30px;
    min-height: 30px;
    border-radius: var(--cs-border-radius);
    padding: 0;
    margin: 0;

    &:last-child {
      margin-right: 2px;
    }
  }
}

.selection-row {
  display: flex;
  align-items: center;
  width: 100%;
}

.selection-button {
  flex: 1;
  width: 100%;
  padding: 0 14px;
}

:deep(.selection-button.el-button) {
  --el-button-bg-color: var(--cs-fill-color-light);
  --el-button-border-color: var(--cs-border-color);
  --el-button-hover-bg-color: var(--cs-fill-color);
  --el-button-hover-border-color: var(--cs-color-primary-light);
  --el-button-text-color: var(--cs-text-primary);
  --el-button-hover-text-color: var(--cs-color-primary);
  box-shadow: none;
}

.field-hint,
.path-value {
  font-size: var(--cs-font-size-sm);
  color: var(--cs-text-color-secondary);
}

.path-value {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.paths-field {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm);
  flex: 1;
  overflow: hidden;
}

.path-list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--cs-space-xs);

  :deep(.el-tag) {
    max-width: 100%;

    .el-tag__content {
      overflow: hidden;
      text-overflow: ellipsis;
    }
  }
}

.weekday-picker-with-label {
  display: flex;
  align-items: flex-start;
  gap: 0;
  padding: 0;
  background: transparent;
  border-radius: 0;

  :deep(.field-label) {
    width: 120px;
    min-height: 36px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: var(--cs-space-md);
    text-align: right;
    font-size: var(--cs-font-size-sm);
    font-weight: 500;
    color: var(--cs-text-primary);
    line-height: 1.4;
    flex-shrink: 0;
    box-sizing: border-box;
  }

  :deep(.weekday-picker-content) {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    padding: 0;
  }

  :deep(.weekday-button) {
    height: 32px;
    padding: 0 var(--cs-space-sm);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius);
    background: var(--cs-bg-color);
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-sm);
    cursor: pointer;
    transition: all 0.2s ease;

    &.active {
      background: var(--cs-color-primary);
      color: #fff !important;
      border-color: var(--cs-color-primary);
    }

    &:hover {
      border-color: var(--cs-color-primary-light);
      color: var(--cs-color-primary) !important;
      background: none;
    }
  }
}

.automation-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--cs-space-sm);
  padding: var(--cs-space-md) 0;

  :deep(.el-button) {
    min-width: 96px;
  }
}

.agent-selector {
  flex: 1;
}

:deep(.button-like-selector .el-dropdown) {
  display: block;
  width: 100%;
}

:deep(.button-like-selector .el-dropdown-link) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--cs-space-sm);
  width: 100%;
  min-height: 32px;
  padding: 0 14px;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--el-border-radius-round);
  background: var(--cs-fill-color-light);
  color: var(--cs-text-primary);
  box-sizing: border-box;
  box-shadow: none;
  transition:
    border-color 0.2s ease,
    color 0.2s ease,
    background-color 0.2s ease;
}

:deep(.button-like-selector .el-dropdown-link:hover) {
  border-color: var(--cs-color-primary-light);
  color: var(--cs-color-primary);
  background: var(--cs-fill-color);
}

:deep(.button-like-selector .agent-name) {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
