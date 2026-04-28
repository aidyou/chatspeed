import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'
import { ElMessageBox } from 'element-plus'
import notificationSoundUrl from '/sound/notification.mp3'
import successSoundUrl from '/sound/success.mp3'
import {
    RUNNING_STATUSES,
    SIGNAL_TYPES,
    TERMINAL_STATUSES,
    WAITING_STATUSES,
    WORKFLOW_STATUSES,
    WORKFLOW_WAIT_REASONS
} from '@/composables/workflow/signalTypes'
import { safeExecute } from './useErrorBoundary'
import { BLOCKING_WAIT_REASONS } from './signalTypes'

/**
 * Composable for core workflow operations
 * Handles CRUD, start/stop/continue, and event handling
 *
 * Phase 9: Add error boundaries to ensure UI exceptions don't block workflow execution
 */
export function useWorkflowCore({
    selectedAgent,
    planningMode,
    approvalLevel,
    finalAuditMode,
    pendingPaths,
    currentWorkflowId,
    currentWorkflow,
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
    const settingStore = useSettingStore()

    const unlistenWorkflowEvents = ref(null)
    const backgroundStateListeners = new Map<string, () => void>()
    const pendingApprovalEntries = ref({})
    const modelSelectorVisible = ref(false)
    const modelSelectorTab = ref('act')
    const modelSelectorMode = ref('provider')

    // Edit workflow dialog
    const editWorkflowDialogVisible = ref(false)
    const editWorkflowId = ref(null)
    const editWorkflowTitle = ref('')

    const workflows = computed(() => workflowStore.workflows)
    const isRunning = computed(() => workflowStore.isRunning)
    const waitReason = computed(() => workflowStore.waitReason)
    const hasLiveSession = computed(() => workflowStore.hasLiveSession)
    const isLiveWaiting = computed(() => workflowStore.isLiveWaiting)
    const canStop = computed(() => workflowStore.canStop)
    const canContinue = computed(() => workflowStore.canContinue)
    const canApprovePlan = computed(() => workflowStore.canApprovePlan)
    const isWaiting = computed(() => workflowStore.isWaiting)

    const isAwaitingApproval = computed(() => {
        return canApprovePlan.value
    })
    const pendingApprovalList = computed(() =>
        Object.values(pendingApprovalEntries.value).sort((a, b) => b.updatedAt - a.updatedAt)
    )
    const approvalNotificationAudio = ref<HTMLAudioElement>()
    const completionAudio = ref<HTMLAudioElement>()

    const playApprovalNotificationSound = async () => {
        if (settingStore.settings.workflowApprovalMuted) return

        try {
            if (!approvalNotificationAudio.value) {
                approvalNotificationAudio.value = new Audio(notificationSoundUrl)
                approvalNotificationAudio.value.preload = 'auto'
            }

            approvalNotificationAudio.value.pause()
            approvalNotificationAudio.value.currentTime = 0
            await approvalNotificationAudio.value.play()
        } catch (error) {
            console.warn('[Workflow] Failed to play approval notification sound:', error)
        }
    }

    const playCompletionSound = async () => {
        if (settingStore.settings.workflowCompletionMuted) return

        try {
            if (!completionAudio.value) {
                completionAudio.value = new Audio(successSoundUrl)
                completionAudio.value.preload = 'auto'
            }

            completionAudio.value.pause()
            completionAudio.value.currentTime = 0
            await completionAudio.value.play()
        } catch (error) {
            console.warn('[Workflow] Failed to play completion sound:', error)
        }
    }

    const activeModelName = computed(() => {
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

        if (!modelName && selectedAgent.value) {
            const fallbackModel =
                tab === 'plan'
                    ? selectedAgent.value.planModel || selectedAgent.value.actModel
                    : selectedAgent.value.actModel || selectedAgent.value.planModel
            if (fallbackModel) {
                providerId = fallbackModel.id
                modelName = fallbackModel.model
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
    const signalMapping: Record<string, string> = {
        finalAudit: SIGNAL_TYPES.UPDATE_FINAL_AUDIT,
        approvalLevel: SIGNAL_TYPES.UPDATE_APPROVAL_LEVEL
    }

    const toSignalType = (key) => {
        if (signalMapping[key]) return signalMapping[key]
        const snake = key.replace(/[A-Z]/g, (ch) => `_${ch.toLowerCase()}`)
        return `update_${snake}`
    }
    const isSyncingWorkflowConfig = ref(false)
    const currentPhaseValue = () => (planningMode.value ? 'planning' : 'standard')

    const applyWorkflowConfigToLocalStore = (nextConfig) => {
        const workflowIndex = workflowStore.workflows.findIndex((w) => w.id === currentWorkflowId.value)
        if (workflowIndex === -1) return

        const existingConfig = workflowStore.workflows[workflowIndex].agentConfig || {}
        workflowStore.workflows[workflowIndex].agentConfig = {
            ...existingConfig,
            ...nextConfig
        }

        if (
            workflowStore.currentWorkflow &&
            workflowStore.currentWorkflow.id === currentWorkflowId.value
        ) {
            workflowStore.currentWorkflow.agentConfig = {
                ...(workflowStore.currentWorkflow.agentConfig || {}),
                ...nextConfig
            }
        }
    }

    const mergeLocalUiOverrides = (baseConfig = {}) => ({
        ...baseConfig,
        approvalLevel: approvalLevel.value,
        finalAudit: finalAuditMode.value === 'on',
        phase: currentPhaseValue()
    })

    const persistCurrentWorkflowUiConfigBeforeStart = async () => {
        if (!currentWorkflowId.value) return

        const snapshot = await invokeWrapper('get_workflow_snapshot', {
            sessionId: currentWorkflowId.value
        })

        let agentConfig = {}
        if (snapshot.workflow?.agentConfig) {
            agentConfig = typeof snapshot.workflow.agentConfig === 'string'
                ? JSON.parse(snapshot.workflow.agentConfig)
                : snapshot.workflow.agentConfig
        }

        const nextConfig = mergeLocalUiOverrides(agentConfig)

        await invokeWrapper('update_workflow_agent_config', {
            sessionId: currentWorkflowId.value,
            agentConfig: JSON.stringify(nextConfig)
        })

        applyWorkflowConfigToLocalStore(nextConfig)
    }

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

            // 2. Signal engine if workflow is active, including structured waiting states.
            const status = currentWorkflow.value?.status
            if (
                signalMapping[key] &&
                status &&
                [
                    WORKFLOW_STATUSES.THINKING,
                    WORKFLOW_STATUSES.EXECUTING,
                    WORKFLOW_STATUSES.PAUSED,
                    WORKFLOW_STATUSES.AWAITING_USER,
                    WORKFLOW_STATUSES.AWAITING_APPROVAL
                ].includes(status)
            ) {
                try {
                    const signalType = toSignalType(key)
                    await invokeWrapper('workflow_signal', {
                        sessionId: currentWorkflowId.value,
                        signal: JSON.stringify({
                            type: signalType,
                            [key]: value
                        })
                    })
                } catch (error) {
                    console.warn(`Failed to signal engine for ${key}:`, error)
                }
            }

            // 3. Update local workflow store state (don't call selectWorkflow to avoid recursion)
            applyWorkflowConfigToLocalStore({ [key]: value })
        } catch (error) {
            console.error(`Failed to update ${key}:`, error)
        }
    }

    const sendUserMessageSignal = async (sessionId, content, queuedUserMessageId = null) => {
        const signalPayload = {
            type: SIGNAL_TYPES.USER_MESSAGE,
            content
        }
        if (queuedUserMessageId) {
            signalPayload.queued_user_message_id = queuedUserMessageId
        }
        const signal = JSON.stringify(signalPayload)
        return invokeWrapper('workflow_signal', { sessionId, signal })
    }

    const removeQueuedUserMessageSignal = async (sessionId, queuedUserMessageId) => {
        const signal = JSON.stringify({
            type: SIGNAL_TYPES.REMOVE_QUEUED_USER_MESSAGE,
            queued_user_message_id: queuedUserMessageId
        })
        return invokeWrapper('workflow_signal', { sessionId, signal })
    }

    const flushDeferredQueuedMessages = async () => {
        if (!currentWorkflowId.value) return
        if (BLOCKING_WAIT_REASONS.includes(waitReason.value)) return
        if (!workflowStore.messageQueue?.length) return

        const deferred = workflowStore.messageQueue.filter((item) => !item.sent)
        for (const item of deferred) {
            try {
                await sendUserMessageSignal(currentWorkflowId.value, item.content, item.id)
                workflowStore.markQueuedMessageSent(item.id)
            } catch (error) {
                console.warn('Failed to flush deferred queued message:', error)
                break
            }
        }
    }

    // Track the current session ID for event isolation
    const currentSessionId = ref<string | null>(null)

    const teardownBackgroundStateListeners = () => {
        for (const unlisten of backgroundStateListeners.values()) {
            try {
                unlisten()
            } catch (error) {
                console.warn('[Workflow] Failed to unlisten background workflow state listener:', error)
            }
        }
        backgroundStateListeners.clear()

        pendingApprovalEntries.value = {}
    }

    const upsertPendingApprovalEntry = (sessionId, payload = {}) => {
        if (!sessionId) return

        const workflow = workflows.value.find((item) => item.id === sessionId)
        const workflowTitle = workflow?.title || workflow?.userQuery || t('workflow.untitled')
        const approvalId = payload.id || 'awaiting_approval'
        const key = `${sessionId}:${approvalId}`

        pendingApprovalEntries.value = {
            ...pendingApprovalEntries.value,
            [key]: {
                key,
                id: approvalId,
                sessionId,
                kind: payload.kind || 'approval',
                workflowTitle,
                action: payload.action || t('workflow.awaitingApproval'),
                updatedAt: Date.now()
            }
        }
    }

    const clearPendingApprovalEntries = (sessionId: string, kind = null) => {
        if (!sessionId) {
            pendingApprovalEntries.value = {}
            return
        }

        const nextEntries = { ...pendingApprovalEntries.value }
        let changed = false

        for (const key of Object.keys(nextEntries)) {
            if (!key.startsWith(`${sessionId}:`)) {
                continue
            }
            if (kind && nextEntries[key]?.kind !== kind) {
                continue
            }
            if (key.startsWith(`${sessionId}:`)) {
                delete nextEntries[key]
                changed = true
            }
        }

        if (changed) {
            pendingApprovalEntries.value = nextEntries
        }
    }

    const getPendingApprovalEntry = (sessionId: string, approvalId: string) => {
        if (!sessionId || !approvalId) return null
        return pendingApprovalEntries.value[`${sessionId}:${approvalId}`] || null
    }

    const getExecutionContextPendingTools = (workflow) => {
        const pendingTools =
            workflow?.executionContext?.pendingTools ||
            workflow?.executionContext?.pending_tools ||
            []
        return Array.isArray(pendingTools) ? pendingTools : []
    }

    const clearPendingApprovalEntry = (sessionId, approvalId) => {
        if (!sessionId || !approvalId) return
        const key = `${sessionId}:${approvalId}`
        if (!pendingApprovalEntries.value[key]) return
        const nextEntries = { ...pendingApprovalEntries.value }
        delete nextEntries[key]
        pendingApprovalEntries.value = nextEntries
    }

    const showBackgroundApprovalNotification = (sessionId, payload) => {
        if (!payload?.id) return

        upsertPendingApprovalEntry(sessionId, payload)
        playApprovalNotificationSound()
    }

    const showBackgroundAskUserNotification = (sessionId) => {
        upsertPendingApprovalEntry(sessionId, {
            id: 'awaiting_user',
            kind: 'ask_user',
            action: t('workflow.awaitingUser')
        })
    }

    const syncBackgroundStateListeners = async () => {
        const activeSessionId = currentWorkflowId.value || currentSessionId.value
        const backgroundWorkflowIds = new Set(
            workflows.value
                .filter((workflow) => {
                    if (!workflow?.id) return false
                    if (workflow.id === activeSessionId) return false
                    const status = String(workflow.status || '').toLowerCase()
                    return status && !TERMINAL_STATUSES.includes(status)
                })
                .map((workflow) => workflow.id)
        )

        for (const [sessionId, unlisten] of backgroundStateListeners.entries()) {
            if (!backgroundWorkflowIds.has(sessionId)) {
                try {
                    unlisten()
                } catch (error) {
                    console.warn(`[Workflow] Failed to unlisten background session ${sessionId}:`, error)
                }
                backgroundStateListeners.delete(sessionId)
            }
        }

        for (const sessionId of backgroundWorkflowIds) {
            if (backgroundStateListeners.has(sessionId)) continue

            const eventName = `workflow://event/${sessionId}`
            const unlisten = await listen(eventName, (event) => {
                safeExecute(() => {
                    const payload = event.payload
                    if (!payload?.type) return

                    if (payload.type === 'confirm') {
                        showBackgroundApprovalNotification(sessionId, payload)
                        return
                    }

                    if (payload.type !== 'state') return

                    workflowStore.updateWorkflowStatus(
                        sessionId,
                        payload.state,
                        payload.wait_reason || null
                    )

                    const statusLower = String(payload.state || '').toLowerCase()
                    const isApprovalWaiting = payload.wait_reason === WORKFLOW_WAIT_REASONS.APPROVAL
                    const isAwaitingUser = payload.wait_reason === WORKFLOW_WAIT_REASONS.USER_INPUT

                    if (isAwaitingUser) {
                        showBackgroundAskUserNotification(sessionId)
                    } else {
                        clearPendingApprovalEntries(sessionId, 'ask_user')
                    }

                    if (!isApprovalWaiting) {
                        clearPendingApprovalEntries(sessionId, 'approval')
                    }
                    if (TERMINAL_STATUSES.includes(statusLower)) {
                        workflowStore.loadWorkflows().catch((error) => {
                            console.warn('[Workflow] Failed to refresh workflows after background terminal state:', error)
                        })
                        // Play completion sound when background workflow successfully completes
                        if (statusLower === WORKFLOW_STATUSES.COMPLETED) {
                            playCompletionSound()
                        }
                        const cleanup = backgroundStateListeners.get(sessionId)
                        if (cleanup) {
                            cleanup()
                            backgroundStateListeners.delete(sessionId)
                        }
                    }
                }, undefined, `backgroundWorkflowState:${sessionId}`)
            })

            backgroundStateListeners.set(sessionId, unlisten)
        }

        for (const workflow of workflows.value) {
            if (!workflow?.id || workflow.id === activeSessionId) continue

            const statusLower = String(workflow.status || '').toLowerCase()
            const waitReasonValue = workflow.waitReason || null

            if (
                statusLower === WORKFLOW_STATUSES.AWAITING_USER ||
                waitReasonValue === WORKFLOW_WAIT_REASONS.USER_INPUT
            ) {
                showBackgroundAskUserNotification(workflow.id)
            } else {
                clearPendingApprovalEntries(workflow.id, 'ask_user')
            }

            if (
                statusLower === WORKFLOW_STATUSES.AWAITING_APPROVAL ||
                waitReasonValue === WORKFLOW_WAIT_REASONS.APPROVAL
            ) {
                upsertPendingApprovalEntry(workflow.id, {
                    id: 'awaiting_approval',
                    action: t('workflow.awaitingApproval')
                })
                continue
            }

            clearPendingApprovalEntries(workflow.id, 'approval')
        }
    }

    /**
     * Setup workflow event listeners with error boundary
     * Phase 9: UI exceptions must be degradable, cannot block workflow execution
     */
    const setupWorkflowEvents = async (sessionId) => {
        // Update current session ID for event isolation
        currentSessionId.value = sessionId

        if (unlistenWorkflowEvents.value) {
            unlistenWorkflowEvents.value()
            unlistenWorkflowEvents.value = null
        }

        const eventName = `workflow://event/${sessionId}`
        unlistenWorkflowEvents.value = await listen(eventName, (event) => {
            // Phase 9: Session isolation check - ignore events from non-current sessions
            if (currentSessionId.value !== sessionId) {
                console.warn(`[Workflow] Ignoring event from non-active session: ${sessionId}`)
                return
            }

            // Phase 9: Error boundary - capture UI update exceptions
            safeExecute(() => {
                const payload = event.payload

                // Any event from this channel means the session is live on backend.
                if (workflowStore.currentWorkflowId === sessionId) {
                    workflowStore.setHasLiveSession(true)
                }

                if (payload.type === 'state') {
                    const prevState = workflowStore.currentWorkflow?.status
                    const prevWaitReason = workflowStore.waitReason
                    workflowStore.updateWorkflowStatus(sessionId, payload.state, payload.wait_reason || null)

                    const isWaiting = WAITING_STATUSES.includes(payload.state)
                    console.log(`[Workflow][state] ${prevState} -> ${payload.state} | wait_reason: ${payload.wait_reason || 'null'} | isWaiting: ${isWaiting}`)

                    if (TERMINAL_STATUSES.includes((payload.state || '').toLowerCase())) {
                        workflowStore.setHasLiveSession(false)
                        workflowStore.loadWorkflows().catch((error) => {
                            console.warn('[Workflow] Failed to refresh workflows after terminal state:', error)
                        })
                        // Play completion sound when workflow successfully completes
                        if ((payload.state || '').toLowerCase() === WORKFLOW_STATUSES.COMPLETED) {
                            playCompletionSound()
                        }
                    }

                    // Check for confirmation waiting
                    if (payload.state === WORKFLOW_STATUSES.PAUSED && payload.wait_reason === WORKFLOW_WAIT_REASONS.CONFIRMATION) {
                        showConfirmationDialog()
                    }

                    // If we move out of Thinking/Executing, reset the parser
                    if (payload.state !== WORKFLOW_STATUSES.THINKING && payload.state !== WORKFLOW_STATUSES.EXECUTING) {
                        setTimeout(() => {
                            safeExecute(() => resetChatState(), undefined, 'resetChatState')
                        }, 500)
                    }

                    const isApprovalWaiting = payload.wait_reason === WORKFLOW_WAIT_REASONS.APPROVAL
                    if (!isApprovalWaiting) {
                        clearPendingApprovalEntries(sessionId)
                        flushDeferredQueuedMessages().catch((error) => {
                            console.warn('Failed to flush deferred queue after state update:', error)
                        })
                    }
                } else if (payload.type === 'chunk') {
                    if (currentWorkflowId.value === sessionId && !workflowStore.hasLiveSession) return
                    // Direct text chunk from LLM or StreamParser
                    processChunk(payload.content)
                    scrollToBottom()
                } else if (payload.type === 'reasoning_chunk') {
                    if (currentWorkflowId.value === sessionId && !workflowStore.hasLiveSession) return
                    // Thinking chunk
                    processReasoningChunk(payload.content)
                    scrollToBottom()
                } else if (payload.type === 'message') {
                    if (currentWorkflowId.value === sessionId && !workflowStore.hasLiveSession) return
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

                    // Message finalized, clear chatting buffer (including reasoning)
                    resetChatState()

                    // Only auto-scroll when the user is still pinned near the bottom.
                    scrollToBottom()
                } else if (payload.type === 'confirm') {
                    // Current-session approvals are rendered inline in tool messages.
                    workflowStore.clearApprovalSubmission(sessionId, payload.id)
                    upsertPendingApprovalEntry(sessionId, payload)
                    playApprovalNotificationSound()
                } else if (payload.type === 'approval_resolved') {
                    clearPendingApprovalEntries(sessionId, 'approval')
                    workflowStore.clearApprovalSubmission(sessionId, payload.tool_call_id)
                    if (payload.approved) {
                        workflowStore.markToolApprovedRunning(payload.tool_call_id)
                    } else {
                        workflowStore.markToolRejected(payload.tool_call_id)
                    }
                } else if (payload.type === 'tool_started') {
                    clearPendingApprovalEntries(sessionId, 'approval')
                    workflowStore.clearApprovalSubmission(sessionId, payload.tool_call_id)
                    workflowStore.markToolApprovedRunning(payload.tool_call_id)
                } else if (payload.type === 'queued_user_message_removed') {
                    workflowStore.removeQueuedMessage(payload.queued_user_message_id)
                } else if (payload.type === 'retry_status') {
                    if (currentWorkflowId.value === sessionId && !workflowStore.hasLiveSession) return
                    // Handle 429 retry status
                    setRetryStatus(payload)
                } else if (payload.type === 'sync_todo') {
                    workflowStore.setTodoList(payload.todo_list)
                } else if (payload.type === 'agent_config_updated') {
                    const nextConfig = payload.agent_config || {}
                    applyWorkflowConfigToLocalStore(nextConfig)
                    if (workflowStore.currentWorkflow?.id === sessionId) {
                        workflowStore.currentWorkflow.allowedPaths = nextConfig.allowedPaths || []
                        workflowStore.currentWorkflow.shellPolicy = nextConfig.shellPolicy || []
                        workflowStore.setShellPolicy(nextConfig.shellPolicy || [])
                        workflowStore.setAutoApprovedTools(nextConfig.autoApprove || [])

                        isSyncingWorkflowConfig.value = true
                        planningMode.value =
                            String(nextConfig.phase || '').toLowerCase() === 'planning'
                        isSyncingWorkflowConfig.value = false
                    }
                } else if (payload.type === 'compression_status') {
                    // Handle context compression status
                    setCompressionStatus(payload.is_compressing, payload.message)
                    if (payload.is_compressing) {
                        scrollToBottom()
                    }
                } else if (payload.type === 'context_usage') {
                    workflowStore.setCurrentContextTokens(
                        sessionId,
                        payload.current_context_tokens ?? payload.total_tokens,
                        payload.max_context_tokens
                    )
                } else if (payload.type === 'sub_agent_progress') {
                    workflowStore.upsertSubAgentProgress(payload)
                } else if (payload.type === 'notification') {
                    if (currentWorkflowId.value === sessionId && !workflowStore.hasLiveSession) return
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
            }, undefined, `workflowEvent:${event.payload?.type || 'unknown'}`)
        })

        await syncBackgroundStateListeners()
    }

    /**
     * Select workflow with session isolation
     * Phase 9: Multi-session switching doesn't cross-contaminate, UI rendering exceptions don't affect workflow execution
     */
    const selectWorkflow = async (id) => {
        if (!canSwitchWorkflow.value) {
            console.warn('Cannot switch workflow while another is running')
            return
        }

        // Phase 9: Update session ID for event isolation
        currentSessionId.value = id

        // Phase 9: Clear previous session's UI state
        safeExecute(() => {
            // Reset chat state
            resetChatState()
            // Reset retry timer
            clearRetryTimer()
        }, undefined, 'cleanupPreviousSession')

        // Select the workflow in store (includes Task Ledger rebuild)
        try {
            await workflowStore.selectWorkflow(id)
        } catch (error) {
            console.error('[Workflow] selectWorkflow failed:', error)
            showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
            return
        }

        console.log('[Workflow] selectWorkflow completed, currentWorkflow:', workflowStore.currentWorkflow?.id, 'status:', workflowStore.currentWorkflow?.status)

        if (workflowStore.currentWorkflow) {
            const agent = agentStore.agents.find((a) => a.id === workflowStore.currentWorkflow.agentId)
            if (agent) {
                selectedAgent.value = agent
            }

            // Setup event listeners for the existing session (always setup, even if no agent)
            await setupWorkflowEvents(id)

            const status = workflowStore.currentWorkflow?.status?.toLowerCase()
            const pendingApprovalRequest = workflowStore.pendingApprovalRequest

            console.log('[Workflow] Checking session recovery:', status, 'workflow:', workflowStore.currentWorkflow?.id, 'hasLiveSession:', workflowStore.hasLiveSession)

            clearPendingApprovalEntries(id)
            workflowStore.clearApprovalSubmissionsForSession(id)

            if (status === WORKFLOW_STATUSES.AWAITING_APPROVAL) {
                const structuredPendingTools = getExecutionContextPendingTools(workflowStore.currentWorkflow)
                workflowStore.reconcilePendingApprovalSubmissions(
                    id,
                    structuredPendingTools
                        .map(tool => tool.toolCallId || tool.tool_call_id || '')
                        .filter(Boolean)
                )

                if (structuredPendingTools.length > 0) {
                    for (const pendingTool of structuredPendingTools) {
                        const toolCallId = pendingTool.toolCallId || pendingTool.tool_call_id || ''
                        const toolName = pendingTool.toolName || pendingTool.tool_name || ''
                        const details = pendingTool.details || ''
                        const displayType = pendingTool.displayType || pendingTool.display_type || ''
                        if (!toolCallId) continue

                        upsertPendingApprovalEntry(id, {
                            id: toolCallId,
                            action: toolName || t('workflow.awaitingApproval')
                        })
                        workflowStore.addMessage({
                            sessionId: id,
                            role: 'tool',
                            message: details,
                            stepType: 'Observe',
                            stepIndex: workflowStore.messages.length,
                            isError: false,
                            errorType: null,
                            metadata: {
                                tool_call_id: toolCallId,
                                tool_name: toolName,
                                display_type: displayType,
                                summary: t('workflow.awaitingApproval'),
                                approval_status: 'pending',
                                execution_status: 'pending_approval'
                            }
                        })
                    }
                } else if (pendingApprovalRequest && !workflowStore.pendingApprovalMessage) {
                    upsertPendingApprovalEntry(id, {
                        id: pendingApprovalRequest.toolCallId || 'awaiting_approval',
                        action: pendingApprovalRequest.toolName || t('workflow.awaitingApproval')
                    })
                    workflowStore.addMessage({
                        sessionId: id,
                        role: 'tool',
                        message: pendingApprovalRequest.details || '',
                        stepType: 'Observe',
                        stepIndex: workflowStore.messages.length,
                        isError: false,
                        errorType: null,
                        metadata: {
                            tool_call_id: pendingApprovalRequest.toolCallId || '',
                            tool_name: pendingApprovalRequest.toolName || '',
                            display_type: pendingApprovalRequest.displayType || '',
                            summary: t('workflow.awaitingApproval'),
                            approval_status: 'pending',
                            execution_status: 'pending_approval'
                        }
                    })
                }
            } else if (status !== WORKFLOW_STATUSES.AWAITING_APPROVAL) {
                clearPendingApprovalEntries(id)
            }

            if (status === WORKFLOW_STATUSES.PAUSED && workflowStore.waitReason === WORKFLOW_WAIT_REASONS.CONFIRMATION && workflowStore.hasLiveSession) {
                console.log('[Workflow] Workflow in live confirmation waiting, showing dialog')
                showConfirmationDialog()
            } else {
                console.log('[Workflow] No confirmation dialog recovery needed. status:', status)
            }

            // Initialize settings from workflow's agentConfig or fallback to agent defaults
            const config = workflowStore.currentWorkflow.agentConfig || {}

            // finalAuditMode
            isSyncingWorkflowConfig.value = true
            if (config.finalAudit !== undefined && config.finalAudit !== null) {
                finalAuditMode.value = config.finalAudit ? 'on' : 'off'
            } else if (selectedAgent.value?.finalAudit) {
                finalAuditMode.value = 'on'
            } else {
                finalAuditMode.value = 'off'
            }
            isSyncingWorkflowConfig.value = false

            // approvalLevel
            isSyncingWorkflowConfig.value = true
            if (config.approvalLevel) {
                approvalLevel.value = config.approvalLevel
            } else if (selectedAgent.value?.approvalLevel) {
                approvalLevel.value = selectedAgent.value.approvalLevel
            } else {
                approvalLevel.value = 'default'
            }
            isSyncingWorkflowConfig.value = false

            // planningMode is a workflow phase display, not a runtime hot-toggle for live sessions.
            isSyncingWorkflowConfig.value = true
            planningMode.value = String(config.phase || '').toLowerCase() === 'planning'
            isSyncingWorkflowConfig.value = false
        }

        // Scroll to bottom after switching workflow (force scroll)
        nextTick(() => {
            console.log('Scrolling to bottom after switching workflow')
            scrollToBottom(true)
        })
    }

    const showConfirmationDialog = async () => {
        ElMessageBox.confirm(t('workflow.confirmationWaiting'), t('workflow.confirmationTitle'), {
            confirmButtonText: t('workflow.continue'),
            cancelButtonText: t('workflow.stop'),
            type: 'warning',
            showClose: false,
            closeOnClickModal: false,
            closeOnPressEscape: false
        }).then(async () => {
            console.log('[Workflow] User chose to continue')
            const signal = JSON.stringify({ type: SIGNAL_TYPES.CONTINUE })
            try {
                await invokeWrapper('workflow_signal', {
                    sessionId: currentWorkflowId.value,
                    signal
                })
            } catch (error) {
                console.error('Failed to send continue signal:', error)
            }
        }).catch(async () => {
            console.log('[Workflow] User chose to stop')
            // Immediately update UI state
            workflowStore.setRunning(false)
            clearRetryTimer()
            resetChatState()
            workflowStore.setNotification('', 'info')

            const signal = JSON.stringify({ type: SIGNAL_TYPES.STOP })
            try {
                await invokeWrapper('workflow_signal', {
                    sessionId: currentWorkflowId.value,
                    signal
                })
            } catch (error) {
                console.error('Failed to send stop signal:', error)
            }
        })
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

                    // Update workflow user_query in backend. Title should be generated by AI.
                    await invokeWrapper('update_workflow_query', {
                        sessionId: currentWorkflowId.value,
                        userQuery: prompt
                    })

                    // Persist any unsaved local UI config toggles before engine startup.
                    await persistCurrentWorkflowUiConfigBeforeStart()

                    // Ensure event listener is attached before starting runtime,
                    // otherwise early UI events (e.g. approval confirm) can be missed.
                    await setupWorkflowEvents(currentWorkflowId.value)

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
                        const baseConfig =
                            typeof snapshot.workflow.agentConfig === 'string'
                                ? JSON.parse(snapshot.workflow.agentConfig)
                                : snapshot.workflow.agentConfig
                        inheritedAgentConfig = JSON.stringify(mergeLocalUiOverrides(baseConfig || {}))
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
            // Phase 3: Use unified waiting check - all waiting states should send signal
            // Backend will validate signal type based on wait_reason
            if (isRunning.value || isWaiting.value) {
                const shouldQueueLocally =
                    isRunning.value || waitReason.value === WORKFLOW_WAIT_REASONS.APPROVAL
                let queuedId = null
                if (shouldQueueLocally) {
                    queuedId = `local_queue_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
                    workflowStore.addMessageToQueue({
                        id: queuedId,
                        content: message,
                        status:
                            waitReason.value === WORKFLOW_WAIT_REASONS.APPROVAL
                                ? 'pending_approval'
                                : 'queued',
                        sent: false
                    })
                }

                // Just send signal to the running loop
                try {
                    // Optimistic update only for states that immediately consume user input.
                    const shouldOptimisticUpdate =
                        waitReason.value === WORKFLOW_WAIT_REASONS.USER_INPUT ||
                        waitReason.value === WORKFLOW_WAIT_REASONS.CONFIRMATION
                    if (shouldOptimisticUpdate) {
                        workflowStore.updateWorkflowStatus(currentWorkflowId.value, WORKFLOW_STATUSES.THINKING)
                    }

                    const res = await sendUserMessageSignal(currentWorkflowId.value, message, queuedId)
                    console.log('Signal sent successfully:', res)

                    if (queuedId) {
                        workflowStore.markQueuedMessageSent(queuedId)
                    }
                } catch (error) {
                    if (queuedId) {
                        workflowStore.removeQueuedMessage(queuedId)
                    }
                    console.error('Failed to send signal:', error)
                }
            } else {
                // Engine is stopped (Completed, Error, or Cancelled).
                // DO NOT add message manually here, workflow_start will handle it and broadcast via events.
                try {
                    // Ensure event listener is setup for this session
                    await setupWorkflowEvents(currentWorkflowId.value)

                    await invokeWrapper('workflow_start', {
                        sessionId: currentWorkflowId.value,
                        agentId: selectedAgent.value.id,
                        initialPrompt: message,
                        planningMode: planningMode.value
                    })
                } catch (error) {
                    const errorText = String(error)
                    // Recovery path: session is already active in manager, route as user_message signal.
                    if (errorText.includes('Session already exists')) {
                        try {
                            workflowStore.setHasLiveSession(true)
                            await sendUserMessageSignal(currentWorkflowId.value, message)
                            return
                        } catch (signalError) {
                            console.error('Failed to fallback to workflow_signal after Session already exists:', signalError)
                            showMessage(t('workflow.startFailed', { error: String(signalError) }), 'error')
                        }
                    }
                    console.error('Failed to resume workflow:', error)
                    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
                }
            }
        }
    }

    const removeQueuedMessage = async (queuedId) => {
        if (!queuedId) return

        const queuedItem = workflowStore.messageQueue.find((item) => item.id === queuedId)
        if (!queuedItem) return

        workflowStore.removeQueuedMessage(queuedId)

        if (!queuedItem.sent || !currentWorkflowId.value) {
            return
        }

        try {
            await removeQueuedUserMessageSignal(currentWorkflowId.value, queuedId)
        } catch (error) {
            workflowStore.addMessageToQueue(queuedItem)
            console.error('Failed to remove queued message from workflow:', error)
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
            workflowStore.updateWorkflowStatus(currentWorkflowId.value, WORKFLOW_STATUSES.CANCELLED, null)
            workflowStore.setRunning(false)
            workflowStore.setHasLiveSession(false)

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

                // Signal the live executor to update runtime config. Use the backend
                // liveness flag instead of a hand-maintained status allowlist so waiting
                // states such as AwaitingSubAgent are covered.
                const status = String(currentWorkflow.value?.status || snapshot.workflow?.status || '').toLowerCase()
                const isLiveSession = !!snapshot.hasLiveSession || workflowStore.hasLiveSession
                const isRuntimeState =
                    (RUNNING_STATUSES as readonly string[]).includes(status) ||
                    (WAITING_STATUSES as readonly string[]).includes(status)
                if (isLiveSession && isRuntimeState) {
                    try {
                        await invokeWrapper('workflow_signal', {
                            sessionId: currentWorkflowId.value,
                            signal: JSON.stringify({
                                type: SIGNAL_TYPES.UPDATE_MODEL_CONFIG,
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

                const savedWorkflowId = settingStore.settings.workflowLastSelectedId
                const restoredWorkflow = savedWorkflowId
                    ? workflows.value.find((workflow) => workflow.id === savedWorkflowId)
                    : null

                // Restore the last selected workflow if it still exists, otherwise fall back to the latest one.
                if (restoredWorkflow) {
                    await selectWorkflow(restoredWorkflow.id)
                } else if (workflows.value.length > 0) {
                    await selectWorkflow(workflows.value[0].id)
                }
            } catch (error) {
                console.error('Failed to delete workflow:', error)
            }
        })
    }

    const createNewWorkflow = async () => {
        try {
            // 0. Ensure we have an agent selected
            if (!selectedAgent.value) {
                // Try to select the first available agent
                if (agentStore.agents.length > 0) {
                    selectedAgent.value = agentStore.agents[0]
                    console.log('[Workflow] Auto-selected first agent:', selectedAgent.value.id)
                } else {
                    // No agents available - show error
                    const errorMsg = t('workflow.noAgentError') || 'No agent available. Please create an agent first.'
                    console.error('[Workflow] Cannot create workflow: no agent available')
                    showMessage(errorMsg, 'error')
                    return
                }
            }

            // 1. Get current config to inherit
            let inheritedAgentConfig = null
            let inheritedAgentId = selectedAgent.value?.id
            let inheritedAllowedPaths = []
            let inheritedFinalAudit = null
            let inheritedApprovalLevel = null

            if (workflowStore.currentWorkflow?.agentConfig) {
                inheritedAgentConfig = JSON.stringify(
                    mergeLocalUiOverrides(workflowStore.currentWorkflow.agentConfig)
                )
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
            } else {
                inheritedAgentConfig = JSON.stringify(
                    mergeLocalUiOverrides({
                        allowedPaths: pendingPaths.value.length > 0 ? [...pendingPaths.value] : undefined
                    })
                )
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
            // IMPORTANT: use core-level selectWorkflow to bind event listener and
            // recover any waiting UI state from live session/snapshot.
            await selectWorkflow(newWorkflowId)

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
        if (isSyncingWorkflowConfig.value) return
        await updateWorkflowConfig('approvalLevel', newVal)
    })

    // Watch for final audit mode changes
    watch(finalAuditMode, async (newVal) => {
        if (isSyncingWorkflowConfig.value) return
        await updateWorkflowConfig('finalAudit', newVal === 'on')
    })

    watch(planningMode, async (newVal) => {
        if (isSyncingWorkflowConfig.value) return
        await updateWorkflowConfig('phase', newVal ? 'planning' : 'standard')
    })

    watch(
        () => [
            currentWorkflowId.value,
            ...workflows.value.map((workflow) => `${workflow.id}:${String(workflow.status || '').toLowerCase()}`)
        ],
        () => {
            syncBackgroundStateListeners().catch((error) => {
                console.warn('[Workflow] Failed to sync background workflow listeners:', error)
            })
        },
        { immediate: true }
    )

    onBeforeUnmount(() => {
        teardownBackgroundStateListeners()
    })

    return {
        unlistenWorkflowEvents,
        currentSessionId,
        modelSelectorVisible,
        modelSelectorTab,
        modelSelectorMode,
        editWorkflowDialogVisible,
        editWorkflowId,
        editWorkflowTitle,
        workflows,
        isRunning,
        isWaiting,
        waitReason,
        hasLiveSession,
        isLiveWaiting,
        canStop,
        canContinue,
        canApprovePlan,
        isAwaitingApproval,
        pendingApprovalList,
        getPendingApprovalEntry,
        clearPendingApprovalEntry,
        activeModelName,
        canSwitchWorkflow,
        setupWorkflowEvents,
        selectWorkflow,
        startNewWorkflow,
        onSendMessage,
        removeQueuedMessage,
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
        toggleFinalAuditMode,
        playCompletionSound
    }
}
