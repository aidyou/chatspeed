<template>
  <div class="messages" ref="messagesRef" @scroll.passive="handleScroll">
    <div
      v-for="(message, index) in visibleMessages"
      :key="message.displayId"
      class="message"
      :data-message-id="message.displayId || message.id || null"
      :data-child-task-id="getMessageSubAgentId(message)"
      :class="[message.role, message.stepType?.toLowerCase(), { 'is-error': message.isError }]">
      <div class="avatar" v-if="message.role === 'user'">
        <cs name="talk" class="user-icon" />
      </div>
        <div class="content-container">
          <div class="content" v-if="message.role === 'user'">
            <div v-if="getAskUserResponseItems(message).length > 0" class="ask-user-response-card">
            <div class="ask-user-response-title">{{ $t('workflow.askUser.responseTitle') }}</div>
            <div
              v-for="(item, itemIndex) in getAskUserResponseItems(message)"
              :key="`${item.title}-${itemIndex}`"
              class="ask-user-response-item">
              <div class="ask-user-response-question">{{ item.title }}</div>
              <div class="ask-user-response-answer">
                <span class="answer-label">{{ $t('workflow.askUser.answerLabel') }}</span>
                <span>{{ formatAskUserAnswer(item) }}</span>
              </div>
              <pre
                v-if="item.source === 'custom' && item.choice"
                class="ask-user-response-custom"
                >{{ item.choice }}</pre
              >
            </div>
          </div>
          <pre
            v-else
            class="simple-text"
            :class="{
              'is-collapsed': isExpandableUserMessage(message) && !isUserMessageExpanded(message),
              'is-expandable': isExpandableUserMessage(message)
            }"
            @click="
              isExpandableUserMessage(message) &&
                $emit('toggle-expand', getUserMessageExpandId(message))
            "
            >{{ getVisibleUserContent(message) }}</pre
          >
        </div>
        <div v-else class="ai-content chat">
          <div v-if="isExplorationBatchMessage(message)" class="exploration-card">
            <div
              class="exploration-card__header"
              @click="$emit('toggle-expand', message.displayId)">
              <div class="exploration-card__title-wrap">
                <div class="exploration-card__title">
                  <cs name="search" size="15px" class="exploration-card__icon" />
                  <span>{{ $t('workflow.exploration.title') }}</span>
                </div>
                <div class="exploration-card__meta">
                  <span>{{ getExplorationBatchSummary(message) }}</span>
                </div>
              </div>
              <span v-if="!isMessageExpanded(message)" class="exploration-card__preview">
                {{ getExplorationBatchPreview(message) }}
              </span>
              <cs
                name="double-arrow-down"
                size="14px"
                class="exploration-card__chevron"
                :class="{ expanded: isMessageExpanded(message) }" />
            </div>

            <div v-if="isMessageExpanded(message)" class="exploration-card__body">
              <div
                v-for="(group, groupIndex) in message.explorationBatch.groups"
                :key="`${message.displayId}_group_${groupIndex}`"
                class="exploration-card__step-card">
                <template v-if="group.thought">
                  <div class="reasoning-container exploration-card__reasoning">
                    <div
                      class="reasoning-header"
                      @click="
                        $emit(
                          'toggle-reasoning',
                          getExplorationGroupReasoningId(message, groupIndex)
                        )
                      ">
                      <cs name="reasoning" size="14px" class="reasoning-icon" />
                      <span
                        class="reasoning-text"
                        :class="{
                          expanded: isExplorationGroupReasoningExpanded(message, groupIndex)
                        }">
                        {{
                          isExplorationGroupReasoningExpanded(message, groupIndex)
                            ? $t('workflow.thinkingExpanded') || 'Thinking Process'
                            : $t('workflow.thoughtCompleted') || 'Thought Complete'
                        }}
                      </span>
                      <span class="reasoning-toggle">
                        {{ isExplorationGroupReasoningExpanded(message, groupIndex) ? '▲' : '▼' }}
                      </span>
                    </div>
                    <div
                      v-if="isExplorationGroupReasoningExpanded(message, groupIndex)"
                      class="reasoning-content">
                      {{ group.thought }}
                    </div>
                  </div>
                </template>

                <div
                  v-for="(tool, toolIndex) in group.tools"
                  :key="`${message.displayId}_group_${groupIndex}_tool_${toolIndex}`"
                  class="cli-tool-call exploration-card__tool"
                  :class="[tool.toolType || 'tool-system']">
                  <div
                    class="tool-line title-wrap expandable"
                    @click="$emit('toggle-expand', getExplorationToolExpandId(message, groupIndex, toolIndex))">
                    <cs :name="tool.icon || 'tool'" size="14px" class="tool-type-icon" />
                    <span class="tool-name">{{ tool.action }}</span>
                    <span class="tool-target">{{ tool.target }}</span>
                  </div>
                  <div
                    v-if="tool.summary && !isExplorationToolExpanded(message, groupIndex, toolIndex)"
                    class="tool-line summary expandable"
                    @click="$emit('toggle-expand', getExplorationToolExpandId(message, groupIndex, toolIndex))">
                    <span class="corner-icon">⎿</span>
                    <span class="summary-text">{{ tool.summary }}</span>
                    <span class="expand-hint">(click to expand)</span>
                  </div>
                  <div v-if="isExplorationToolExpanded(message, groupIndex, toolIndex)" class="tool-detail">
                    <MarkdownSimple
                      v-if="
                        shouldShowExplorationToolRawContent(tool) &&
                        tool.displayType === 'diff'
                      "
                      :content="getDiffMarkdown(removeSystemReminder(tool.message))" />
                    <MarkdownSimple
                      v-else-if="
                        shouldShowExplorationToolRawContent(tool) &&
                        tool.displayType === 'markdown'
                      "
                      :content="removeSystemReminder(tool.message)" />
                    <pre
                      v-else-if="shouldShowExplorationToolRawContent(tool)"
                      class="raw-content"
                      >{{ removeSystemReminder(tool.message) }}</pre
                    >
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- CLI Style Tool Call (Results) -->
          <div
            v-else-if="message.role === 'tool'"
            class="cli-tool-call"
            :class="[
              message.toolDisplay.toolType || 'tool-system',
              message.toolDisplay.isError ? 'status-error' : 'status-success'
            ]">
            <template v-if="isSubAgentRunMessage(message) && message.subAgentCard">
              <div class="sub-agent-card">
                <div class="sub-agent-card__header">
                  <div class="sub-agent-card__title-wrap">
                    <div class="sub-agent-card__title">
                      <cs name="task" size="15px" class="sub-agent-card__icon" />
                      <span>Delegated Task</span>
                    </div>
                    <div class="sub-agent-card__status" :class="subAgentStatusClass(message)">
                      <cs
                        name="loading"
                        class="cs-spin"
                        v-if="getSubAgentStatusLabel(message) === 'Running'" />
                      {{ getSubAgentStatusLabel(message) }}
                    </div>
                  </div>
                  <div class="sub-agent-card__meta">
                    <div class="sub-agent-card__row">
                      <span class="sub-agent-card__label">Agent</span>
                      <span class="sub-agent-card__value">{{ message.subAgentCard.agent }}</span>
                    </div>
                    <div class="sub-agent-card__row">
                      <span class="sub-agent-card__label">Mode</span>
                      <span class="sub-agent-card__value mode">
                        {{ message.subAgentCard.mode }}
                      </span>
                    </div>
                    <div class="sub-agent-card__row">
                      <span class="sub-agent-card__label">Tools</span>
                      <span class="sub-agent-card__value">{{ getSubAgentLiveTools(message) }}</span>
                    </div>
                    <div class="sub-agent-card__row">
                      <span class="sub-agent-card__label">Context</span>
                      <span class="sub-agent-card__value">{{
                        getSubAgentLiveContext(message)
                      }}</span>
                    </div>
                  </div>
                </div>

                <div
                  class="sub-agent-card__task"
                  :class="{ expanded: isSubAgentTaskExpanded(message) }">
                  <div
                    class="sub-agent-card__task-toggle"
                    @click="$emit('toggle-expand', getSubAgentTaskExpandId(message))">
                    <div class="sub-agent-card__task-heading">
                      <span class="sub-agent-card__label">Task</span>
                      <span
                        v-if="!isSubAgentTaskExpanded(message)"
                        class="sub-agent-card__task-preview"
                        >{{ getSubAgentTaskPreview(message) }}</span
                      >
                    </div>
                    <cs
                      name="double-arrow-down"
                      size="14px"
                      class="sub-agent-card__task-chevron"
                      :class="{ expanded: isSubAgentTaskExpanded(message) }" />
                  </div>
                  <div v-if="isSubAgentTaskExpanded(message)" class="sub-agent-card__task-body">
                    <MarkdownSimple :content="message.subAgentCard.taskMarkdown" />
                  </div>
                </div>

                <div
                  v-if="message.subAgentCard.hasResult"
                  class="sub-agent-card__result"
                  :class="{ expanded: isMessageExpanded(message) }">
                  <div
                    class="sub-agent-card__result-toggle"
                    @click="$emit('toggle-expand', message.displayId)">
                    <div class="sub-agent-card__result-heading">
                      <span class="sub-agent-card__label">Result</span>
                      <span
                        v-if="!isMessageExpanded(message)"
                        class="sub-agent-card__result-preview"
                        >{{ getSubAgentResultPreview(message) }}</span
                      >
                    </div>
                    <cs
                      name="double-arrow-down"
                      size="14px"
                      class="sub-agent-card__result-chevron"
                      :class="{ expanded: isMessageExpanded(message) }" />
                  </div>
                  <div v-if="isMessageExpanded(message)" class="sub-agent-card__result-body">
                    <MarkdownSimple :content="message.subAgentCard.resultMarkdown" />
                  </div>
                </div>
              </div>
            </template>

            <!-- complete_workflow_with_summary special display -->
            <template v-else-if="isFinishTaskMessage(message)">
              <div class="tool-line finish-task-display">
                <cs
                  :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'"
                  size="14px"
                  class="tool-type-icon finish-icon" />
                <span class="finish-text">
                  {{ getFinishTaskLabel(message) }}
                </span>
              </div>
            </template>

            <!-- Normal tool call display -->
            <template v-else>
              <div
                class="tool-line title-wrap expandable"
                :class="{ 'tool-rejected': message.isRejected }"
                @click="$emit('toggle-expand', message.displayId)">
                <cs :name="message.toolDisplay.icon || 'tool'" size="15px" class="tool-type-icon" />
                <span class="tool-name">{{ message.toolDisplay.action }}</span>
                <span class="tool-target">{{ message.toolDisplay.target }}</span>
                <cs v-if="message.isApproved" name="check" size="14px" class="approved-icon" />
              </div>
              <!-- Hide summary when expanded -->
              <div
                class="tool-line summary expandable"
                v-if="!isMessageExpanded(message)"
                @click="$emit('toggle-expand', message.displayId)">
                <span class="corner-icon">⎿</span>
                <span class="summary-text">{{ message.toolDisplay.summary }}</span>
                <span class="expand-hint">(click to expand)</span>
              </div>
              <div v-if="isMessageExpanded(message)" class="tool-detail">
                <!-- Tool Stream Output (for bash commands) -->
                <div
                  v-if="
                    message.metadata?.tool_call_id &&
                    workflowStore.getToolStream(message.metadata.tool_call_id).length > 0
                  "
                  class="tool-stream-output">
                  <div
                    v-for="(line, idx) in workflowStore.getToolStream(
                      message.metadata.tool_call_id
                    )"
                    :key="idx"
                    class="stream-line">
                    {{ line }}
                  </div>
                </div>
                <!-- Final Result -->
                <MarkdownSimple
                  v-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'diff'
                  "
                  :content="getDiffMarkdown(removeSystemReminder(message.message))" />
                <div
                  v-else-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'choice'
                  "
                  class="choice-container">
                  <div
                    v-for="group in getChoiceGroups(message)"
                    :key="group.title"
                    class="choice-group">
                    <div class="choice-question">
                      {{ group.title }}
                    </div>
                    <el-radio-group
                      :model-value="getAskUserSelection(message, group.title)"
                      class="choice-options vertical numbered"
                      @update:model-value="
                        value => setAskUserSelection(message, group.title, value)
                      ">
                      <el-radio
                        v-for="(opt, optIndex) in group.options"
                        :key="`${group.title}-${opt}`"
                        :value="opt"
                        :disabled="!canAnswerAskUser(message) || askUserSubmitting">
                        <span class="choice-option-label">{{ optIndex + 1 }}. {{ opt }}</span>
                      </el-radio>
                      <div class="choice-custom-row">
                        <el-radio
                          :value="CUSTOM_ASK_USER_VALUE"
                          :disabled="!canAnswerAskUser(message) || askUserSubmitting">
                          <span class="choice-option-label">{{ group.options.length + 1 }}.</span>
                        </el-radio>
                        <el-input
                          :model-value="getAskUserCustomInput(message, group.title)"
                          class="choice-custom-input"
                          type="textarea"
                          :autosize="{ minRows: 1, maxRows: 6 }"
                          :placeholder="$t('workflow.askUser.customPlaceholder')"
                          :disabled="!canAnswerAskUser(message) || askUserSubmitting"
                          @focus="setAskUserSelection(message, group.title, CUSTOM_ASK_USER_VALUE)"
                          @update:model-value="
                            value => setAskUserCustomInput(message, group.title, value)
                          " />
                      </div>
                    </el-radio-group>
                  </div>
                  <div v-if="canAnswerAskUser(message)" class="choice-submit-row">
                    <el-button
                      size="small"
                      type="primary"
                      :loading="askUserSubmitting"
                      @click="submitAskUserResponse(message)">
                      {{ $t('workflow.askUser.submit') }}
                    </el-button>
                  </div>
                </div>
                <MarkdownSimple
                  v-else-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'markdown'
                  "
                  :content="removeSystemReminder(message.message)" />
                <pre
                  v-else-if="
                    message.metadata?.approval_status !== 'pending' &&
                    shouldShowToolRawContent(message)
                  "
                  class="raw-content"
                  >{{ removeSystemReminder(message.message) }}</pre
                >
                <ApprovalDialog
                  v-if="
                    message.metadata?.approval_status === 'pending' &&
                    !isApprovalSubmitting(currentWorkflowId, message.metadata?.tool_call_id)
                  "
                  inline
                  :action="message.metadata?.tool_name || message.toolDisplay.action"
                  :target="message.toolDisplay.target"
                  :details="getApprovalDetailsPayload(message)"
                  :display-type="message.metadata?.display_type || message.toolDisplay.displayType"
                  :rejection-message="getApprovalDraft(message.metadata?.tool_call_id)"
                  :loading="approvalLoading && activeApprovalId === message.metadata?.tool_call_id"
                  :pending-count="inlinePendingApprovalCount"
                  @update:rejection-message="
                    value => setApprovalDraft(message.metadata?.tool_call_id, value)
                  "
                  @approve="$emit('approve-tool', message.metadata?.tool_call_id)"
                  @approve-all="$emit('approve-all-tool', message.metadata?.tool_call_id)"
                  @approve-all-pending="onApproveAllPending"
                  @reject="
                    $emit(
                      'reject-tool',
                      message.metadata?.tool_call_id,
                      getApprovalDraft(message.metadata?.tool_call_id)
                    )
                  " />
              </div>
            </template>
          </div>

          <!-- Regular Assistant Content -->
          <div v-else>
            <div v-if="isContextSnapshotMessage(message)" class="context-snapshot-card">
              <div
                class="context-snapshot-card__header"
                @click="$emit('toggle-expand', getContextSnapshotExpandId(message))">
                <cs name="archive" size="14px" class="context-snapshot-card__icon" />
                <span class="context-snapshot-card__title">Previous Context Snapshot</span>
                <span
                  v-if="!isContextSnapshotExpanded(message)"
                  class="context-snapshot-card__preview">
                  {{ getContextSnapshotPreview(message) }}
                </span>
                <cs
                  name="double-arrow-down"
                  size="14px"
                  class="context-snapshot-card__chevron"
                  :class="{ expanded: isContextSnapshotExpanded(message) }" />
              </div>
              <div v-if="isContextSnapshotExpanded(message)" class="context-snapshot-card__body">
                <MarkdownSimple :content="formatContextSnapshotForDisplay(message)" />
              </div>
            </div>

            <!-- Thought/Content FIRST (Separate reasoning field has priority) -->
            <div
              v-else-if="message.reasoning || message.stepType === 'Think'"
              class="reasoning-container">
              <div class="reasoning-header" @click="$emit('toggle-reasoning', message.displayId)">
                <cs
                  name="reasoning"
                  size="14px"
                  class="reasoning-icon"
                  :class="{
                    rotating:
                      isRunning &&
                      !hasThoughtCompleted(message) &&
                      !isReasoningExpanded(message.displayId) &&
                      message === lastAssistantMessage
                  }" />
                <span
                  class="reasoning-text"
                  :class="{ expanded: isReasoningExpanded(message.displayId) }">
                  <template v-if="isReasoningExpanded(message.displayId)">
                    {{ $t('workflow.thinkingExpanded') || 'Thinking Process' }}
                  </template>
                  <template
                    v-else-if="
                      isRunning && !hasThoughtCompleted(message) && message === lastAssistantMessage
                    ">
                    {{ getReasoningPreview(message.reasoning || message.message) }}
                  </template>
                  <template v-else>
                    {{ $t('workflow.thoughtCompleted') || 'Thought Complete' }}
                  </template>
                </span>
                <span class="reasoning-toggle">
                  {{ isReasoningExpanded(message.displayId) ? '▲' : '▼' }}
                </span>
              </div>
              <div v-if="isReasoningExpanded(message.displayId)" class="reasoning-content">
                {{ message.reasoning || message.message }}
              </div>
            </div>
            <MarkdownSimple
              v-if="!isContextSnapshotMessage(message) && getParsedMessage(message).content"
              :content="getParsedMessage(message).content" />

            <!-- Tool Call Indicators SECOND (Only pending ones) -->
            <div v-if="message.pendingToolCalls?.length > 0" class="cli-tool-calls-container">
              <div
                v-for="call in message.pendingToolCalls"
                :key="call.id"
                class="cli-tool-call pending"
                :class="[
                  call.toolType || 'tool-system',
                  call.isRejected ? 'status-error' : 'status-running'
                ]">
                <div class="tool-line title-wrap" :class="{ 'tool-rejected': call.isRejected }">
                  <cs :name="call.icon || 'tool'" size="14px" class="tool-type-icon" />
                  <span class="tool-name">{{ call.action }}</span>
                  <span class="tool-target">{{ call.target }}</span>
                </div>
                <div class="tool-line summary">
                  <span class="corner-icon">⎿</span>
                  <span class="summary-text">{{ call.summary }}</span>
                </div>
                <div
                  v-if="
                    call.toolName === 'complete_workflow_with_summary' && call.completionSummary
                  "
                  class="finish-task-summary markdown-body">
                  <MarkdownSimple :content="call.completionSummary" />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Streaming Chat State -->
    <div
      v-if="isChatting && (chatState.content || chatState.reasoning)"
      class="message assistant chatting">
      <div class="content-container">
        <div class="ai-content chat">
          <div v-if="chatState.reasoning" class="reasoning-container">
            <div
              class="reasoning-header"
              :class="{ clickable: hasStreamingThoughtCompleted }"
              @click="
                hasStreamingThoughtCompleted
                  ? $emit('toggle-reasoning', STREAMING_REASONING_DISPLAY_ID)
                  : null
              ">
              <cs
                name="reasoning"
                size="14px"
                class="reasoning-icon"
                :class="{ rotating: !hasStreamingThoughtCompleted }" />
              <span class="reasoning-text">
                {{
                  hasStreamingThoughtCompleted
                    ? $t('workflow.thoughtCompleted') || 'Thought Complete'
                    : getReasoningPreview(chatState.reasoning)
                }}
              </span>
              <span v-if="hasStreamingThoughtCompleted" class="reasoning-toggle">
                {{ isReasoningExpanded(STREAMING_REASONING_DISPLAY_ID) ? '▲' : '▼' }}
              </span>
            </div>
            <div
              v-if="hasStreamingThoughtCompleted && isReasoningExpanded(STREAMING_REASONING_DISPLAY_ID)"
              class="reasoning-content">
              {{ chatState.reasoning }}
            </div>
          </div>
          <!-- Streaming Blocks (Optimized rendering) -->
          <div v-for="(block, bIdx) in chatState.blocks" :key="bIdx">
            <!-- Output all blocks from the parser (paragraph, code, math, etc.) -->
            <MarkdownSimple :content="block.content" />
          </div>

          <!-- Retry Countdown... -->
          <div
            v-if="chatState.retryInfo && chatState.retryInfo.nextRetryIn > 0"
            class="retry-status-alert">
            <el-alert type="warning" :closable="false" show-icon>
              <template #title>
                {{
                  $t('workflow.retrying', {
                    attempt: chatState.retryInfo.attempt,
                    total: chatState.retryInfo.total,
                    seconds: chatState.retryInfo.nextRetryIn
                  })
                }}
              </template>
            </el-alert>
          </div>
        </div>
      </div>
    </div>

    <!-- Context Compression Status -->
    <div v-if="isCompressing" class="compression-status">
      <div class="compression-indicator">
        <cs name="loading" size="14px" class="rotating" />
        <span class="compression-text">{{ compressionMessage }}</span>
      </div>
    </div>

    <!-- Frontend queued user messages -->
    <div v-if="queuedMessages.length > 0" class="queued-list">
      <div v-for="item in queuedMessages" :key="item.id" class="queued-item">
        <cs name="clock" size="12px" class="queued-icon" />
        <span class="queued-text">{{ item.content }}</span>
        <button
          type="button"
          class="queued-remove"
          @click="$emit('remove-queued-message', item.id)">
          <cs name="close" size="12px" />
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { showMessage } from '@/libs/util'
import ApprovalDialog from './ApprovalDialog.vue'
import MarkdownSimple from './MarkdownSimple.vue'
import { useWorkflowStore } from '@/stores/workflow'
import { WORKFLOW_STATUSES, WORKFLOW_WAIT_REASONS } from '@/composables/workflow/signalTypes'

const workflowStore = useWorkflowStore()
const { t } = useI18n()
const CUSTOM_ASK_USER_VALUE = '__custom__'
const STREAMING_REASONING_DISPLAY_ID = '__streaming_reasoning__'

const props = defineProps({
  messages: {
    type: Array,
    default: () => []
  },
  queuedMessages: {
    type: Array,
    default: () => []
  },
  isRunning: {
    type: Boolean,
    default: false
  },
  isChatting: {
    type: Boolean,
    default: false
  },
  chatState: {
    type: Object,
    default: () => ({
      content: '',
      reasoning: '',
      blocks: [],
      retryInfo: null
    })
  },
  isCompressing: {
    type: Boolean,
    default: false
  },
  compressionMessage: {
    type: String,
    default: ''
  },
  lastAssistantMessage: {
    type: Object,
    default: null
  },
  approvalLoading: {
    type: Boolean,
    default: false
  },
  activeApprovalId: {
    type: String,
    default: ''
  },
  currentWorkflowId: {
    type: String,
    default: ''
  },
  isApprovalSubmitting: {
    type: Function,
    default: () => false
  },
  isMessageExpanded: {
    type: Function,
    required: true
  },
  isReasoningExpanded: {
    type: Function,
    required: true
  },
  removeSystemReminder: {
    type: Function,
    required: true
  },
  getDiffMarkdown: {
    type: Function,
    required: true
  },
  parseChoiceContent: {
    type: Function,
    required: true
  },
  getParsedMessage: {
    type: Function,
    required: true
  },
  shouldShowToolRawContent: {
    type: Function,
    required: true
  },
  getReasoningPreview: {
    type: Function,
    required: true
  },
  askUserSubmitting: {
    type: Boolean,
    default: false
  },
  pendingCount: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits([
  'toggle-expand',
  'toggle-reasoning',
  'scroll-bottom',
  'approve-tool',
  'approve-all-tool',
  'approve-all-pending',
  'reject-tool',
  'submit-ask-user',
  'remove-queued-message'
])

const messagesRef = ref(null)
const approvalDrafts = ref({})
const askUserDrafts = ref({})
const AUTO_SCROLL_THRESHOLD = 64
const shouldAutoScroll = ref(true)

const inlinePendingApprovalCount = computed(() => {
  const toolIds = new Set()
  for (const message of props.messages || []) {
    if (message?.role !== 'tool') continue
    const toolCallId = message?.metadata?.tool_call_id
    if (!toolCallId) continue
    if (message?.metadata?.approval_status !== 'pending') continue
    toolIds.add(toolCallId)
  }
  return toolIds.size
})

const isNearBottom = el => {
  if (!el) return true
  return el.scrollHeight - el.scrollTop - el.clientHeight <= AUTO_SCROLL_THRESHOLD
}

const handleScroll = () => {
  shouldAutoScroll.value = isNearBottom(messagesRef.value)
}

const isHiddenSystemObservation = message => {
  const uiVisibility = message?.metadata?.ui_visibility || message?.metadata?.uiVisibility
  if (uiVisibility === 'hide') return true
  if (
    message?.metadata?.message_kind === 'runtime_observation' ||
    message?.metadata?.messageKind === 'runtime_observation'
  ) {
    return false
  }
  if (message?.metadata?.error_type === 'SubAgentInterrupted') return true
  if (message?.metadata?.errorType === 'SubAgentInterrupted') return true
  if (message?.role !== 'user') return false
  if ((message.stepType || '').toLowerCase() !== 'observe') return false
  if (getAskUserResponseItems(message).length > 0) return false
  return props.removeSystemReminder(message.message || '').trim() === ''
}

const isContextSnapshotMessage = message =>
  message?.role === 'system' && message?.metadata?.type === 'summary'

const getContextSnapshotContent = message => {
  const content = props.removeSystemReminder(message?.message || '')
  const normalized = content.replace(/^##\s*Previous Context Snapshot\s*/i, '').trim()

  try {
    const parsed = JSON.parse(normalized)
    if (typeof parsed?.content === 'string' && parsed.content.trim()) {
      return parsed.content.trim()
    }
  } catch {
    // Fall back to raw content when the snapshot is already plain text/XML.
  }

  return normalized
}

const xmlNodeText = (parent, tagName) => {
  const node = parent?.getElementsByTagName?.(tagName)?.[0]
  return node?.textContent?.trim() || ''
}

const formatContextSnapshotForDisplay = message => {
  const content = getContextSnapshotContent(message)
  if (!content || !content.includes('<state_snapshot')) return content

  try {
    const parser = new DOMParser()
    const doc = parser.parseFromString(content, 'application/xml')
    if (doc.querySelector('parsererror')) return content

    const root = doc.getElementsByTagName('state_snapshot')[0]
    if (!root) return content

    const sections = [
      ['Overall Goal', xmlNodeText(root, 'overall_goal')],
      ['Key Knowledge', xmlNodeText(root, 'key_knowledge')],
      ['Error Log', xmlNodeText(root, 'error_log')],
      ['File System State', xmlNodeText(root, 'file_system_state')],
      ['Recent Actions', xmlNodeText(root, 'recent_actions')],
      ['Task State', xmlNodeText(root, 'task_state')]
    ].filter(([, value]) => value)

    return sections
      .map(([title, value]) => `### ${title}\n\n${value}`)
      .join('\n\n')
      .trim()
  } catch {
    return content
  }
}

const getContextSnapshotExpandId = message =>
  `${message?.displayId || message?.id || 'snapshot'}:snapshot`

const isContextSnapshotExpanded = message =>
  props.isMessageExpanded({
    displayId: getContextSnapshotExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const getContextSnapshotPreview = message => {
  const content = formatContextSnapshotForDisplay(message)
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()

  if (!content) return ''
  return content.length > 96 ? `${content.slice(0, 96)}...` : content
}

const getMessageToolName = message => {
  return String(
    message?.metadata?.tool_name ||
      message?.metadata?.tool_call?.name ||
      message?.metadata?.tool_call?.function?.name ||
      ''
  ).toLowerCase()
}

const getToolCallArguments = message => {
  const rawArgs =
    message?.metadata?.tool_call?.function?.arguments ?? message?.metadata?.tool_call?.arguments
  if (!rawArgs) return null
  if (typeof rawArgs === 'string') {
    try {
      return JSON.parse(rawArgs)
    } catch {
      return null
    }
  }
  return typeof rawArgs === 'object' ? rawArgs : null
}

const getApprovalDetailsPayload = message => {
  const structuredDetails = message?.metadata?.details
  if (structuredDetails !== undefined && structuredDetails !== null) {
    return structuredDetails
  }

  const toolName = getMessageToolName(message)
  if (['edit_file', 'write_file', 'plan_edit_note', 'plan_write_note'].includes(toolName)) {
    const args = getToolCallArguments(message)
    if (args && typeof args === 'object') {
      return args
    }
  }

  return props.removeSystemReminder(message?.message || '')
}

const isFinishTaskMessage = message => {
  const metaToolName = getMessageToolName(message)
  const action = message?.toolDisplay?.action || ''
  return (
    metaToolName === 'complete_workflow_with_summary' ||
    action === t('workflow.finishTask') ||
    action.includes('Finish')
  )
}

const isFinishTaskErrorMessage = message => {
  if (!message || message.role !== 'tool') return false
  return isFinishTaskMessage(message) && !!message.toolDisplay?.isError
}

const isSameFinishTaskError = (left, right) => {
  if (!isFinishTaskErrorMessage(left) || !isFinishTaskErrorMessage(right)) return false
  return (
    props.removeSystemReminder(left.message || '') ===
      props.removeSystemReminder(right.message || '') &&
    (left.toolDisplay?.summary || '') === (right.toolDisplay?.summary || '')
  )
}

const collapseRepeatedFinishTaskErrors = messages => {
  const collapsed = []

  for (let index = 0; index < messages.length; ) {
    const current = messages[index]

    if (!isFinishTaskErrorMessage(current)) {
      collapsed.push(current)
      index += 1
      continue
    }

    let count = 1
    let nextIndex = index + 1
    while (nextIndex < messages.length && isSameFinishTaskError(current, messages[nextIndex])) {
      count += 1
      nextIndex += 1
    }

    if (count > 1) {
      collapsed.push({
        ...current,
        displayId: `${current.displayId || current.id || `finish_task_${index}`}_collapsed_${count}`,
        metadata: {
          ...(current.metadata || {}),
          finish_task_error_count: count
        }
      })
    } else {
      collapsed.push(current)
    }

    index = nextIndex
  }

  return collapsed
}

const isCompletionReportMessage = message =>
  message?.role === 'assistant' &&
  (message?.metadata?.message_kind === 'completion_report' ||
    message?.metadata?.messageKind === 'completion_report')

const READ_ONLY_EXPLORATION_TOOLS = new Set([
  'read_file',
  'grep',
  'glob',
  'list_dir',
  'skill',
  'web_fetch',
  'web_search'
])

const READ_ONLY_BASH_SEGMENTS = new Set([
  'cd',
  'cat',
  'find',
  'grep',
  'head',
  'ls',
  'pwd',
  'readlink',
  'rg',
  'sed',
  'sort',
  'tail',
  'tree',
  'which'
])

const isThinkOnlyAssistantMessage = message => {
  if (message?.role !== 'assistant') return false
  const content = props.removeSystemReminder(message?.message || '').trim()
  const reasoning = String(message?.reasoning || '').trim()
  return !content && !!reasoning
}

const getToolArguments = message => {
  const rawArgs =
    message?.metadata?.tool_call?.function?.arguments ||
    message?.metadata?.tool_call?.arguments ||
    message?.metadata?.arguments ||
    {}

  if (typeof rawArgs === 'string') {
    try {
      return JSON.parse(rawArgs)
    } catch {
      return {}
    }
  }

  return rawArgs && typeof rawArgs === 'object' ? rawArgs : {}
}

const isReadOnlyBashCommand = command => {
  const normalized = String(command || '').trim()
  if (!normalized) return false

  const segments = normalized
    .split(/(?:&&|\|\||;|\|)/)
    .map(segment => segment.trim())
    .filter(Boolean)

  if (!segments.length) return false

  return segments.every(segment => {
    const token = segment.split(/\s+/)[0]?.toLowerCase() || ''
    return READ_ONLY_BASH_SEGMENTS.has(token)
  })
}

const isReadOnlyExplorationToolMessage = message => {
  if (message?.role !== 'tool') return false
  if (message?.metadata?.approval_status === 'pending') return false
  if (message?.metadata?.execution_status === 'running') return false
  if (message?.isRejected || message?.toolDisplay?.isError) return false
  if (isSubAgentRunMessage(message) || isFinishTaskMessage(message)) return false
  if (message?.toolDisplay?.hasStreamOutput) return false

  const toolName = getMessageToolName(message)
  if (toolName === 'bash') {
    return isReadOnlyBashCommand(getToolArguments(message)?.command)
  }
  return READ_ONLY_EXPLORATION_TOOLS.has(toolName)
}

const getExplorationToolSubject = message => {
  const toolName = getMessageToolName(message)
  const action = String(message?.toolDisplay?.action || '').trim()
  const target = String(message?.toolDisplay?.target || '').trim()
  if (target) return target
  if (toolName === 'grep') {
    const match = action.match(/\sin\s(.+)$/)
    if (match?.[1]) return match[1].trim()
  }
  if (toolName === 'skill') return target || action.replace(/^Activate Skill\s*/i, '').trim()
  if (toolName === 'web_fetch') return action.replace(/^Fetch\s+/i, '').trim()
  if (toolName === 'web_search') return action.replace(/^Search\s+/i, '').trim()
  if (toolName === 'bash') {
    return normalizeBashSubject(getToolArguments(message)?.command || action)
  }
  return action
}

const normalizeBashSubject = command => {
  const normalized = String(command || '').trim()
  if (!normalized) return ''

  const fileLikeMatch = normalized.match(/([A-Za-z0-9_./-]+\.[A-Za-z0-9_-]+)/g)
  if (fileLikeMatch?.length) {
    const lastPath = fileLikeMatch[fileLikeMatch.length - 1]
    return lastPath.split('/').filter(Boolean).pop() || lastPath
  }

  return normalized.length > 48 ? `${normalized.slice(0, 48)}...` : normalized
}

const buildExplorationBatchMessage = (messages, startIndex) => {
  const toolMessages = messages.filter(isReadOnlyExplorationToolMessage)
  const thoughtMessages = messages.filter(isThinkOnlyAssistantMessage)
  if (toolMessages.length < 2) return null

  const files = []
  for (const toolMessage of toolMessages) {
    const subject = getExplorationToolSubject(toolMessage)
    if (subject) files.push(subject)
  }
  const uniqueFiles = [...new Set(files)].slice(0, 3)
  const readCount = toolMessages.filter(
    item => getMessageToolName(item) === 'read_file' || getMessageToolName(item) === 'list_dir'
  ).length
  const searchCount = toolMessages.length - readCount
  const firstId = messages[0]?.displayId || messages[0]?.id || `exploration_${startIndex}`
  const lastId =
    messages[messages.length - 1]?.displayId || messages[messages.length - 1]?.id || firstId
  const groups = []
  let currentGroup = null

  for (const item of messages) {
    if (isThinkOnlyAssistantMessage(item)) {
      currentGroup = {
        thought: item.reasoning || item.message,
        tools: []
      }
      groups.push(currentGroup)
      continue
    }

    if (!currentGroup) {
      currentGroup = {
        thought: '',
        tools: []
      }
      groups.push(currentGroup)
    }

    currentGroup.tools.push({
      message: item?.message || '',
      metadata: item?.metadata || {},
      icon: item?.toolDisplay?.icon || 'tool',
      toolType: item?.toolDisplay?.toolType || 'tool-system',
      action: item?.toolDisplay?.action || '',
      target: item?.toolDisplay?.target || '',
      summary: item?.toolDisplay?.summary || '',
      displayType: item?.toolDisplay?.displayType || '',
      sourceMessage: item
    })
  }

  return {
    role: 'tool',
    displayId: `exploration_batch_${firstId}_${lastId}`,
    id: `exploration_batch_${startIndex}`,
    metadata: {
      message_kind: 'exploration_batch'
    },
    explorationBatch: {
      toolCount: toolMessages.length,
      thoughtCount: thoughtMessages.length,
      readCount,
      searchCount,
      files: uniqueFiles,
      groups
    }
  }
}

const collapseExplorationBatches = messages => {
  const collapsed = []

  for (let index = 0; index < messages.length; ) {
    const current = messages[index]
    const isBatchCandidate =
      isThinkOnlyAssistantMessage(current) || isReadOnlyExplorationToolMessage(current)

    if (!isBatchCandidate) {
      collapsed.push(current)
      index += 1
      continue
    }

    const batch = []
    let cursor = index
    while (cursor < messages.length) {
      const candidate = messages[cursor]
      if (!isThinkOnlyAssistantMessage(candidate) && !isReadOnlyExplorationToolMessage(candidate)) {
        break
      }
      batch.push(candidate)
      cursor += 1
    }

    if (props.isRunning && cursor >= messages.length) {
      const lastToolIndex = [...batch]
        .map((item, batchIndex) => (isReadOnlyExplorationToolMessage(item) ? batchIndex : -1))
        .filter(batchIndex => batchIndex >= 0)
        .pop()

      if (typeof lastToolIndex === 'number') {
        const stableBatch = batch.slice(0, lastToolIndex)
        const liveTail = batch.slice(lastToolIndex)

        if (stableBatch.length > 0) {
          const grouped = buildExplorationBatchMessage(stableBatch, index)
          if (grouped) collapsed.push(grouped)
          else collapsed.push(...stableBatch)
        }

        collapsed.push(...liveTail)
        index = cursor
        continue
      }
    }

    const grouped = buildExplorationBatchMessage(batch, index)
    if (grouped) collapsed.push(grouped)
    else collapsed.push(...batch)
    index = cursor
  }

  return collapsed
}

const collapseAssistantCompletionPairs = messages => {
  const collapsed = []

  for (let index = 0; index < messages.length; index += 1) {
    const current = messages[index]
    const next = messages[index + 1]

    if (
      isThinkOnlyAssistantMessage(current) &&
      isCompletionReportMessage(next) &&
      String(current.stepIndex || '') === String(next.stepIndex || '')
    ) {
      continue
    }

    collapsed.push(current)
  }

  return collapsed
}

/// Ordered FIFO queue of pending approval tool_call_ids from the source messages.
const pendingApprovalQueue = computed(() =>
  props.messages
    .filter(msg => msg.metadata?.approval_status === 'pending')
    .map(msg => msg.metadata?.tool_call_id)
)

/// Returns true when the message is the first pending approval in the FIFO queue.
const isFirstPendingApproval = message => {
  if (message.metadata?.approval_status !== 'pending') return false
  const queue = pendingApprovalQueue.value
  if (queue.length === 0) return false
  return queue[0] === message.metadata?.tool_call_id
}

const visibleMessages = computed(() =>
  collapseExplorationBatches(
    collapseAssistantCompletionPairs(
      collapseRepeatedFinishTaskErrors(
        props.messages.filter(message => !isHiddenSystemObservation(message))
      )
    )
  )
)
const lastVisibleMessage = computed(
  () => visibleMessages.value[visibleMessages.value.length - 1] || null
)
const getVisibleMessageIndex = message =>
  visibleMessages.value.findIndex(item => item.displayId === message?.displayId)

const hasSubsequentVisibleOutput = message => {
  const index = getVisibleMessageIndex(message)
  if (index === -1) return false

  return visibleMessages.value.slice(index + 1).some(item => item.role !== 'user')
}

const hasStreamingThoughtCompleted = computed(() => {
  if (props.chatState.content) return true

  const message = lastVisibleMessage.value
  if (!message || message.role === 'user') return false

  if (message.role === 'tool') return true

  return hasThoughtCompleted(message)
})

const hasThoughtCompleted = message => {
  if (!message) return false
  if (props.getParsedMessage(message).content) return true
  if ((message.metadata?.tool_calls?.length || 0) > 0) return true
  if ((message.pendingToolCalls?.length || 0) > 0) return true
  if (
    message === lastVisibleMessage.value &&
    props.isRunning &&
    !props.isChatting &&
    !!(message.reasoning || message.message)
  ) {
    return true
  }
  if (hasSubsequentVisibleOutput(message)) return true
  return false
}

const getApprovalDraft = toolCallId => {
  if (!toolCallId) return ''
  return approvalDrafts.value[toolCallId] || ''
}

const setApprovalDraft = (toolCallId, value) => {
  if (!toolCallId) return
  approvalDrafts.value = {
    ...approvalDrafts.value,
    [toolCallId]: value
  }
}

const onApproveAllPending = () => {
  emit('approve-all-pending')
}

const getChoiceKey = message =>
  message.metadata?.tool_call_id || message.displayId || message.id || ''

const getAskUserResponseItems = message => {
  const content = message?.message || ''
  const match = content.match(/<ask_user_response>\s*([\s\S]*?)\s*<\/ask_user_response>/i)
  if (!match) return []

  try {
    const parsed = JSON.parse(match[1])
    return Array.isArray(parsed) ? parsed : []
  } catch (error) {
    return []
  }
}

const formatAskUserAnswer = item => {
  if (!item) return ''
  if (item.source === 'custom') {
    return `${t('workflow.askUser.customLabel')} (${item.choice_index})`
  }
  return item.choice_index ? `${item.choice_index}. ${item.choice}` : item.choice || ''
}

const getFinishTaskLabel = message => {
  const count = Number(message?.metadata?.finish_task_error_count || 1)
  if (count > 1) return `${t('workflow.finishTask')} (${count})`
  return t('workflow.finishTask')
}

const isExplorationBatchMessage = message => message?.metadata?.message_kind === 'exploration_batch'

const getExplorationBatchSummary = message => {
  const batch = message?.explorationBatch
  if (!batch) return ''
  const parts = []
  if (batch.readCount) parts.push(t('workflow.exploration.reads', { count: batch.readCount }))
  if (batch.searchCount)
    parts.push(t('workflow.exploration.searches', { count: batch.searchCount }))
  if (batch.thoughtCount)
    parts.push(t('workflow.exploration.thoughts', { count: batch.thoughtCount }))
  return parts.join(', ')
}

const getExplorationBatchPreview = message => {
  const files = message?.explorationBatch?.files || []
  if (files.length === 0) return ''
  return files
    .map(file => {
      const normalized = String(file || '').replace(/\\/g, '/')
      const name = normalized.split('/').filter(Boolean).pop() || normalized
      return `Read ${name}`
    })
    .join(', ')
}

const getExplorationGroupReasoningId = (message, groupIndex) =>
  `${message?.displayId || message?.id || 'exploration'}:group_reasoning:${groupIndex}`

const isExplorationGroupReasoningExpanded = (message, groupIndex) =>
  props.isReasoningExpanded(getExplorationGroupReasoningId(message, groupIndex))

const getExplorationToolExpandId = (message, groupIndex, toolIndex) =>
  `${message?.displayId || message?.id || 'exploration'}:group_tool:${groupIndex}:${toolIndex}`

const isExplorationToolExpanded = (message, groupIndex, toolIndex) =>
  props.isMessageExpanded({
    displayId: getExplorationToolExpandId(message, groupIndex, toolIndex),
    metadata: {},
    toolDisplay: {}
  })

const shouldShowExplorationToolRawContent = tool => {
  if (!tool) return false
  if (tool.sourceMessage) return props.shouldShowToolRawContent(tool.sourceMessage)
  return !!props.removeSystemReminder(tool.message || '').trim()
}

const getVisibleUserContent = message => props.removeSystemReminder(message?.message || '')

const getUserMessageExpandId = message => `${message?.displayId || message?.id || 'user'}:user`

const isUserMessageExpanded = message =>
  props.isMessageExpanded({
    displayId: getUserMessageExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const isExpandableUserMessage = message => {
  if (!message || getAskUserResponseItems(message).length > 0) return false
  const content = getVisibleUserContent(message)
  if (!content) return false

  const lines = content.split('\n')
  if (lines.length > 2) return true

  return content.trim().length > 120
}

const getMessageSubAgentId = message => {
  const meta = message?.metadata || {}
  if (meta.sub_agent_id || meta.subAgentId) return meta.sub_agent_id || meta.subAgentId
  if ((meta.tool_name || '').toLowerCase() !== 'sub_agent_run') return null

  try {
    const parsed = JSON.parse(message.message || '{}')
    return parsed.task_id || parsed.taskId || null
  } catch {
    return null
  }
}

const getChoiceGroups = message =>
  props.parseChoiceContent(props.removeSystemReminder(message.message || '')).groups || []

const isSubAgentRunMessage = message =>
  String(message?.metadata?.tool_name || '').toLowerCase() === 'sub_agent_run' &&
  !!message?.subAgentCard

const getSubAgentStatusLabel = message => {
  const status = String(message?.subAgentCard?.status || 'running').toLowerCase()
  if (status === 'completed') return 'Completed'
  if (status === 'failed') return 'Failed'
  if (status === 'cancelled' || status === 'interrupted') return 'Stopped'
  return 'Running'
}

const getSubAgentLiveContext = message => {
  const card = message?.subAgentCard || {}
  const currentContextTokens = Number(card.currentContextTokens)
  const maxContextTokens = Number(card.maxContextTokens)

  if (
    Number.isFinite(currentContextTokens) &&
    currentContextTokens >= 0 &&
    Number.isFinite(maxContextTokens) &&
    maxContextTokens > 0
  ) {
    return `${Math.round((currentContextTokens / maxContextTokens) * 100)}% ctx`
  }

  return '--'
}

const getSubAgentLiveTools = message => {
  const card = message?.subAgentCard || {}
  const toolCallsCount = Number(card.toolCallsCount || 0)

  if (Number.isFinite(toolCallsCount) && toolCallsCount > 0) {
    return `${toolCallsCount} tools`
  }
  return '0 tools'
}

const subAgentStatusClass = message => {
  const status = String(message?.subAgentCard?.status || 'running').toLowerCase()
  if (status === 'completed') return 'is-completed'
  if (status === 'failed') return 'is-failed'
  if (status === 'cancelled' || status === 'interrupted') return 'is-stopped'
  return 'is-running'
}

const getSubAgentResultPreview = message => {
  const result = props
    .removeSystemReminder(message?.subAgentCard?.result || '')
    .replace(/\s+/g, ' ')
  if (!result) return ''
  return result.length > 96 ? `${result.slice(0, 96)}...` : result
}

const getSubAgentTaskExpandId = message => `${message?.displayId || message?.id || ''}:task`

const isSubAgentTaskExpanded = message => {
  return props.isMessageExpanded({
    displayId: getSubAgentTaskExpandId(message),
    metadata: {},
    toolDisplay: {}
  })
}

const getSubAgentTaskPreview = message => {
  const task = props.removeSystemReminder(message?.subAgentCard?.task || '').replace(/\s+/g, ' ')
  if (!task) return ''
  return task.length > 96 ? `${task.slice(0, 96)}...` : task
}

const ensureAskUserDraft = message => {
  const key = getChoiceKey(message)
  if (!key) return {}
  if (askUserDrafts.value[key]) return askUserDrafts.value[key]

  const groups = getChoiceGroups(message)
  const nextDraft = groups.reduce((acc, group) => {
    acc[group.title] = {
      selection: '',
      customInput: ''
    }
    return acc
  }, {})

  askUserDrafts.value = {
    ...askUserDrafts.value,
    [key]: nextDraft
  }

  return nextDraft
}

const updateAskUserDraft = (message, updater) => {
  const key = getChoiceKey(message)
  if (!key) return
  const current = ensureAskUserDraft(message)
  askUserDrafts.value = {
    ...askUserDrafts.value,
    [key]: updater(current)
  }
}

const getAskUserSelection = (message, title) => ensureAskUserDraft(message)[title]?.selection || ''

const setAskUserSelection = (message, title, value) => {
  updateAskUserDraft(message, current => ({
    ...current,
    [title]: {
      ...current[title],
      selection: value
    }
  }))
}

const getAskUserCustomInput = (message, title) =>
  ensureAskUserDraft(message)[title]?.customInput || ''

const setAskUserCustomInput = (message, title, value) => {
  updateAskUserDraft(message, current => ({
    ...current,
    [title]: {
      ...current[title],
      selection: value?.trim() ? CUSTOM_ASK_USER_VALUE : current[title]?.selection,
      customInput: value
    }
  }))
}

/**
 * Check if there is a real user message after the given message in props.messages.
 * Uses the message's displayId to find its actual position, avoiding index mismatch
 * between visibleMessages (v-for) and props.messages (filtered/merged).
 */
const hasRealUserResponseAfter = message => {
  const startIndex = props.messages.findIndex(
    m => (m.displayId || m.id) === (message.displayId || message.id)
  )
  if (startIndex === -1) return false

  for (let i = startIndex + 1; i < props.messages.length; i++) {
    const msg = props.messages[i]
    if (msg?.role !== 'user') continue
    if (msg?.metadata?.queue_status === 'queued') continue
    const content = props.removeSystemReminder(msg.message || '').trim()
    if (!content) continue
    return true
  }
  return false
}

/**
 * Check whether a user can answer an ask_user message.
 *
 * The function uses a two-layer check:
 * 1. Workflow state (primary): the session must be in AwaitingUser / wait_reason=user_input.
 * 2. Message-level fallback (redundancy): the message itself has display_type=choice
 *    and parsable choice groups. This guards against edge cases where the state event
 *    arrives slightly before or after the message event due to reactivity ordering.
 */
const canAnswerAskUser = (message) => {
  const currentStatus = String(workflowStore.currentWorkflow?.status || '').toLowerCase()
  const isAwaitingUser =
    workflowStore.waitReason === WORKFLOW_WAIT_REASONS.USER_INPUT ||
    currentStatus === WORKFLOW_STATUSES.AWAITING_USER

  // Primary guard: workflow must be in user-waiting state
  if (!isAwaitingUser) return false

  // Must have parsable choice groups
  if (!getChoiceGroups(message).length) return false

  // Check no real user response already exists after this message
  return !hasRealUserResponseAfter(message)
}

const buildAskUserResponse = message => {
  const groups = getChoiceGroups(message)
  const draft = ensureAskUserDraft(message)
  const selections = []

  for (const group of groups) {
    const groupDraft = draft[group.title] || {}
    const selection = groupDraft.selection || ''
    const customInput = (groupDraft.customInput || '').trim()

    if (!selection) {
      return {
        ok: false,
        error: 'workflow.askUser.validationRequired'
      }
    }

    if (selection === CUSTOM_ASK_USER_VALUE) {
      if (!customInput) {
        return {
          ok: false,
          error: 'workflow.askUser.validationCustomRequired'
        }
      }

      selections.push({
        title: group.title,
        choice_index: group.options.length + 1,
        choice: customInput,
        source: 'custom'
      })
      continue
    }

    const optionIndex = group.options.findIndex(option => option === selection)
    if (optionIndex === -1) {
      return {
        ok: false,
        error: 'workflow.askUser.validationRequired'
      }
    }

    selections.push({
      title: group.title,
      choice_index: optionIndex + 1,
      choice: selection,
      source: 'option'
    })
  }

  return {
    ok: true,
    content: `<ask_user_response>\n${JSON.stringify(selections, null, 2)}\n</ask_user_response>`
  }
}

const submitAskUserResponse = message => {
  const result = buildAskUserResponse(message)
  if (!result.ok) {
    showMessage(t(result.error), 'warning')
    return
  }

  emit('submit-ask-user', result.content)
}

const scrollToBottom = (force = false) => {
  if (messagesRef.value) {
    const el = messagesRef.value
    if (force || shouldAutoScroll.value || isNearBottom(el)) {
      nextTick(() => {
        el.scrollTop = el.scrollHeight
        shouldAutoScroll.value = true
      })
    }
  }
}

defineExpose({
  scrollToBottom,
  messagesRef
})
</script>

<style scoped lang="scss">
.simple-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;

  &.is-expandable {
    cursor: pointer;
  }

  &.is-collapsed {
    position: relative;
    max-height: 50px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  &.is-collapsed::after {
    content: '...';
    position: absolute;
    right: 0;
    bottom: 0;
    padding-left: 12px;
    color: var(--cs-text-color-secondary);
    background: linear-gradient(
      90deg,
      rgb(from var(--cs-bg-color) r g b / 0) 0%,
      var(--cs-bg-color) 40%
    );
  }
}

.exploration-card {
  margin-bottom: 12px;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  background: var(--cs-bg-color-light);
  overflow: hidden;
}

.exploration-card__header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: var(--cs-space-sm) var(--cs-space);
  cursor: pointer;
  background: var(--cs-bg-color);
}

.exploration-card__header:hover {
  background: var(--cs-hover-bg-color);
}

.exploration-card__title-wrap {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
  flex: 0 1 auto;
}

.exploration-card__title {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--cs-text-color-primary);
  font-size: var(--cs-font-size-sm);
  font-weight: 600;
  flex-shrink: 0;
}

.exploration-card__icon {
  color: var(--el-color-primary);
}

.exploration-card__meta,
.exploration-card__preview,
.exploration-card__tool-summary,
.exploration-card__thought-label {
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
}

.exploration-card__preview {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.exploration-card__chevron {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--cs-text-color-secondary);
  transition: transform 0.2s ease;
}

.exploration-card__chevron.expanded {
  transform: rotate(180deg);
}

.exploration-card__body {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm);
  padding: var(--cs-space-sm) var(--cs-space);
  border-top: 1px solid var(--cs-border-color);
}

.exploration-card__step-card {
  min-width: 0;
  padding: 8px 10px;
  border-radius: var(--cs-border-radius);
}

.exploration-card__reasoning {
  margin-bottom: 0;

  .reasoning-content {
    background: none !important;
  }
}

.exploration-card__tool {
  margin-bottom: 0;
  padding-left: 0;
  border-left: none;
}

.context-snapshot-card {
  margin-bottom: 12px;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  background: var(--cs-bg-color-light);
  overflow: hidden;
}

.context-snapshot-card__header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: var(--cs-space-sm) var(--cs-space);
  cursor: pointer;
  color: var(--cs-text-color-primary);
  background: var(--cs-bg-color);
}

.context-snapshot-card__header:hover {
  background: var(--cs-hover-bg-color);
}

.context-snapshot-card__icon {
  color: var(--el-color-primary);
}

.context-snapshot-card__title {
  font-size: var(--cs-font-size-sm);
  font-weight: 600;
}

.context-snapshot-card__preview {
  flex: 1;
  min-width: 0;
  font-size: var(--cs-font-size-xs);
  color: var(--cs-text-color-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.context-snapshot-card__chevron {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--cs-text-color-secondary);
  transition: transform 0.2s ease;
}

.context-snapshot-card__chevron.expanded {
  transform: rotate(180deg);
}

.context-snapshot-card__body {
  padding: var(--cs-space-sm) var(--cs-space);
  border-top: 1px solid var(--cs-border-color);
}

/* Pending approval queue items (non-first in FIFO) */
.approval-queue-item {
  opacity: 0.7;
  padding-left: var(--cs-space-lg, 16px);
  border-left: 2px solid var(--el-color-warning);
  margin-bottom: 4px;
  cursor: default;
  pointer-events: none;
}

.approval-queue-item .tool-type-icon {
  color: var(--el-color-warning);
}

.queued-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  padding: 2px;
  border: none;
  background: transparent;
  color: var(--cs-text-color-secondary);
  cursor: pointer;
}

.queued-remove:hover {
  color: var(--cs-text-color-primary);
}
</style>
