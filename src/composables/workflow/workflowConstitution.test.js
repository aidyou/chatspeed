import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import {
  WORKFLOW_SUB_AGENT_TOOL_NAMES,
  WORKFLOW_TODO_TOOL_NAMES
} from './toolClassification.js'

const projectRoot = new URL('../../../', import.meta.url)
const readProjectFile = relativePath =>
  readFileSync(new URL(relativePath, projectRoot), 'utf8')

const sourceSection = (source, startMarker, endMarker) => {
  const start = source.indexOf(startMarker)
  assert.notEqual(start, -1, `missing source marker: ${startMarker}`)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(end, -1, `missing source marker: ${endMarker}`)
  return source.slice(start, end)
}

const projectionRules = readProjectFile('src/composables/workflow/messageProjectionRules.js')
const structuredToolNameRule = sourceSection(
  projectionRules,
  'export const getStructuredWorkflowToolName',
  'export const isPendingApprovalEntryForTool'
)
assert.match(structuredToolNameRule, /metadata\.tool_name/)
assert.match(structuredToolNameRule, /metadata\.tool_call\?\.function\?\.name/)
assert.doesNotMatch(
  structuredToolNameRule,
  /metadata\.toolName|metadata\.title|metadata\.action|message\?*\.message|metadata\.content/
)

const completionRule = sourceSection(
  projectionRules,
  'export const isWorkflowCompletionMessage',
  'export const shouldRenderSubAgentCard'
)
assert.match(completionRule, /getStructuredWorkflowToolName\(message\) === 'complete_workflow'/)
assert.doesNotMatch(completionRule, /title|action|content|includes\(|startsWith\(/)

const toolStateMapper = readProjectFile('src/composables/workflow/useToolStateMapper.ts')
const metadataContract = sourceSection(
  toolStateMapper,
  'export interface MessageMetadata',
  '/** Tool call information */'
)
assert.match(metadataContract, /tool_name\?: string/)
assert.doesNotMatch(metadataContract, /toolName\?:/)

const extractToolName = sourceSection(
  toolStateMapper,
  'function extractToolName',
  'function extractArguments'
)
assert.match(extractToolName, /getStructuredWorkflowToolName\(message\)/)
assert.doesNotMatch(extractToolName, /title|action|message\.message|content/)

const workflowCore = readProjectFile('src/composables/workflow/useWorkflowCore.ts')
const workflowChat = readProjectFile('src/composables/workflow/useWorkflowChat.ts')
assert.match(workflowCore, /autoApprovePlan: autoApprovePlan\.value/)
assert.match(workflowCore, /'autoApprovePlan'/)
assert.match(
  workflowCore,
  /watch\(autoApprovePlan,[\s\S]*updateWorkflowConfig\('autoApprovePlan', !!newVal\)/
)
const approvePlan = sourceSection(workflowCore, 'const onApprovePlan', 'const onStop')
assert.doesNotMatch(approvePlan, /entry\?*\.action|includes\(['"]submit plan['"]\)/i)
assert.match(approvePlan, /isPendingApprovalEntryForTool\(entry, currentSessionId, 'submit_plan'\)/)

const approvalResolvedHandler = sourceSection(
  workflowCore,
  "} else if (payload.type === 'approval_resolved') {",
  "} else if (payload.type === 'tool_started') {"
)
assert.match(approvalResolvedHandler, /payload\.tool_name === 'submit_plan'/)
assert.match(approvalResolvedHandler, /resolvePendingTool\(sessionId, payload\.tool_call_id\)/)

const messageList = readProjectFile('src/components/workflow/WorkflowMessageList.vue')
const workflowMessageStyles = readProjectFile('src/styles/workflow/messages.scss')
const themeVariables = readProjectFile('src/style/element/css-vars.css')
assert.match(messageList, /:tool-name="getMessageToolName\(message\)"/)
assert.equal(
  messageList.match(/:class="getToolDetailClass\((?:tool|message)\)"/g)?.length,
  3,
  'all expanded tool detail projections must expose a normalized tool-name class'
)
assert.match(messageList, /return normalizedToolName/)
assert.match(
  workflowMessageStyles,
  /&\.ask_user \{[\s\S]*padding: 0;[\s\S]*background: none;[\s\S]*border-radius: 0;/
)
assert.match(workflowMessageStyles, /&\.ask_user \{[\s\S]*\.choice-container \{[\s\S]*padding: 5px;/)
assert.doesNotMatch(messageList, /:action="message\.metadata\?\.tool_name/)
assert.match(
  messageList,
  /isWorkflowMessagePendingApproval\(message, pendingApprovalIdSet\.value\)/
)
assert.equal(
  messageList.match(/class="bash-command-frame"/g)?.length,
  3,
  'all Bash result projections must use the approval-style command frame'
)
assert.equal(
  messageList.match(/class="bash-command-frame__prompt" aria-hidden="true">\$<\/span>/g)?.length,
  3,
  'all Bash result projections must show the approval-style shell prompt'
)
const bashCommandStyles = sourceSection(
  workflowMessageStyles,
  '            .bash-command-frame {',
  '            .tool-stream-output {'
)
assert.match(bashCommandStyles, /grid-template-columns: auto minmax\(0, 1fr\)/)
assert.match(bashCommandStyles, /&__prompt \{[\s\S]*color: var\(--cs-color-primary\)/)
assert.match(bashCommandStyles, /\.bash-command \{[\s\S]*border: 0/)
assert.match(bashCommandStyles, /\.bash-command \{[\s\S]*background: transparent/)
const toolGroupProjectionStart = projectionRules.indexOf('export const projectWorkflowMessageList')
assert.notEqual(toolGroupProjectionStart, -1, 'missing centralized message-list projection')
const toolGroupProjection = projectionRules.slice(toolGroupProjectionStart)
assert.match(toolGroupProjection, /isWorkflowMcpTool\(toolName\)/)
assert.match(toolGroupProjection, /mcp_tools/)
assert.match(projectionRules, /mcp_tools: 'mcp'/)
assert.match(toolGroupProjection, /mixed_tools/)
assert.match(
  toolGroupProjection,
  /const buildToolGroupSummary = groupMessages => \{[\s\S]*const counts = new Map\(\)/,
  'tool groups must aggregate repeated operations into a compact title'
)
assert.match(
  toolGroupProjection,
  /Array\.from\(counts, \(\[label, count\]\) => `\$\{label\} x\$\{count\}`\)\.join\(' · '\)/,
  'tool group summaries must display compact label counts'
)
assert.match(
  toolGroupProjection,
  /if \(TODO_TOOL_NAMES\.has\(toolName\)\) return translate\('workflow\.toolGroups\.taskChanges'\)/,
  'todo tools must aggregate under the task changes operation label'
)
assert.match(
  toolGroupProjection,
  /if \(isWorkflowMcpTool\(toolName\)\) return translate\('workflow\.toolGroups\.callMcp'\)/,
  'MCP tools must aggregate under the MCP operation label'
)
assert.match(
  toolGroupProjection,
  /return translate\(TOOL_GROUP_LABEL_KEYS\[toolName\] \|\| 'workflow\.toolGroups\.useTool'\)/,
  'known tools must aggregate by their localized operation label'
)
assert.match(
  toolGroupProjection,
  /return `tool_group:\$\{firstToolCallId \|\| firstId\}`/,
  'tool-group identity must remain stable as tools append or transition from pending to completed'
)
assert.match(
  toolGroupProjection,
  /const projectPendingToolGroups = input =>/,
  'auto-executing pending tool calls must use the same grouped presentation before results arrive'
)
assert.match(
  messageList,
  /projectWorkflowMessageList\(props\.messages/,
  'the visible message projection must include running pending tool groups'
)
assert.match(workflowMessageStyles, /background: linear-gradient\(/)
assert.match(workflowMessageStyles, /color-mix\(in srgb, var\(--cs-text-color-primary\)/)
assert.match(workflowMessageStyles, /animation: tool-group-title-shimmer 3s ease-in-out infinite/)
assert.match(workflowMessageStyles, /50% \{[\s\S]*background-position: 0 0/)
assert.match(workflowMessageStyles, /100% \{[\s\S]*background-position: 0 0/)
assert.match(
  themeVariables,
  /:root\.light[\s\S]*--cs-shimmer-inverse-color: rgba\(255, 255, 255, 0\.55\)/
)
assert.match(
  themeVariables,
  /:root\.dark[\s\S]*--cs-shimmer-inverse-color: rgba\(0, 0, 0, 0\.55\)/
)
assert.match(
  toolGroupProjection,
  /while \(nextIndex < input\.length && getCollapsibleToolGroupKind\(input\[nextIndex\]\)\)/
)
assert.match(
  toolGroupProjection,
  /const isApprovalPending = message =>\s*isWorkflowMessagePendingApproval\(message, pendingApprovalIds\)/,
  'only the canonical pending-approval ID set may exclude a tool from grouping'
)
assert.doesNotMatch(
  toolGroupProjection,
  /approval_submitted/,
  'approval-submitted and running tools must join their continuous tool group'
)

const workflowMessagesProjection = readProjectFile('src/composables/workflow/useWorkflowMessages.ts')
assert.match(workflowMessagesProjection, /isWorkflowManualClearContextMessage/)
assert.doesNotMatch(
  workflowMessagesProjection,
  /\bisManualClearContextMessage\b/,
  'the message composable must use the imported shared manual-clear projection rule'
)

const toolIcons = readProjectFile('src/composables/workflow/toolIcons.ts')
assert.match(toolIcons, /isWorkflowMcpTool\(normalized\)\) return 'mcp'/)

const workflowView = readProjectFile('src/views/Workflow.vue')
const globalApprovalProjection = sourceSection(
  workflowView,
  'const approvalQueueCount = computed(() => {',
  'const canDeleteLastMessage = computed(() => {'
)
assert.match(globalApprovalProjection, /globalPendingApprovalList\.value\.length/)
assert.match(globalApprovalProjection, /const backgroundEntries = pendingApprovalList\.value\.filter/)
assert.match(globalApprovalProjection, /entry\.sessionId !== activeSessionId/)
assert.match(
  globalApprovalProjection,
  /const currentEntries = workflowStore\.currentInlinePendingApprovals/
)
assert.match(
  globalApprovalProjection,
  /const merged = \[\.\.\.currentEntries, \.\.\.currentAskUserEntry, \.\.\.backgroundEntries\]/
)
assert.match(
  globalApprovalProjection,
  /const key = `\$\{entry\?\.sessionId \|\| ''\}:\$\{entry\?\.id \|\| ''\}`/,
  'global action reminders must deduplicate per session and approval, not collapse all sessions'
)
const queuedImageSend = sourceSection(
  workflowView,
  'inputComposable.onSendMessage.value = async () => {',
  '// ============================================================\n// Wrapper functions combining multiple composables'
)
assert.ok(
  queuedImageSend.indexOf('const messageTarget = {') <
    queuedImageSend.indexOf('await analyzeImageAttachments('),
  'image sends must capture their workflow target before asynchronous attachment analysis'
)
assert.match(queuedImageSend, /sessionId: workflowStore\.currentWorkflowId/)
assert.match(queuedImageSend, /target: messageTarget/)
assert.match(
  workflowView,
  /const inFlightDraftSessionIds = new Set\(\)/,
  'workflow switches must know which captured draft is protected by an in-flight send'
)
assert.match(
  workflowView,
  /const hydrationRevision = \+\+inputDraftHydrationRevision[\s\S]*await restoreDraftImageAttachments[\s\S]*hydrationRevision !== inputDraftHydrationRevision[\s\S]*workflowStore\.currentWorkflowId !== sessionId[\s\S]*return[\s\S]*inputMessage\.value = draft\?\.inputMessage \|\| ''[\s\S]*imageAttachments\.value = restoredAttachments/,
  'draft hydration must ignore stale async attachment restores after workflow switches'
)
assert.match(
  workflowView,
  /if \(hydrationRevision === inputDraftHydrationRevision\) \{[\s\S]*isHydratingInputDraft = false/,
  'stale draft hydration must not reset a newer hydration guard'
)
assert.match(
  queuedImageSend,
  /inFlightDraftSessionIds\.add\(messageTarget\.sessionId\)[\s\S]*saveCapturedInputDraft\(messageTarget\.sessionId, backupMessage, backupAttachments\)/,
  'image sends must persist and protect the captured target draft before clearing visible input'
)
assert.match(
  queuedImageSend,
  /saveCapturedInputDraft\(messageTarget\.sessionId, backupMessage, backupAttachments\)[\s\S]*if \(workflowStore\.currentWorkflowId === messageTarget\.sessionId\) \{[\s\S]*inputMessage\.value = backupMessage/,
  'image analysis failure must retain the captured target draft even after switching workflows'
)
assert.match(
  queuedImageSend,
  /if \(sendResult === false\) \{[\s\S]*saveCapturedInputDraft\(messageTarget\.sessionId, backupMessage, backupAttachments\)[\s\S]*workflowStore\.currentWorkflowId === messageTarget\.sessionId[\s\S]*inputMessage\.value = backupMessage/,
  'dispatch failure must retain the captured target draft while restoring visible input only for the active matching workflow'
)
assert.match(
  queuedImageSend,
  /else if \(sendResult === true\) \{[\s\S]*removeWorkflowInputDraft\(messageTarget\.sessionId\)/,
  'a successful captured send must clear that workflow draft even after switching workflows'
)
assert.match(
  queuedImageSend,
  /if \(workflowStore\.currentWorkflowId === messageTarget\.sessionId\) \{[\s\S]*clearRecoverableWorkflowErrorMessages\(\)/,
  'successful background sends must only clear visible recoverable errors for the active matching workflow'
)
assert.match(
  queuedImageSend,
  /inFlightDraftSessionIds\.delete\(messageTarget\.sessionId\)/,
  'in-flight draft protection must be released after terminal send paths'
)
const automationSelection = sourceSection(
  workflowView,
  'const onSelectAutomation = async',
  'const onDeleteAutomation = async'
)
assert.match(automationSelection, /selectionRevision = \+\+workflowSelectionIntentRevision/)
assert.match(
  automationSelection,
  /selectionRevision !== workflowSelectionIntentRevision[\s\S]*return/
)
const workflowInputArea = readProjectFile('src/components/workflow/WorkflowInputArea.vue')
assert.doesNotMatch(
  workflowInputArea,
  /<!-- Approval Level Dropdown -->/,
  'approval must no longer have a standalone footer button'
)
assert.match(
  workflowInputArea,
  /command="autoCompress"[\s\S]*quick-actions-section-title[\s\S]*settings\.agent\.approvalLevel[\s\S]*command="approvalDefault"/,
  'approval choices must be grouped after auto rollup compression'
)
const visibleAutoApprovalTools = sourceSection(
  workflowInputArea,
  'const workflowAvailableToolIds',
  'const allowedShellCommands'
)
const nativeToolProjection = sourceSection(
  workflowInputArea,
  'const agentAvailableTools',
  'const workflowMcpTools'
)
assert.match(
  nativeToolProjection,
  /props\.selectedAgent\?\.availableTools[\s\S]*configuredNativeTools/
)
assert.doesNotMatch(
  nativeToolProjection,
  /return workflowAvailableToolIds\.value/,
  'native capability rows must remain visible after a workflow preference is unchecked'
)
const mcpToolProjection = sourceSection(
  workflowInputArea,
  'const workflowMcpTools',
  'const workflowAvailableToolIds'
)
assert.match(
  mcpToolProjection,
  /currentlyAvailableMcpTools[\s\S]*currentlyAvailableMcpTools, \.\.\.configuredMcpTools/
)
assert.doesNotMatch(
  mcpToolProjection,
  /props\.selectedAgent\?\.mcpTools\?\.available/
)
assert.ok(
  visibleAutoApprovalTools.indexOf('props.currentWorkflow?.agentConfig?.availableTools') <
    visibleAutoApprovalTools.indexOf('props.selectedAgent?.availableTools'),
  'workflow tool capabilities must take precedence over a newer unsynchronized Agent definition'
)
assert.match(visibleAutoApprovalTools, /filter\(tool => availableSet\.has\(tool\)\)/)
assert.match(
  workflowInputArea,
  /<el-tabs v-model="approvalToolsTab"[\s\S]*settings\.agent\.availableTools[\s\S]*workflow\.toolConfig[\s\S]*workflow\.allowedShellCommands/
)
assert.match(workflowInputArea, /workflow\.mcpConfig[\s\S]*name="mcp"|name="mcp"[\s\S]*workflow\.mcpConfig/)
assert.match(workflowInputArea, /settings\.agent\.mcpToolAvailable[\s\S]*settings\.agent\.mcpToolAutoApprove[\s\S]*settings\.agent\.mcpToolAutoExpand/)
assert.match(
  workflowInputArea,
  /toggleWorkflowAvailableTool[\s\S]*availableTools: nextAvailableTools[\s\S]*autoApprove: nextAutoApprove/
)
assert.match(workflowInputArea, /const SHELL_POLICY_PAGE_SIZE = 10/)
assert.match(workflowInputArea, /v-for="\(rule, idx\) in paginatedShellPolicyRules"/)
assert.match(workflowInputArea, /:page-size="SHELL_POLICY_PAGE_SIZE"/)
assert.match(workflowCore, /'availableTools'/)
assert.match(workflowCore, /'sandboxOverride'/)
assert.match(
  workflowCore,
  /normal new workflow sends no inherited configuration[\s\S]*checked `availableTools` set/
)

const workflowCommands = readProjectFile('src-tauri/src/commands/workflow.rs')
assert.match(
  workflowCommands,
  /user left checked[\s\S]*Agent's tool list/
)
assert.match(
  workflowCommands,
  /filter\(\|tool\| inherited_tools\.contains\(\*tool\)\)/
)

const workflowConstitution = readProjectFile(
  'src-tauri/src/workflow/react/CONSTITUTION.md'
)
assert.match(workflowConstitution, /Auto-approved tools must be visible tools/)
assert.match(workflowConstitution, /auto-approved tool set must be a subset/)
assert.match(workflowConstitution, /Shell approval policy remains separate and cumulative/)
assert.match(workflowConstitution, /workflow-level shell `Allow` rules/)

assert.match(
  workflowInputArea,
  /v-if="showPlanningModeToggle && planningMode"[\s\S]*command="autoApprovePlan"/
)
assert.match(workflowInputArea, /emit\('toggle-auto-approve-plan'\)/)
assert.match(workflowInputArea, /:hide-on-click="false"/)
assert.match(
  workflowInputArea,
  /command === 'attachment'[\s\S]*quickActionsDropdownRef\.value\?\.handleClose\?\.\(\)[\s\S]*command === 'skillsConfig'[\s\S]*quickActionsDropdownRef\.value\?\.handleClose\?\.\(\)/
)
assert.match(
  workflowInputArea,
  /command="modelConfig"[\s\S]*<cs name="model"[\s\S]*command="skillsConfig"/
)
assert.match(workflowInputArea, /quickActionsConfiguration[\s\S]*quickActionsRuntime/)
assert.match(workflowInputArea, /modelSelectorOpen[\s\S]*nextTick\(\)[\s\S]*scrollIntoView\(\{ block: 'nearest' \}\)/)
assert.match(
  workflowInputArea,
  /event\.key !== 'Escape'[\s\S]*modelSelectorOpen\.value = false[\s\S]*document\.addEventListener\('keydown', closeOnEscape\)/
)
assert.match(workflowInputArea, /<cs name="stop" @click="confirmStop" v-if="canStop" \/>/)
assert.match(
  workflowInputArea,
  /ElMessageBox\.confirm\(t\('workflow\.stopConfirmMessage'\), t\('workflow\.stopConfirmTitle'\)[\s\S]*emit\('stop'\)/
)
const workflowMessages = readProjectFile('src/composables/workflow/useWorkflowMessages.ts')
assert.doesNotMatch(
  workflowMessages,
  /visibleTaskGroupCount\.value\s*=\s*DEFAULT_VISIBLE_TASK_GROUPS/,
  'normal task transitions must preserve the history window explicitly expanded by the user'
)
const workflowSidebar = readProjectFile('src/components/workflow/WorkflowSidebar.vue')
const compactWorkflowProjection = sourceSection(
  workflowSidebar,
  'const compactActiveWorkflows',
  'const filteredAutomations'
)
assert.match(compactWorkflowProjection, /props\.workflows\.filter/)
assert.match(compactWorkflowProjection, /props\.workflows[\s\S]*\.slice\(0, 5\)/)
assert.doesNotMatch(
  compactWorkflowProjection,
  /filteredWorkflows\.value/,
  'compact mode must ignore expanded-mode search and directory filters'
)

const reactEngine = readProjectFile('src-tauri/src/workflow/react/engine.rs')
const reactLlm = readProjectFile('src-tauri/src/workflow/react/llm.rs')
assert.doesNotMatch(reactEngine, /DEFAULT_MAX_STEPS|STEP BUDGET|max-step budget|self\.max_steps/)
assert.doesNotMatch(
  reactEngine,
  /Step budget:/,
  'runtime reminders must not pressure the model with a removed step limit'
)
const postMessageCompression = sourceSection(
  reactEngine,
  'async fn maybe_run_blocking_compression_after_message',
  'async fn apply_background_compression_ready'
)
assert.match(postMessageCompression, /if needs_compression/)
assert.match(postMessageCompression, /build_pressure_compression_candidate\(\)/)
assert.match(postMessageCompression, /"context_pressure"/)
const runLoopFinalization = sourceSection(
  reactEngine,
  'let result = self.run_loop_internal().await;',
  'async fn begin_new_context_segment'
)
assert.match(runLoopFinalization, /self\.update_state\(WorkflowState::Error\)\.await/)
assert.doesNotMatch(
  reactLlm,
  /AI server error\. Retrying|AI server returned an empty response\. Retrying/
)
assert.doesNotMatch(workflowCore, /confirmationWaiting|showConfirmationDialog/)
const workflowStore = readProjectFile('src/stores/workflow.js')
const terminalResumeProjection = sourceSection(
  workflowCore,
  'const onSendMessage = async',
  'const removeQueuedMessage = async'
)
assert.match(
  terminalResumeProjection,
  /client_message_id: clientMessageId/,
  'terminal-session resume must assign a stable local-to-persisted user-message identity'
)
assert.match(
  terminalResumeProjection, /workflowStore\.addMessage\(\{[\s\S]*id: `temporary_\$\{clientMessageId\}`/)
assert.match(
  terminalResumeProjection, /removeCurrentWorkflowMessages\([\s\S]*client_message_id === clientMessageId/)
assert.match(
  workflowStore, /incomingClientMessageId = message\.metadata\?\.client_message_id/)
assert.match(
  workflowStore, /m\.metadata\?\.client_message_id === incomingClientMessageId/)
assert.doesNotMatch(
  workflowStore,
  /startsNewTaskAfterPartialHistory|messages\.value = \[\][\s\S]*hiddenCompletedTaskCount\.value \+=/
)
const streamingReasoningContract = sourceSection(
  workflowChat,
  'const refreshDerivedChatState',
  '// Handle retry status'
)
assert.match(streamingReasoningContract, /hadStreamingReasoning/)
assert.match(streamingReasoningContract, /source === 'reasoning' \|\| hasOpenThink \|\| hadStreamingReasoning/)
assert.match(messageList, /const hasStreamingThoughtCompleted = computed\([\s\S]*reasoningStatus === 'done'/)

const queuedMessageRouting = sourceSection(
  workflowCore,
  'const flushDeferredQueuedMessages',
  '// Track the current session ID for event isolation'
)
assert.match(queuedMessageRouting, /item\.sessionId === activeSessionId/)
assert.match(queuedMessageRouting, /sendUserMessageSignal\(activeSessionId/)
const sendMessageRouting = sourceSection(
  workflowCore,
  'const onSendMessage = async',
  'const removeQueuedMessage = async'
)
assert.match(sendMessageRouting, /targetSessionId = hasExplicitTarget/)
assert.match(sendMessageRouting, /sendUserMessageSignal\([\s\S]*targetSessionId/)
assert.doesNotMatch(sendMessageRouting, /sendUserMessageSignal\(\s*currentWorkflowId\.value/)
const removeQueuedMessageRouting = sourceSection(
  workflowCore,
  'const removeQueuedMessage = async',
  'const handleBuiltinCommand = async'
)
assert.match(
  removeQueuedMessageRouting,
  /removeQueuedUserMessageSignal\(queuedItem\.sessionId, queuedId\)/
)
assert.match(workflowStore, /sessionId: message\.sessionId \|\| currentWorkflowId\.value \|\| null/)
assert.match(
  workflowStore,
  /queuedUserMessages: Array\.isArray\(ctx\.queued_user_messages\)[\s\S]*queued_user_message_id/
)
assert.match(
  workflowStore,
  /const hydrateQueuedMessages = \(executionContext, workflowId\) =>[\s\S]*normalizedContext\?\.sessionId === workflowId[\s\S]*messageQueue\.value = queuedMessages/
)
assert.match(
  workflowStore,
  /hydrateQueuedMessages\(snapshot\.workflow\.executionContext, workflowId\)/
)
assert.match(
  workflowStore,
  /get_auto_approved_tools[\s\S]*currentWorkflowId\.value !== workflowId \|\| messageLoadRevision !== requestRevision/,
  'late workflow hydration must not overwrite the newly selected workflow'
)
const workflowSelection = sourceSection(
  workflowCore,
  'const selectWorkflow = async',
  'const startNewWorkflow = async'
)
assert.match(workflowSelection, /previousInlineApprovals/)
assert.match(
  workflowSelection,
  /upsertPendingApprovalEntry\(previousWorkflowId/,
  'switching sessions must retain the previous session approvals in the global reminder cache'
)
assert.match(workflowSelection, /workflowStore\.currentWorkflowId !== id/)
assert.match(workflowSelection, /currentSessionId\.value !== id/)
const workflowEventSetup = sourceSection(
  workflowCore,
  'const setupWorkflowEvents = async',
  '/**\n     * Select workflow with session isolation'
)
assert.match(workflowEventSetup, /setupRevision !== workflowEventSetupRevision/)
assert.match(workflowEventSetup, /unlisten\(\)[\s\S]*return false/)
assert.match(
  workflowEventSetup,
  /if \(isTerminalState\) \{[\s\S]*clearRetryTimer\(\)[\s\S]*workflowStore\.setNotification\('', 'info'\)/
)
const retryStatusHandler = sourceSection(
  workflowChat,
  'const setRetryStatus = (payload) => {',
  '// Handle chunk for streaming'
)
assert.ok(
  retryStatusHandler.indexOf('clearRetryTimer()') <
    retryStatusHandler.indexOf('chatState.value.retryInfo = {'),
  'retry timer cleanup must happen before installing the next structured retry state'
)
const stopWorkflow = sourceSection(workflowCore, 'const onStop = async', 'const openModelSelector')
assert.match(stopWorkflow, /const sessionId = currentWorkflowId\.value/)
assert.match(stopWorkflow, /invokeWrapper\('workflow_stop', \{[\s\S]*sessionId/)
assert.match(stopWorkflow, /currentWorkflowId\.value === sessionId/)
assert.match(workflowStore, /invokeWrapper\('get_earlier_workflow_message_page'/)
assert.match(
  workflowStore,
  /hiddenEarlierMessageCount\.value = Number\(snapshot\.hiddenEarlierMessageCount\) \|\| 0/
)
assert.match(
  workflowMessages,
  /workflowStore\.hiddenEarlierMessageCount[\s\S]*loadedHiddenEarlierMessageCount\.value/
)
const clearContextEligibility = sourceSection(
  workflowStore,
  'const canClearContext = computed(() => {',
  'const canStop = computed(() => {'
)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.CANCELLED/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.COMPLETED/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.ERROR/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.FAILED/)
assert.match(
  clearContextEligibility,
  /workflowState !== WORKFLOW_STATUSES\.PENDING \|\| hasLiveSession\.value/
)
assert.ok(
  clearContextEligibility.indexOf('if (stoppedStates.includes(workflowState)) return true') <
    clearContextEligibility.indexOf('const executionState'),
  'terminal workflow status must override stale non-terminal execution context state'
)
const clearContextProjection = sourceSection(
  workflowView,
  'const onClearContextFrame = async () => {',
  '// Wrapper for skill select that properly handles send'
)
assert.match(clearContextProjection, /const sessionId = currentWorkflowId\.value/)
assert.doesNotMatch(
  clearContextProjection,
  /sessionId:\s*currentWorkflowId\.value/,
  'clear-context commands must retain the workflow selected when the action began'
)
assert.match(
  clearContextProjection,
  /await workflowStore\.updateWorkflowStatus\([\s\S]*result\?\.state \|\| WORKFLOW_STATUSES\.PENDING[\s\S]*result\?\.waitReason/
)
assert.match(
  clearContextProjection,
  /workflowStore\.setHasLiveSession\(result\?\.hasLiveSession === true\)/
)
assert.ok(
  clearContextProjection.indexOf('await workflowStore.updateWorkflowStatus(') <
    clearContextProjection.indexOf('if (result?.noop)'),
  'clear-context noop recovery must reconcile backend lifecycle state before returning'
)
assert.match(
  clearContextProjection,
  /if \(result\?\.noop\) \{[\s\S]*await refreshCurrentWorkflowUiConfig\(sessionId\)/,
  'clear-context noop recovery must refresh the synchronized workflow Agent configuration'
)
assert.match(
  clearContextProjection,
  /await workflowStore\.loadMessages\(sessionId\)[\s\S]*currentWorkflowId\.value !== sessionId[\s\S]*await refreshCurrentWorkflowUiConfig\(sessionId\)/,
  'clear-context success must refresh the synchronized Agent configuration only for the selected workflow'
)
assert.doesNotMatch(
  clearContextProjection,
  /visibleTaskGroupCount|hiddenCompletedTaskGroupCount/,
  'clear-context success must not reset or reveal task-segment pagination state'
)

const approvalDialog = readProjectFile('src/components/workflow/ApprovalDialog.vue')
const statusNotifier = readProjectFile('src/components/workflow/StatusNotifier.vue')
assert.match(approvalDialog, /toolName: String/)
assert.match(approvalDialog, /v-if="isPlanApproval" class="plan-details-actions"/)
assert.match(approvalDialog, /const copyPlanMarkdown = async/)
assert.match(approvalDialog, /await writeClipboard\(planMarkdown\.value\)/)
assert.match(approvalDialog, /<cs name="copy" \/>/)
assert.match(approvalDialog, /workflow\.approval\.executionBackends\.host/)
assert.match(approvalDialog, /workflow\.approval\.fallbackReasons\.host_only_mode/)
const zhHansLocale = JSON.parse(readProjectFile('src/i18n/locales/zh-Hans.json'))
assert.equal(zhHansLocale.workflow.approval.executionBackend, '执行后端')
assert.equal(zhHansLocale.workflow.approval.executionBackends.host, '本机')
assert.equal(zhHansLocale.workflow.approval.fallbackReason, '回退原因')
assert.equal(
  zhHansLocale.workflow.approval.fallbackReasons.host_only_mode,
  '仅主机模式'
)
assert.equal(
  zhHansLocale.workflow.approval.fallbackReasons.sandbox_config_missing,
  '缺少沙箱配置'
)
assert.doesNotMatch(
  approvalDialog,
  /action: String|props\.action|normalizedAction|isFileChangePayload/
)
assert.match(messageList, /copySubAgentTask\(message\)/)
assert.match(messageList, /copySubAgentResult\(message\)/)
assert.match(messageList, /message\?\.subAgentCard\?\.task/)
assert.match(messageList, /message\?\.subAgentCard\?\.result/)
assert.match(statusNotifier, /TERMINAL_STATUSES\.includes\(workflowStatus\.value\)/)
assert.match(statusNotifier, /t\('workflow\.retrying', \{/)
assert.match(statusNotifier, /hasActiveRetry\.value/)
assert.match(statusNotifier, /const getPreviewSegment = text =>/)
assert.match(statusNotifier, /const latestTextActivity = computed\(\(\) => \{/)
assert.match(statusNotifier, /sort\(\(left, right\) => getToolTimestamp\(right\) - getToolTimestamp\(left\)\)/)
assert.match(statusNotifier, /!textIsLatestActivity/)
assert.match(statusNotifier, /elapsedNow\.value - latestToolUpdatedAt < 5000/)
assert.match(statusNotifier, /workflow\.statusNotifier\.runningElapsed/)
assert.match(statusNotifier, /workflowStore\.isRunning \|\| latestToolState\.value\?\.status === 'approved_running'/)
assert.match(statusNotifier, /setInterval\(\(\) => \{/)
assert.match(workflowStore, /const startedAt =[\s\S]*newStatus === 'approved_running'/)

const deleteWorkflow = sourceSection(
  workflowCore,
  'const onDeleteWorkflow',
  'const createNewWorkflow'
)
assert.match(deleteWorkflow, /await invokeWrapper\('delete_workflow', \{ sessionId: id \}\)/)
assert.match(deleteWorkflow, /clearPendingApprovalEntries\(id\)/)
assert.match(deleteWorkflow, /backgroundStateListeners\.delete\(id\)/)
assert.match(deleteWorkflow, /setWorkflowDeleting\(id, true\)/)
assert.match(deleteWorkflow, /backendDeleteCompleted = true/)
assert.match(deleteWorkflow, /removeWorkflowInputDraft\(id\)/)
assert.match(deleteWorkflow, /if \(!backendDeleteCompleted\)/)
assert.ok(
  deleteWorkflow.indexOf('setWorkflowDeleting(id, true)') <
    deleteWorkflow.indexOf("await invokeWrapper('delete_workflow'"),
  'deleting a workflow must block late approval events before invoking backend deletion'
)
assert.ok(
  deleteWorkflow.indexOf("await invokeWrapper('delete_workflow'") <
    deleteWorkflow.indexOf('removeWorkflowInputDraft(id)'),
  'deleting a workflow must remove input draft only after backend deletion succeeds'
)
assert.ok(
  deleteWorkflow.indexOf('clearPendingApprovalEntries(id)') <
    deleteWorkflow.indexOf('await workflowStore.loadWorkflows()'),
  'deleting a workflow must clear its background reminder cache before refreshing the sidebar'
)
assert.match(workflowCore, /if \(isWorkflowBeingDeleted\(sessionId\)\) return/)
assert.match(workflowView, /existingWorkflowIds\.has\(entry\?\.sessionId\)/)
assert.match(workflowView, /!isWorkflowBeingDeleted\(entry\?\.sessionId\)/)

const classification = readProjectFile('src/composables/workflow/toolClassification.js')
assert.doesNotMatch(classification, /startsWith\s*\(/)

const rustConstants = readProjectFile('src-tauri/src/tools/constants.rs')
const rustToolName = constantName => {
  const match = rustConstants.match(
    new RegExp(`pub const ${constantName}: &str = "([^"]+)";`)
  )
  assert.ok(match, `missing Rust tool constant: ${constantName}`)
  return match[1]
}

assert.deepEqual(WORKFLOW_TODO_TOOL_NAMES, [
  rustToolName('TOOL_TODO_CREATE'),
  rustToolName('TOOL_TODO_LIST'),
  rustToolName('TOOL_TODO_UPDATE'),
  rustToolName('TOOL_TODO_GET')
])
assert.deepEqual(WORKFLOW_SUB_AGENT_TOOL_NAMES, [
  rustToolName('TOOL_SUB_AGENT_RUN'),
  rustToolName('TOOL_SUB_AGENT_OUTPUT'),
  rustToolName('TOOL_SUB_AGENT_STOP')
])

console.log('workflow constitution tests passed')
