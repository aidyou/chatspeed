import { ref, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useModelStore } from '@/stores/model'
import { ElMessageBox } from 'element-plus'

/**
 * Composable for core workflow operations
 * Handles CRUD, start/stop/continue, and event handling
 */
export function useWorkflowCore({
  selectedAgent,
  planningMode,
  approvalLevel,
  finalAuditMode,
  pendingPaths,
  currentWorkflowId,
  currentWorkflow,
  chattingParser,
  chatState,
  approvalVisible,
  approvalRequestId,
  approvalAction,
  approvalDetails,
  enhancedMessages,
  isCompressing,
  compressionMessage,
  fetchSystemSkills,
  resetChatState,
  clearRetryTimer,
  setRetryStatus,
  processChunk,
  processReasoningChunk,
  setCompressionStatus,
  scrollToBottom
}) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()
  const agentStore = useAgentStore()
  const modelStore = useModelStore()

  const unlistenWorkflowEvents = ref(null)
  const modelSelectorVisible = ref(false)
  const modelSelectorTab = ref('act')
  const modelSelectorMode = ref('provider')

  // Edit workflow dialog
  const editWorkflowDialogVisible = ref(false)
  const editWorkflowId = ref(null)
  const editWorkflowTitle = ref('')

  const workflows = computed(() => workflowStore.workflows)
  const isRunning = computed(() => workflowStore.isRunning)
  const isAwaitingApproval = computed(() => currentWorkflow.value?.status === 'awaiting_approval')

  const activeModelName = computed(() => {
    // 1. Try to get from current configs (reflected in settings/workflow)
    const tab = planningMode.value ? 'plan' : 'act'
    const workflow =
      workflowStore.currentWorkflow ||
      (workflowStore.workflows.length > 0 ? workflowStore.workflows[0] : null)

    let providerId = null
    let modelName = null

    if (workflow && workflow.agentConfig && workflow.agentConfig.models) {
      const models = workflow.agentConfig.models
      const model = planningMode.value ? models.plan || models.act : models.act
      if (model) {
        providerId = model.id
        modelName = model.model
      }
    }

    // Handle proxy models (providerId === 0)
    if (providerId === 0 && modelName) {
      // Proxy model format: "group@alias" or just "alias" (default group)
      if (modelName.includes('@')) {
        const [group, alias] = modelName.split('@')
        return `${alias} (${group})`
      }
      return modelName
    }

    // Handle regular models
    if (providerId && modelName) {
      const provider = modelStore.getModelProviderById(providerId)
      if (provider) {
        const model = provider.models.find((m) => m.id === modelName)
        if (model) return model.name
      }
      return modelName
    }

    if (selectedAgent.value) return selectedAgent.value.name
    return 'Select Model'
  })

  const canSwitchWorkflow = computed(() => {
    // [UI Enhancement] Allow switching workflow view even if one is running in background.
    // The event listener will switch to the new session, and background sessions will
    // continue to run on the server.
    return true
  })

  // Unified function to update workflow config
  const updateWorkflowConfig = async (key, value) => {
    if (!currentWorkflowId.value) return

    try {
      // 1. Update database
      const snapshot = await invokeWrapper('get_workflow_snapshot', {
        sessionId: currentWorkflowId.value
      })

      let agentConfig = {}
      if (snapshot.workflow?.agentConfig) {
        agentConfig = typeof snapshot.workflow.agentConfig === 'string'
          ? JSON.parse(snapshot.workflow.agentConfig)
          : snapshot.workflow.agentConfig
      }

      agentConfig[key] = value

      await invokeWrapper('update_workflow_agent_config', {
        sessionId: currentWorkflowId.value,
        agentConfig: JSON.stringify(agentConfig)
      })

      // 2. Signal engine if workflow is active
      const status = currentWorkflow.value?.status
      if (status && ['thinking', 'executing', 'paused', 'awaiting_approval'].includes(status)) {
        try {
          await invokeWrapper('workflow_signal', {
            sessionId: currentWorkflowId.value,
            signal: JSON.stringify({
              type: `update_${key}`,
              [key]: value
            })
          })
        } catch (error) {
          console.warn(`Failed to signal engine for ${key}:`, error)
        }
      }

      // 3. Refresh UI
      await workflowStore.selectWorkflow(currentWorkflowId.value)
    } catch (error) {
      console.error(`Failed to update ${key}:`, error)
    }
  }

  // Watch for state changes to handle UI side effects
  watch(
    () => currentWorkflow.value?.status,
    (newStatus) => {
      // If state is no longer Paused, we should hide any open approval dialog
      if (newStatus !== 'paused' && approvalVisible.value) {
        approvalVisible.value = false
      }
    }
  )

  const setupWorkflowEvents = async (sessionId) => {
    if (unlistenWorkflowEvents.value) {
      unlistenWorkflowEvents.value()
      unlistenWorkflowEvents.value = null
    }

    const eventName = `workflow://event/${sessionId}`
    unlistenWorkflowEvents.value = await listen(eventName, (event) => {
      const payload = event.payload

      if (payload.type === 'state') {
        workflowStore.updateWorkflowStatus(sessionId, payload.state)

        // If we move out of Thinking/Executing, reset the parser
        // Use a small timeout to allow final rendering of streaming buffers
        if (payload.state !== 'thinking' && payload.state !== 'executing') {
          setTimeout(() => {
            resetChatState()
          }, 500)
        }
      } else if (payload.type === 'chunk') {
        // Direct text chunk from LLM or StreamParser
        processChunk(payload.content)
        scrollToBottom()
      } else if (payload.type === 'reasoning_chunk') {
        // Thinking chunk
        processReasoningChunk(payload.content)
        scrollToBottom()
      } else if (payload.type === 'message') {
        // ReAct engine sends incremental messages or chunks
        workflowStore.addMessage({
          sessionId: sessionId,
          role: payload.role,
          message: payload.content,
          reasoning: payload.reasoning,
          stepType: payload.step_type,
          stepIndex: payload.step_index,
          isError: payload.is_error,
          errorType: payload.error_type,
          metadata: payload.metadata
        })

        // Clear tool stream when tool message is finalized
        if (payload.role === 'tool' && payload.metadata?.tool_call_id) {
          workflowStore.clearToolStream(payload.metadata.tool_call_id)
        }

        // Message finalized, clear chatting buffer (including reasoning)
        resetChatState()

        // Force scroll for new full messages
        scrollToBottom(true)
      } else if (payload.type === 'confirm') {
        approvalRequestId.value = payload.id
        approvalAction.value = payload.action
        approvalDetails.value = payload.details
        approvalVisible.value = true
      } else if (payload.type === 'retry_status') {
        // Handle 429 retry status
        setRetryStatus(payload)
      } else if (payload.type === 'sync_todo') {
        workflowStore.setTodoList(payload.todo_list)
      } else if (payload.type === 'compression_status') {
        // Handle context compression status
        setCompressionStatus(payload.is_compressing, payload.message)
        if (payload.is_compressing) {
          scrollToBottom(true)
        }
      } else if (payload.type === 'notification') {
        workflowStore.setNotification(payload.message, payload.category)
      } else if (payload.type === 'auto_approved_tools_updated') {
        workflowStore.setAutoApprovedTools(payload.tools)
      } else if (payload.type === 'shell_policy_updated') {
        workflowStore.setShellPolicy(payload.policy)
      } else if (payload.type === 'tool_stream') {
        // Handle tool streaming output
        const { tool_id, output } = payload
        workflowStore.appendToolStream(tool_id, output)
      }
    })
  }

  const selectWorkflow = async (id) => {
    if (!canSwitchWorkflow.value) {
      console.warn('Cannot switch workflow while another is running')
      return
    }

    // Select the workflow in store
    await workflowStore.selectWorkflow(id)

    if (workflowStore.currentWorkflow) {
      const agent = agentStore.agents.find((a) => a.id === workflowStore.currentWorkflow.agentId)
      if (agent) {
        selectedAgent.value = agent
        // Setup event listeners for the existing session
        await setupWorkflowEvents(id)
      }

      // Initialize settings from workflow's agentConfig or fallback to agent defaults
      const config = workflowStore.currentWorkflow.agentConfig || {}

      // finalAuditMode
      if (config.finalAudit !== undefined && config.finalAudit !== null) {
        finalAuditMode.value = config.finalAudit ? 'on' : 'off'
      } else if (selectedAgent.value?.finalAudit) {
        finalAuditMode.value = 'on'
      } else {
        finalAuditMode.value = 'off'
      }

      // approvalLevel
      if (config.approvalLevel) {
        approvalLevel.value = config.approvalLevel
      } else if (selectedAgent.value?.approvalLevel) {
        approvalLevel.value = selectedAgent.value.approvalLevel
      } else {
        approvalLevel.value = 'default'
      }
    }
  }

  const startNewWorkflow = async (prompt) => {
    if (!selectedAgent.value) {
      console.error('No agent selected')
      return
    }

    if (!prompt || !prompt.trim()) return

    try {
      console.log('Starting workflow...')

      // Check if we already have an empty workflow (created by createNewWorkflow)
      if (currentWorkflowId.value && workflowStore.currentWorkflow) {
        const currentQuery = workflowStore.currentWorkflow.userQuery
        if (!currentQuery || currentQuery.trim() === '') {
          // We have an empty workflow, update it with the query and start
          console.log('Using existing empty workflow:', currentWorkflowId.value)

          // Update workflow title and user_query in backend
          await invokeWrapper('update_workflow_title_and_query', {
            sessionId: currentWorkflowId.value,
            title: prompt.substring(0, 50),
            userQuery: prompt
          })

          // Trigger engine
          console.log('Calling workflow_start backend command...')
          await invokeWrapper('workflow_start', {
            sessionId: currentWorkflowId.value,
            agentId: selectedAgent.value.id,
            initialPrompt: prompt,
            planningMode: planningMode.value
          })
          console.log('Workflow engine started successfully')
          return
        }
      }

      // No empty workflow, create a new one with query
      console.log('Creating new workflow with query')

      // Get inherited config if from another workflow
      let inheritedAgentConfig = null
      let inheritedAgentId = null
      if (currentWorkflowId.value) {
        try {
          const snapshot = await invokeWrapper('get_workflow_snapshot', {
            sessionId: currentWorkflowId.value
          })
          if (snapshot.workflow?.agentConfig) {
            inheritedAgentConfig = JSON.stringify(snapshot.workflow.agentConfig)
            inheritedAgentId = snapshot.workflow.agentId
          }
        } catch (error) {
          console.warn('Failed to get previous workflow config:', error)
        }
      }

      // Get allowed paths
      let workflowAllowedPaths = []
      if (pendingPaths.value.length > 0) {
        workflowAllowedPaths = [...pendingPaths.value]
      } else if (selectedAgent.value.allowedPaths) {
        try {
          workflowAllowedPaths =
            typeof selectedAgent.value.allowedPaths === 'string'
              ? JSON.parse(selectedAgent.value.allowedPaths)
              : selectedAgent.value.allowedPaths
        } catch (e) {
          console.error('Failed to parse agent allowedPaths:', e)
        }
      }

      // Create workflow with query
      const res = await invokeWrapper('create_workflow', {
        request: {
          userQuery: prompt,
          agentId: inheritedAgentId || selectedAgent.value.id,
          allowedPaths: workflowAllowedPaths,
          finalAudit: finalAuditMode.value === 'on',
          inheritedAgentConfig
        }
      })

      const newWorkflowId = typeof res === 'string' ? res : res.id || res
      console.log('Workflow session created:', newWorkflowId)

      // Clear pending paths after workflow is created
      pendingPaths.value = []

      // Update selectedAgent if we inherited a different agent
      if (inheritedAgentId && inheritedAgentId !== selectedAgent.value?.id) {
        const inheritedAgent = agentStore.agents.find(a => a.id === inheritedAgentId)
        if (inheritedAgent) {
          selectedAgent.value = inheritedAgent
        }
      }

      // Sync UI state
      await workflowStore.loadWorkflows()
      await workflowStore.selectWorkflow(newWorkflowId)
      await setupWorkflowEvents(newWorkflowId)

      // Trigger engine
      console.log('Calling workflow_start backend command...')
      await invokeWrapper('workflow_start', {
        sessionId: newWorkflowId,
        agentId: inheritedAgentId || selectedAgent.value.id,
        initialPrompt: prompt,
        planningMode: planningMode.value
      })
      console.log('Workflow engine started successfully')
    } catch (error) {
      console.error('Failed to start workflow:', error)
      showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
    }
  }

  const onSendMessage = async (message) => {
    // Handle Builtin UI Commands (Exact match after trim)
    if (message.trim().startsWith('/')) {
      if (await handleBuiltinCommand(message)) {
        return true // Indicate that command was handled
      }
    }

    console.log('Sending message to workflow:', message)

    // CRITICAL: Reset the stream parser and UI buffer BEFORE sending the new request.
    // This ensures no residual data from the previous turn pollutes the next response.
    resetChatState()

    if (!currentWorkflowId.value) {
      // Start brand new workflow
      await startNewWorkflow(message)
    } else {
      // 2. Decide: Signal or Re-start?
      const isPaused = currentWorkflow.value?.status === 'paused'
      if (isRunning.value || isPaused) {
        // Just send signal to the running loop
        try {
          const signal = JSON.stringify({
            type: 'user_input',
            content: message
          })

          // Optimistic update to clear the "AI is waiting" hint immediately
          if (isPaused) {
            workflowStore.updateWorkflowStatus(currentWorkflowId.value, 'thinking')
          }

          const res = await invokeWrapper('workflow_signal', {
            sessionId: currentWorkflowId.value,
            signal: signal
          })
          console.log('Signal sent successfully:', res)
        } catch (error) {
          console.error('Failed to send signal:', error)
        }
      } else {
        // Engine is stopped (Completed, Error, or Awaiting Approval).
        // DO NOT add message manually here, workflow_start will handle it and broadcast via events.
        try {
          // If we were awaiting approval, continue in planning mode if we send a message (rejecting the plan)
          const isCurrentlyAwaiting = currentWorkflow.value?.status === 'awaiting_approval'

          // Ensure event listener is setup for this session
          await setupWorkflowEvents(currentWorkflowId.value)

          await invokeWrapper('workflow_start', {
            sessionId: currentWorkflowId.value,
            agentId: selectedAgent.value.id,
            initialPrompt: message,
            planningMode: isCurrentlyAwaiting || planningMode.value
          })
        } catch (error) {
          console.error('Failed to resume workflow:', error)
          showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
        }
      }
    }
  }

  const handleBuiltinCommand = async (command) => {
    const cmd = command.trim().toLowerCase()

    if (cmd === '/settings') {
      await invokeWrapper('open_setting_window', { settingType: 'general' })
      return true
    }
    if (cmd === '/mcp') {
      await invokeWrapper('open_setting_window', { settingType: 'mcp' })
      return true
    }
    if (cmd === '/proxy') {
      await invokeWrapper('open_setting_window', { settingType: 'proxy' })
      return true
    }
    if (cmd === '/agent') {
      await invokeWrapper('open_setting_window', { settingType: 'agent' })
      return true
    }
    if (cmd === '/about') {
      await invokeWrapper('open_setting_window', { settingType: 'about' })
      return true
    }
    if (cmd === '/models') {
      openModelSelector()
      return true
    }
    return false
  }

  const onContinue = async () => {
    if (!currentWorkflowId.value || isRunning.value) return

    try {
      // If it's paused, we might need to send a signal,
      // but usually 'workflow_start' with no prompt works to resume the loop if it's not active.
      await invokeWrapper('workflow_start', {
        sessionId: currentWorkflowId.value,
        agentId: selectedAgent.value.id
      })
    } catch (error) {
      console.error('Failed to continue workflow:', error)
      showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
    }
  }

  const onApprovePlan = async () => {
    if (!currentWorkflowId.value) return

    // Find the last assistant message that contains 'submit_plan' tool call
    const assistantMsgs = workflowStore.messages.filter((m) => m.role === 'assistant')
    const lastAssistantMsg = assistantMsgs[assistantMsgs.length - 1]

    if (!lastAssistantMsg) return

    // Extract plan from tool call arguments if available, otherwise use message content
    let planContent = lastAssistantMsg.message
    try {
      const metadata =
        typeof lastAssistantMsg.metadata === 'string'
          ? JSON.parse(lastAssistantMsg.metadata)
          : lastAssistantMsg.metadata

      if (metadata && (metadata.tool_calls || metadata.tool)) {
        const toolCalls = metadata.tool_calls || (metadata.tool ? [metadata.tool] : [])
        const submitPlanCall = toolCalls.find(
          (c) => c.name === 'submit_plan' || (c.function && c.function.name === 'submit_plan')
        )
        if (submitPlanCall) {
          const args =
            typeof submitPlanCall.arguments === 'string'
              ? JSON.parse(submitPlanCall.arguments)
              : submitPlanCall.arguments ||
                submitPlanCall.function?.arguments ||
                submitPlanCall.input
          if (args && args.plan) {
            planContent = args.plan
          }
        }
      }
    } catch (e) {
      console.warn(
        'Failed to extract plan from metadata, using raw message content instead:',
        e
      )
    }

    try {
      await invokeWrapper('workflow_approve_plan', {
        sessionId: currentWorkflowId.value,
        agentId: selectedAgent.value.id,
        plan: planContent
      })
      console.log('Plan approved and execution started')
    } catch (error) {
      console.error('Failed to approve plan:', error)
      showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
    }
  }

  const onStop = async () => {
    if (currentWorkflowId.value) {
      // Optimistic update: Immediately set running to false to toggle the UI button.
      // The backend might take a moment to gracefully cancel, but the user expects immediate feedback.
      workflowStore.setRunning(false)
      
      // Clear any pending retry status or AI notifications
      clearRetryTimer()
      resetChatState()
      workflowStore.setNotification('', 'info')

      try {
        await invokeWrapper('workflow_stop', {
          sessionId: currentWorkflowId.value
        })
      } catch (error) {
        console.error('Failed to stop workflow:', error)
      }
    }
  }

  const openModelSelector = () => {
    modelSelectorTab.value = planningMode.value ? 'plan' : 'act'
    modelSelectorVisible.value = true
  }

  const onModelConfigSave = async (configs) => {
    console.log('Saving model config:', configs)
    try {
      // 1. If we have an active workflow session, update workflow's agent_config
      if (currentWorkflowId.value) {
        // Get current agent_config
        const snapshot = await invokeWrapper('get_workflow_snapshot', {
          sessionId: currentWorkflowId.value
        })

        let agentConfig = {}
        if (snapshot.workflow?.agentConfig) {
          agentConfig = typeof snapshot.workflow.agentConfig === 'string'
            ? JSON.parse(snapshot.workflow.agentConfig)
            : snapshot.workflow.agentConfig
        }

        // Update models in agent_config
        agentConfig.models = {
          plan: configs.plan,
          act: configs.act
        }

        // Save back to workflow (database update)
        await invokeWrapper('update_workflow_agent_config', {
          sessionId: currentWorkflowId.value,
          agentConfig: JSON.stringify(agentConfig)
        })

        // Signal the engine to update runtime config (only if workflow is active)
        const status = currentWorkflow.value?.status
        if (status && ['thinking', 'executing', 'paused', 'awaiting_approval'].includes(status)) {
          try {
            await invokeWrapper('workflow_signal', {
              sessionId: currentWorkflowId.value,
              signal: JSON.stringify({
                type: 'update_model_config',
                configs: configs
              })
            })
          } catch (error) {
            console.warn('Failed to signal engine (workflow may not be running):', error)
          }
        }

        // Refresh current workflow state from DB to update UI
        await workflowStore.selectWorkflow(currentWorkflowId.value)
      } else if (selectedAgent.value) {
        // 2. No active workflow - update agent's default config
        const updatedAgent = {
          ...selectedAgent.value,
          planModel: configs.plan,
          actModel: configs.act
        }

        await agentStore.saveAgent(updatedAgent)
        await agentStore.fetchAgents()
        selectedAgent.value =
          agentStore.agents.find((a) => a.id === updatedAgent.id) || updatedAgent
      }

      showMessage(t('common.saveSuccess'), 'success')
    } catch (error) {
      console.error('Failed to save model config:', error)
      showMessage(t('common.saveFailed'), 'error')
    }
  }

  const onEditWorkflow = (id) => {
    editWorkflowId.value = id
    editWorkflowTitle.value = workflows.value.find((wf) => wf.id === id)?.title || ''
    editWorkflowDialogVisible.value = true
  }

  const onSaveEditWorkflow = async () => {
    if (!editWorkflowId.value) return

    try {
      await invokeWrapper('update_workflow_title', {
        sessionId: editWorkflowId.value,
        title: editWorkflowTitle.value
      })

      // Reload workflows to get updated data
      await workflowStore.loadWorkflows()

      editWorkflowDialogVisible.value = false
      editWorkflowTitle.value = ''
      editWorkflowId.value = null
    } catch (error) {
      console.error('Failed to update workflow:', error)
    }
  }

  const onDeleteWorkflow = (id) => {
    ElMessageBox.confirm(
      t('workflow.confirmDeleteWorkflow'),
      t('common.warning'),
      {
        confirmButtonText: t('common.confirm'),
        cancelButtonText: t('common.cancel'),
        teleported: true
      }
    ).then(async () => {
      try {
        await invokeWrapper('delete_workflow', { sessionId: id })

        // If deleting the current workflow, clear it
        if (id === currentWorkflowId.value) {
          workflowStore.clearCurrentWorkflow()
        }

        // Reload workflows
        await workflowStore.loadWorkflows()

        // Load the last workflow if available
        if (workflows.value.length > 0) {
          await selectWorkflow(workflows.value[0].id)
        }
      } catch (error) {
        console.error('Failed to delete workflow:', error)
      }
    })
  }

  const createNewWorkflow = async () => {
    try {
      // 1. Get current config to inherit
      let inheritedAgentConfig = null
      let inheritedAgentId = selectedAgent.value?.id
      let inheritedAllowedPaths = []
      let inheritedFinalAudit = null
      let inheritedApprovalLevel = null

      if (workflowStore.currentWorkflow?.agentConfig) {
        inheritedAgentConfig = JSON.stringify(workflowStore.currentWorkflow.agentConfig)
        inheritedAgentId = workflowStore.currentWorkflow.agentId
        // Inherit allowed paths from current workflow's agentConfig
        const config = workflowStore.currentWorkflow.agentConfig
        if (config.allowedPaths && Array.isArray(config.allowedPaths)) {
          inheritedAllowedPaths = config.allowedPaths
        }
        // Inherit finalAudit from current workflow's agentConfig
        if (config.finalAudit !== undefined && config.finalAudit !== null) {
          inheritedFinalAudit = config.finalAudit
        }
        // Inherit approvalLevel from current workflow's agentConfig
        if (config.approvalLevel) {
          inheritedApprovalLevel = config.approvalLevel
        }
      }

      // 2. Get allowed paths - prioritize inherited paths
      let workflowAllowedPaths = []
      if (inheritedAllowedPaths.length > 0) {
        workflowAllowedPaths = [...inheritedAllowedPaths]
      } else if (pendingPaths.value.length > 0) {
        workflowAllowedPaths = [...pendingPaths.value]
      } else if (selectedAgent.value?.allowedPaths) {
        try {
          workflowAllowedPaths =
            typeof selectedAgent.value.allowedPaths === 'string'
              ? JSON.parse(selectedAgent.value.allowedPaths)
              : selectedAgent.value.allowedPaths
        } catch (e) {
          console.error('Failed to parse agent allowedPaths:', e)
        }
      }

      // 3. Use inherited finalAudit if available, otherwise use current local state
      const workflowFinalAudit = inheritedFinalAudit !== null ? inheritedFinalAudit : finalAuditMode.value === 'on'

      // 4. Update local state to match inherited values
      if (inheritedApprovalLevel) {
        approvalLevel.value = inheritedApprovalLevel
      }
      if (inheritedFinalAudit !== null) {
        finalAuditMode.value = inheritedFinalAudit ? 'on' : 'off'
      }

      // 5. Create empty workflow in backend
      const newWorkflowId = await invokeWrapper('create_workflow', {
        request: {
          agentId: inheritedAgentId,
          allowedPaths: workflowAllowedPaths,
          finalAudit: workflowFinalAudit,
          inheritedAgentConfig
        }
      })

      // 6. Update selectedAgent if we inherited a different agent
      if (inheritedAgentId && inheritedAgentId !== selectedAgent.value?.id) {
        const inheritedAgent = agentStore.agents.find(a => a.id === inheritedAgentId)
        if (inheritedAgent) {
          selectedAgent.value = inheritedAgent
        }
      }

      // 7. Load and select the new workflow
      await workflowStore.loadWorkflows()
      await workflowStore.selectWorkflow(newWorkflowId)

      console.log('Created empty workflow:', newWorkflowId)
    } catch (error) {
      console.error('Failed to create new workflow:', error)
      showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
    }
  }

  const toggleFinalAuditMode = () => {
    const newValue = finalAuditMode.value === 'on' ? 'off' : 'on'
    finalAuditMode.value = newValue
  }

  // Watch for approval level changes
  watch(approvalLevel, async (newVal) => {
    await updateWorkflowConfig('approvalLevel', newVal)
  })

  // Watch for final audit mode changes
  watch(finalAuditMode, async (newVal) => {
    await updateWorkflowConfig('finalAudit', newVal === 'on')
  })

  return {
    unlistenWorkflowEvents,
    modelSelectorVisible,
    modelSelectorTab,
    modelSelectorMode,
    editWorkflowDialogVisible,
    editWorkflowId,
    editWorkflowTitle,
    workflows,
    isRunning,
    isAwaitingApproval,
    activeModelName,
    canSwitchWorkflow,
    setupWorkflowEvents,
    selectWorkflow,
    startNewWorkflow,
    onSendMessage,
    handleBuiltinCommand,
    onContinue,
    onApprovePlan,
    onStop,
    openModelSelector,
    onModelConfigSave,
    onEditWorkflow,
    onSaveEditWorkflow,
    onDeleteWorkflow,
    createNewWorkflow,
    toggleFinalAuditMode
  }
}
