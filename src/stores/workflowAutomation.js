import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useSettingStore } from '@/stores/setting'

const parseJsonField = (value, fallback) => {
  if (value === null || value === undefined || value === '') return fallback
  if (typeof value !== 'string') return value
  try {
    return JSON.parse(value)
  } catch {
    return fallback
  }
}

const normalizeAutomation = item => ({
  ...item,
  allowedPaths: parseJsonField(item.allowedPaths, []),
  agentConfig: parseJsonField(item.agentConfig, null),
  shellConfig: parseJsonField(item.shellConfig, null),
  scheduleConfig: parseJsonField(item.scheduleConfig, {}),
  continuousContext: Boolean(item.continuousContext),
  currentWorkflowSessionId: item.currentWorkflowSessionId || null
})

export const useWorkflowAutomationStore = defineStore('workflowAutomation', () => {
  const settingStore = useSettingStore()
  const automations = ref([])
  const runsByAutomation = ref({})
  const projectedRunsByAutomation = ref({})
  const loading = ref(false)
  const error = ref(null)
  const selectedAutomationId = ref(null)

  const selectedAutomation = computed(() =>
    automations.value.find(item => item.id === selectedAutomationId.value) || null
  )

  const persistSelectedAutomationId = automationId => {
    void settingStore
      .setSetting('workflowAutomationLastSelectedId', automationId || '')
      .catch(error => {
        console.warn('[WorkflowAutomation] Failed to persist last selected automation id:', error)
      })
  }

  const selectAutomation = automationId => {
    selectedAutomationId.value = automationId || null
    persistSelectedAutomationId(selectedAutomationId.value)
  }

  const toAutomationRequest = automation => ({
    id: automation.id,
    title: automation.title || '',
    prompt: automation.prompt || '',
    promptFilePath: automation.promptFilePath || '',
    agentId: automation.agentId || '',
    agentConfig: automation.agentConfig || null,
    allowedPaths: Array.isArray(automation.allowedPaths) ? [...automation.allowedPaths] : [],
    shellConfig: automation.shellConfig || null,
    scheduleKind: automation.scheduleKind || 'daily',
    scheduleConfig: automation.scheduleConfig || {},
    continuousContext: Boolean(automation.continuousContext),
    selfReview: Boolean(automation.selfReview),
    enabled: automation.enabled !== false
  })

  const fetchAutomations = async () => {
    loading.value = true
    error.value = null
    try {
      const result = await invokeWrapper('workflow_automation_list')
      automations.value = (result || []).map(normalizeAutomation)
      if (
        selectedAutomationId.value &&
        !automations.value.some(item => item.id === selectedAutomationId.value)
      ) {
        selectedAutomationId.value = null
      }
      return automations.value
    } catch (err) {
      error.value = err.message || String(err)
      throw err
    } finally {
      loading.value = false
    }
  }

  const saveAutomation = async request => {
    loading.value = true
    error.value = null
    try {
      const result = await invokeWrapper('workflow_automation_save', { request })
      const automation = normalizeAutomation(result)
      const index = automations.value.findIndex(item => item.id === automation.id)
      if (index >= 0) {
        automations.value.splice(index, 1, automation)
      } else {
        automations.value.unshift(automation)
      }
      selectAutomation(automation.id)
      return automation
    } catch (err) {
      error.value = err.message || String(err)
      throw err
    } finally {
      loading.value = false
    }
  }

  const deleteAutomation = async (id, confirm = false) => {
    // Destructive: the caller's confirm dialog must set `confirm` true, matching
    // the backend safety gate (AC-10). A refused cascade surfaces as an error.
    await invokeWrapper('workflow_automation_delete', { id, confirm })
    automations.value = automations.value.filter(item => item.id !== id)
    if (selectedAutomationId.value === id) {
      selectAutomation(automations.value[0]?.id || null)
    }
  }

  const updateAutomationAllowedPaths = async (id, allowedPaths) => {
    const automation = automations.value.find(item => item.id === id)
    if (!automation) {
      throw new Error(`Automation not found: ${id}`)
    }

    return saveAutomation({
      ...toAutomationRequest(automation),
      allowedPaths
    })
  }

  const setAutomationEnabled = async (id, enabled) => {
    await invokeWrapper('workflow_automation_set_enabled', { id, enabled })
    await fetchAutomations()
  }

  const runAutomationNow = async automationId => {
    const result = await invokeWrapper('workflow_automation_run_now', { automationId })
    await fetchRuns(automationId)
    return result
  }

  const fetchRuns = async automationId => {
    if (!automationId) return []
    const result = await invokeWrapper('workflow_automation_list_runs', { automationId })
    runsByAutomation.value = {
      ...runsByAutomation.value,
      [automationId]: result || []
    }
    return runsByAutomation.value[automationId]
  }

  // Maps a canonical snake_case projected run view to camelCase for the UI. The
  // status/workflowStatus/waitReason come straight from the durable snapshot
  // projection; the UI never infers lifecycle from transcript text (AC-7/INV-7).
  const normalizeRunView = run => ({
    ...run,
    runId: run.run_id,
    automationId: run.automation_id,
    dispatchKey: run.dispatch_key ?? null,
    workflowSessionId: run.workflow_session_id ?? null,
    scheduledFor: run.scheduled_for,
    startedAt: run.started_at ?? null,
    finishedAt: run.finished_at ?? null,
    workflowStatus: run.workflow_status ?? null,
    waitReason: run.wait_reason ?? null,
    createdAt: run.created_at ?? null,
    updatedAt: run.updated_at ?? null
  })

  const fetchProjectedRuns = async automationId => {
    if (!automationId) return []
    const result = await invokeWrapper('workflow_automation_run_views', { automationId })
    const views = (result || []).map(normalizeRunView)
    projectedRunsByAutomation.value = {
      ...projectedRunsByAutomation.value,
      [automationId]: views
    }
    return views
  }

  // draft/apply reach the same typed facade the scheduler and CLI use. `draft`
  // is side-effect-free; `apply` carries the reviewed plan plus the exact hash
  // and an explicit permission acknowledgement (AC-3/AC-4/INV-4/INV-5).
  const draftPlan = async input => invokeWrapper('workflow_automation_draft', { input })

  const applyPlan = async request => invokeWrapper('workflow_automation_apply', { request })

  return {
    automations,
    runsByAutomation,
    projectedRunsByAutomation,
    loading,
    error,
    selectedAutomationId,
    selectedAutomation,
    selectAutomation,
    fetchAutomations,
    saveAutomation,
    deleteAutomation,
    updateAutomationAllowedPaths,
    setAutomationEnabled,
    runAutomationNow,
    fetchRuns,
    fetchProjectedRuns,
    draftPlan,
    applyPlan
  }
})
