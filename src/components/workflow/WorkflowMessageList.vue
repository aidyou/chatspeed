<template>
  <div
    class="messages"
    ref="messagesRef"
    :data-workflow-id="props.currentWorkflowId || null"
    @scroll.passive="handleScroll">
    <div v-if="props.isLoading" class="message-skeleton" aria-busy="true">
      <el-skeleton animated>
        <template #template>
          <div class="message-skeleton__message message-skeleton__message--assistant">
            <el-skeleton-item variant="text" class="message-skeleton__line message-skeleton__line--wide" />
            <el-skeleton-item variant="text" class="message-skeleton__line" />
            <el-skeleton-item variant="text" class="message-skeleton__line message-skeleton__line--short" />
          </div>
          <div class="message-skeleton__message message-skeleton__message--user">
            <el-skeleton-item variant="text" class="message-skeleton__line message-skeleton__line--medium" />
          </div>
          <div class="message-skeleton__message message-skeleton__message--assistant">
            <el-skeleton-item variant="text" class="message-skeleton__line message-skeleton__line--medium" />
            <el-skeleton-item variant="text" class="message-skeleton__line message-skeleton__line--short" />
          </div>
        </template>
      </el-skeleton>
    </div>

    <template v-else>
      <a
      v-if="props.hiddenEarlierMessageCount > 0"
      class="history-window-indicator"
      @click="revealEarlierMessages">
      <cs name="double-arrow-down" class="cs-rotate" size="var(--cs-font-size-xs)" />
      <span>{{ t('workflow.earlierMessages') }}</span>
    </a>
    <a
      v-if="
        props.hiddenEarlierMessageCount <= 0 && props.hiddenCompletedTaskGroupCount > 0
      "
      class="history-window-indicator"
      @click="revealEarlierTaskGroup">
      <cs name="double-arrow-down" class="cs-rotate" size="var(--cs-font-size-xs)" />
      <span>{{ t('workflow.earlierTasks', { count: props.hiddenCompletedTaskGroupCount }) }}</span>
    </a>

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
          <div
            v-if="message.metadata?.attachments?.length > 0"
            class="workflow-message-attachments">
            <div
              v-for="(attachment, attachmentIndex) in message.metadata.attachments"
              :key="`${message.displayId || message.id || attachmentIndex}_attachment_${attachmentIndex}`"
              class="workflow-message-attachment-item">
              <el-image
                v-if="attachment.type === 'image'"
                :src="attachment.url"
                :preview-src-list="[attachment.url]"
                :initial-index="0"
                fit="cover"
                class="workflow-message-attachment-image"
                preview-teleported />
            </div>
          </div>
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
          <div
            v-else
            class="user-message-wrap"
            :class="{
              'is-expandable': isExpandableUserMessage(message),
              'is-collapsed': isExpandableUserMessage(message) && !isUserMessageExpanded(message)
            }"
            @click="
              isExpandableUserMessage(message) &&
              $emit('toggle-expand', getUserMessageExpandId(message))
            ">
            <pre
              :data-user-expand-id="getUserMessageExpandId(message)"
              :style="getUserMessageCollapsedStyle(message)"
              class="simple-text"
              :class="{
                'is-collapsed': !isUserMessageExpanded(message),
                'is-expandable': isExpandableUserMessage(message)
              }"
              >{{ getVisibleUserContent(message) }}</pre
            >
            <button
              v-if="isExpandableUserMessage(message)"
              type="button"
              class="user-message-toggle"
              :aria-label="isUserMessageExpanded(message) ? 'Collapse message' : 'Expand message'"
              @click.stop="$emit('toggle-expand', getUserMessageExpandId(message))">
              <cs
                name="double-arrow-down"
                size="14px"
                class="user-message-toggle__icon"
                :class="{ expanded: isUserMessageExpanded(message) }" />
            </button>
          </div>
        </div>
        <div v-else class="ai-content chat">
          <div
            v-if="isCollapsedToolGroupMessage(message)"
            class="cli-tool-call tool-group"
            :class="{
              'tool-group--running':
                isCollapsedToolGroupRunning(message) || isStreamingThoughtMergedIntoToolGroup(message)
            }">
            <div
              class="tool-line title-wrap expandable tool-group__summary"
              :class="{ expanded: isMessageExpanded(message) }"
              @click="$emit('toggle-expand', message.displayId)">
              <cs :name="message.groupDisplay.icon || 'tool'" size="15px" class="tool-type-icon" />
              <span
                v-if="getCollapsedToolGroupThoughtSummary(message)"
                class="tool-group__thought-count">
                {{ getCollapsedToolGroupThoughtSummary(message) }}
              </span>
              <span v-if="message.groupDisplay.errorSummary" class="tool-group__error-count">
                {{ message.groupDisplay.errorSummary }}
              </span>
              <span class="tool-group__summary-text">{{ message.groupDisplay.summary }}</span>
              <button
                type="button"
                class="tool-group__expand-button"
                :aria-label="$t('workflow.toolGroups.expand')"
                @click.stop="$emit('toggle-expand', message.displayId)">
                <cs name="caret-down" size="14px" />
              </button>
            </div>
            <div v-if="isMessageExpanded(message)" class="collapsed-tool-group__body">
              <div
                v-for="(thought, thoughtIndex) in message.groupedThoughts || []"
                :key="`${message.displayId}_grouped_thought_${thoughtIndex}`"
                class="reasoning-container collapsed-tool-group__thought"
                :style="{ order: thought.groupOrder ?? thoughtIndex }">
                <div class="reasoning-header" @click="toggleReasoningForMessage(thought)">
                  <cs name="reasoning" size="14px" class="reasoning-icon" />
                  <span class="reasoning-text">
                    {{
                      isReasoningExpandedForMessage(thought)
                        ? $t('workflow.thinkingExpanded') || 'Thinking Process'
                        : $t('workflow.thoughtCompleted') || 'Thought Complete'
                    }}
                  </span>
                  <span class="reasoning-toggle">
                    {{ isReasoningExpandedForMessage(thought) ? '▲' : '▼' }}
                  </span>
                </div>
                <div v-if="isReasoningExpandedForMessage(thought)" class="reasoning-content">
                  {{ thought.reasoning || thought.message }}
                </div>
              </div>
              <div
                v-if="isStreamingThoughtMergedIntoToolGroup(message)"
                class="reasoning-container collapsed-tool-group__thought"
                :style="{ order: getStreamingThoughtGroupOrder(message) }">
                <div
                  class="reasoning-header"
                  @click="$emit('toggle-reasoning', STREAMING_REASONING_ID)">
                  <cs
                    name="reasoning"
                    size="14px"
                    class="reasoning-icon"
                    :class="{
                      rotating:
                        !hasStreamingThoughtCompleted && !isReasoningExpanded(STREAMING_REASONING_ID)
                    }" />
                  <span class="reasoning-text">
                    {{
                      isReasoningExpanded(STREAMING_REASONING_ID)
                        ? $t('workflow.thinkingExpanded') || 'Thinking Process'
                        : hasStreamingThoughtCompleted
                          ? $t('workflow.thoughtCompleted') || 'Thought Complete'
                          : $t('workflow.thinking') || 'Thinking...'
                    }}
                  </span>
                  <span class="reasoning-toggle">
                    {{ isReasoningExpanded(STREAMING_REASONING_ID) ? '▲' : '▼' }}
                  </span>
                </div>
                <div v-if="isReasoningExpanded(STREAMING_REASONING_ID)" class="reasoning-content">
                  {{ chatState.reasoning }}
                </div>
              </div>
              <div
                v-for="(tool, toolIndex) in message.groupedTools"
                :key="`${message.displayId}_grouped_tool_${toolIndex}`"
                class="cli-tool-call collapsed-tool-group__item"
                :class="[
                  tool.toolDisplay?.toolType || 'tool-system',
                  tool.toolDisplay?.isError ? 'status-error' : 'status-success'
                ]"
                :style="{ order: tool.groupOrder ?? toolIndex }">
                <div
                  class="tool-line title-wrap expandable"
                  :class="{
                    'tool-rejected': tool.isRejected,
                    'title-wrap--bash': isBashToolCall(tool)
                  }"
                  @click="$emit('toggle-expand', tool.displayId)">
                  <cs
                    :name="tool.toolDisplay?.icon || 'tool'"
                    size="15px"
                    class="tool-type-icon" />
                  <span class="tool-name" :class="{ 'tool-name--bash': isBashToolCall(tool) }">{{
                    tool.toolDisplay?.action
                  }}</span>
                  <span v-if="getToolTitleTarget(tool)" class="tool-target">{{
                    getToolTitleTarget(tool)
                  }}</span>
                  <span
                    v-if="isBashToolCall(tool) && getShellExecutionRouteLabel(tool)"
                    class="shell-execution-route-badge">
                    {{ getShellExecutionRouteLabel(tool) }}
                  </span>
                  <span
                    v-if="getToolExecutionDurationLabel(tool)"
                    class="shell-execution-route-badge">
                    {{ getToolExecutionDurationLabel(tool) }}
                  </span>
                  <cs v-if="tool.isApproved" name="check" size="14px" class="approved-icon" />
                </div>
                <div
                  v-if="!isToolMessageExpanded(tool)"
                  class="tool-line summary expandable"
                  @click="$emit('toggle-expand', tool.displayId)">
                  <span class="corner-icon">⎿</span>
                  <span class="summary-text">{{ getToolSummaryText(tool) }}</span>
                  <span class="expand-hint">(click to expand)</span>
                </div>
                <div
                  v-if="isToolMessageExpanded(tool)"
                  class="tool-detail tool-detail--expanded"
                  :class="getToolDetailClass(tool)">
                  <div v-if="isBashToolCall(tool)" class="bash-command-frame">
                    <span class="bash-command-frame__prompt" aria-hidden="true">$</span>
                    <pre class="bash-command" aria-label="Bash command"><code
                      class="hljs"
                      v-html="getHighlightedBashCommand(tool)"></code></pre>
                  </div>
                  <div
                    v-if="isMcpToolMessage(tool) && getFormattedMcpToolArguments(tool)"
                    class="mcp-tool-arguments">
                    <div class="mcp-tool-arguments__label">{{ $t('workflow.taskLedger.arguments') }}</div>
                    <pre class="raw-content">{{ getFormattedMcpToolArguments(tool) }}</pre>
                  </div>
                  <div
                    v-if="
                      tool.metadata?.tool_call_id &&
                      workflowStore.getToolStream(tool.metadata.tool_call_id).length > 0
                    "
                    class="tool-stream-output">
                    <div
                      v-for="(line, idx) in workflowStore.getToolStream(tool.metadata.tool_call_id)"
                      :key="idx"
                      class="stream-line">
                      {{ line }}
                    </div>
                  </div>
                  <div
                    v-else-if="shouldShowRunningPlaceholder(tool)"
                    class="tool-running-placeholder">
                    <cs name="loading" size="14px" class="tool-running-placeholder__icon cs-spin" />
                    <span class="tool-running-placeholder__text">
                      {{ getRunningPlaceholderText(tool) }}
                    </span>
                  </div>
                  <div
                    v-else-if="shouldShowToolRawContent(tool) && tool.toolDisplay?.displayType === 'diff'"
                    class="tool-diff-view">
                    <FilePreviewDiff
                      :file-path="getDiffFilePath(tool)"
                      :old-content="getDiffOldContent(tool)"
                      :new-content="getDiffNewContent(tool)"
                      :context-data="getDiffContextData(tool)" />
                  </div>
                  <div
                    v-else-if="
                      shouldShowToolRawContent(tool) && tool.toolDisplay?.displayType === 'choice'
                    "
                    class="choice-container">
                    <div
                      v-for="group in getChoiceGroups(tool)"
                      :key="group.title"
                      class="choice-group">
                      <div class="choice-question">
                        {{ group.title }}
                      </div>
                      <div class="choice-options vertical numbered choice-options--readonly">
                        <div
                          v-for="(opt, optIndex) in group.options"
                          :key="`${group.title}-${opt}`"
                          class="choice-option-label">
                          {{ optIndex + 1 }}. {{ opt }}
                        </div>
                      </div>
                    </div>
                  </div>
                  <MarkdownSimple
                    v-else-if="
                      shouldShowToolRawContent(tool) &&
                      tool.toolDisplay?.displayType === 'markdown' &&
                      !isMcpToolMessage(tool)
                    "
                    :content="removeSystemReminder(tool.message)" />
                  <pre
                    v-else-if="
                      shouldShowToolRawContent(tool) && !isDuplicateBashCommandContent(tool)
                    "
                    class="raw-content"
                    >{{ getMcpToolContentForDisplay(tool) }}</pre
                  >
                </div>
              </div>
            </div>
          </div>
          <div v-else-if="isExplorationBatchMessage(message)" class="exploration-card">
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
                      <span class="reasoning-text">
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
                      {{ sanitizeReasoningContent(group.thought) }}
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
                    :class="{ 'title-wrap--bash': isBashToolCall(tool) }"
                    @click="
                      $emit(
                        'toggle-expand',
                        getExplorationToolExpandId(message, groupIndex, toolIndex)
                      )
                    ">
                    <cs
                      :name="tool.icon || 'tool'"
                      size="14px"
                      class="tool-type-icon" />
                    <span class="tool-name" :class="{ 'tool-name--bash': isBashToolCall(tool) }">{{
                      tool.action
                    }}</span>
                    <span v-if="getToolTitleTarget(tool)" class="tool-target">{{
                      getToolTitleTarget(tool)
                    }}</span>
                  </div>
                  <div
                    v-if="
                      tool.summary && !isExplorationToolExpanded(message, groupIndex, toolIndex)
                    "
                    class="tool-line summary expandable"
                    @click="
                      $emit(
                        'toggle-expand',
                        getExplorationToolExpandId(message, groupIndex, toolIndex)
                      )
                    ">
                    <span class="corner-icon">⎿</span>
                    <span class="summary-text">{{ tool.summary }}</span>
                    <span class="expand-hint">(click to expand)</span>
                  </div>
                  <div
                    v-if="isExplorationToolExpanded(message, groupIndex, toolIndex)"
                    class="tool-detail tool-detail--expanded"
                    :class="getToolDetailClass(tool)">
                    <div v-if="isBashToolCall(tool)" class="bash-command-frame">
                      <span class="bash-command-frame__prompt" aria-hidden="true">$</span>
                      <pre class="bash-command" aria-label="Bash command"><code
                        class="hljs"
                        v-html="getHighlightedBashCommand(tool)"></code></pre>
                    </div>
                    <MarkdownSimple
                      v-if="
                        shouldShowExplorationToolRawContent(tool) && tool.displayType === 'diff'
                      "
                      :content="getDiffMarkdown(removeSystemReminder(tool.message))" />
                    <MarkdownSimple
                      v-else-if="
                        shouldShowExplorationToolRawContent(tool) &&
                        tool.displayType === 'markdown' &&
                        !isMcpToolMessage(tool)
                      "
                      :content="removeSystemReminder(tool.message)" />
                    <pre
                      v-else-if="
                        shouldShowExplorationToolRawContent(tool) &&
                        !isDuplicateBashCommandContent(tool)
                      "
                      class="raw-content"
                      >{{ getMcpToolContentForDisplay(tool) }}</pre
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
              <div
                class="tool-line title-wrap expandable tool-group__summary"
                :class="{ expanded: isSubAgentCardExpanded(message) }"
                @click="$emit('toggle-expand', getSubAgentCardExpandId(message))">
                <cs name="skill-relation-chart" size="15px" class="tool-type-icon" />
                <span
                  class="sub-agent-card__toggle-status"
                  :class="getSubAgentStatusBadgeClass(message)">
                  <cs name="loading" class="cs-spin" v-if="subagentIsRunning(message)" />
                  {{ getSubAgentStatusLabel(message) }}
                </span>
                <span class="tool-group__summary-text">{{ message.subAgentCard.agent }}</span>
                <button
                  type="button"
                  class="tool-group__expand-button"
                  :aria-label="$t('workflow.toolGroups.expand')"
                  @click.stop="$emit('toggle-expand', getSubAgentCardExpandId(message))">
                  <cs name="caret-down" size="14px" />
                </button>
              </div>

              <div
                v-if="isSubAgentCardExpanded(message)"
                class="sub-agent-card"
                :class="{
                  'is-expanded': isSubAgentCardExpanded(message),
                  'is-collapsed': !isSubAgentCardExpanded(message)
                }">
                <div class="sub-agent-card__header">
                  <div class="sub-agent-card__title-wrap">
                    <div class="sub-agent-card__title">
                      <cs name="task" size="15px" class="sub-agent-card__icon" />
                      <span>Delegated Task</span>
                    </div>
                    <div class="sub-agent-card__status" :class="subAgentStatusClass(message)">
                      <cs name="loading" class="cs-spin" v-if="subagentIsRunning(message)" />
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
                      <span class="sub-agent-card__value mode">{{
                        message.subAgentCard.mode
                      }}</span>
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
                    <div class="sub-agent-card__section-actions">
                      <button
                        type="button"
                        class="sub-agent-card__copy-button"
                        :title="$t('common.copy')"
                        :aria-label="$t('common.copy')"
                        @click.stop="copySubAgentTask(message)">
                        <cs name="copy" />
                      </button>
                      <cs
                        name="double-arrow-down"
                        size="14px"
                        class="sub-agent-card__task-chevron"
                        :class="{ expanded: isSubAgentTaskExpanded(message) }" />
                    </div>
                  </div>
                  <div v-if="isSubAgentTaskExpanded(message)" class="sub-agent-card__task-body">
                    <MarkdownSimple :content="message.subAgentCard.taskMarkdown" />
                  </div>
                </div>

                <div
                  v-if="message.subAgentCard.hasResult"
                  class="sub-agent-card__result"
                  :class="{ expanded: isSubAgentResultExpanded(message) }">
                  <div
                    class="sub-agent-card__result-toggle"
                    @click="$emit('toggle-expand', getSubAgentResultExpandId(message))">
                    <div class="sub-agent-card__result-heading">
                      <span class="sub-agent-card__label">Result</span>
                      <span
                        v-if="!isSubAgentResultExpanded(message)"
                        class="sub-agent-card__result-preview"
                        >{{ getSubAgentResultPreview(message) }}</span
                      >
                    </div>
                    <div class="sub-agent-card__section-actions">
                      <button
                        type="button"
                        class="sub-agent-card__copy-button"
                        :title="$t('common.copy')"
                        :aria-label="$t('common.copy')"
                        @click.stop="copySubAgentResult(message)">
                        <cs name="copy" />
                      </button>
                      <cs
                        name="double-arrow-down"
                        size="14px"
                        class="sub-agent-card__result-chevron"
                        :class="{ expanded: isSubAgentResultExpanded(message) }" />
                    </div>
                  </div>
                  <div v-if="isSubAgentResultExpanded(message)" class="sub-agent-card__result-body">
                    <MarkdownSimple :content="message.subAgentCard.resultMarkdown" />
                  </div>
                </div>

                <div
                  v-if="getSubAgentUsageSummary(message)"
                  class="sub-agent-card__cost"
                  :class="{ expanded: isSubAgentCostExpanded(message) }">
                  <div
                    class="sub-agent-card__cost-toggle"
                    @click="$emit('toggle-expand', getSubAgentCostExpandId(message))">
                    <div class="sub-agent-card__cost-heading">
                      <span class="sub-agent-card__label">Cost</span>
                    </div>
                    <div class="sub-agent-card__section-actions">
                      <cs
                        name="double-arrow-down"
                        size="14px"
                        class="sub-agent-card__cost-chevron"
                        :class="{ expanded: isSubAgentCostExpanded(message) }" />
                    </div>
                  </div>
                  <WorkflowCostAnalysis
                    v-if="isSubAgentCostExpanded(message)"
                    :summary="getSubAgentUsageSummary(message)" />
                </div>
              </div>
            </template>

            <!-- complete_workflow special display -->
            <template v-else-if="isFinishTaskMessage(message)">
              <button
                v-if="getFinishTaskUsageSummary(message)"
                type="button"
                class="tool-line finish-task-display finish-task-display--expandable"
                @click="$emit('toggle-expand', getFinishTaskCostExpandId(message))">
                <cs
                  :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'"
                  size="14px"
                  class="tool-type-icon finish-icon" />
                <span class="finish-text">{{ getFinishTaskLabel(message) }}</span>
                <cs name="arrow-right" size="14px" class="finish-task-cost-arrow" />
              </button>
              <div v-else class="tool-line finish-task-display">
                <cs
                  :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'"
                  size="14px"
                  class="tool-type-icon finish-icon" />
                <span class="finish-text">{{ getFinishTaskLabel(message) }}</span>
              </div>
              <WorkflowCostAnalysis
                v-if="getFinishTaskUsageSummary(message) && isFinishTaskCostExpanded(message)"
                class="finish-task-cost-card"
                :summary="getFinishTaskUsageSummary(message)" />
            </template>

            <!-- Normal tool call display -->
            <template v-else>
              <div
                class="tool-line title-wrap expandable"
                :class="{
                  'tool-rejected': message.isRejected,
                  'title-wrap--bash': isBashToolCall(message)
                }"
                @click="$emit('toggle-expand', message.displayId)">
                <cs :name="message.toolDisplay.icon || 'tool'" size="15px" class="tool-type-icon" />
                <span class="tool-name" :class="{ 'tool-name--bash': isBashToolCall(message) }">{{
                  message.toolDisplay.action
                }}</span>
                <span v-if="getToolTitleTarget(message)" class="tool-target">{{
                  getToolTitleTarget(message)
                }}</span>
                <span
                  v-if="isBashToolCall(message) && getShellExecutionRouteLabel(message)"
                  class="shell-execution-route-badge">
                  {{ getShellExecutionRouteLabel(message) }}
                </span>
                <span
                  v-if="getToolExecutionDurationLabel(message)"
                  class="shell-execution-route-badge">
                  {{ getToolExecutionDurationLabel(message) }}
                </span>
                <cs v-if="message.isApproved" name="check" size="14px" class="approved-icon" />
              </div>
              <!-- Hide summary when expanded -->
              <div
                class="tool-line summary expandable"
                v-if="!isToolMessageExpanded(message)"
                @click="$emit('toggle-expand', message.displayId)">
                <span class="corner-icon">⎿</span>
                <span class="summary-text">{{ getToolSummaryText(message) }}</span>
                <span class="expand-hint">(click to expand)</span>
              </div>
              <div
                v-if="isToolMessageExpanded(message)"
                class="tool-detail tool-detail--expanded"
                :class="getToolDetailClass(message)">
                <div
                  v-if="isBashToolCall(message) && !isApprovalPending(message)"
                  class="bash-command-frame">
                  <span class="bash-command-frame__prompt" aria-hidden="true">$</span>
                  <pre class="bash-command" aria-label="Bash command"><code
                    class="hljs"
                    v-html="getHighlightedBashCommand(message)"></code></pre>
                </div>
                <div
                  v-if="isMcpToolMessage(message) && getFormattedMcpToolArguments(message)"
                  class="mcp-tool-arguments">
                  <div class="mcp-tool-arguments__label">{{ $t('workflow.taskLedger.arguments') }}</div>
                  <pre class="raw-content">{{ getFormattedMcpToolArguments(message) }}</pre>
                </div>
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
                <div
                  v-else-if="shouldShowRunningPlaceholder(message)"
                  class="tool-running-placeholder">
                  <cs name="loading" size="14px" class="tool-running-placeholder__icon cs-spin" />
                  <span class="tool-running-placeholder__text">
                    {{ getRunningPlaceholderText(message) }}
                  </span>
                </div>
                <!-- Final Result -->
                <div
                  v-if="
                    !isApprovalPending(message) &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'diff'
                  "
                  class="tool-diff-view">
                  <FilePreviewDiff
                    :file-path="getDiffFilePath(message)"
                    :old-content="getDiffOldContent(message)"
                    :new-content="getDiffNewContent(message)"
                    :context-data="getDiffContextData(message)" />
                </div>
                <div
                  v-else-if="
                    !isApprovalPending(message) &&
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
                    !isApprovalPending(message) &&
                    shouldShowToolRawContent(message) &&
                    message.toolDisplay.displayType === 'markdown' &&
                    !isMcpToolMessage(message)
                  "
                  :content="removeSystemReminder(message.message)" />
                <pre
                  v-else-if="
                    !isApprovalPending(message) &&
                    shouldShowToolRawContent(message) &&
                    !isDuplicateBashCommandContent(message)
                  "
                  class="raw-content"
                  >{{ getMcpToolContentForDisplay(message) }}</pre
                >
                <ApprovalDialog
                  v-if="shouldShowApprovalDialog(message)"
                  inline
                  :tool-name="getMessageToolName(message)"
                  :details="getApprovalDetailsPayload(message)"
                  :display-type="message.metadata?.display_type || message.toolDisplay.displayType"
                  :rejection-message="getApprovalDraft(message.metadata?.tool_call_id)"
                  :loading="approvalLoading && activeApprovalId === message.metadata?.tool_call_id"
                  :pending-count="inlineBulkApprovalCount"
                  @update:rejection-message="
                    value => setApprovalDraft(message.metadata?.tool_call_id, value)
                  "
                  @approve="onApproveTool(message.metadata?.tool_call_id)"
                  @approve-all="onApproveAllTool(message.metadata?.tool_call_id)"
                  @approve-all-pending="onApproveAllPending(message.metadata?.tool_call_id)"
                  @reject="onRejectTool(message.metadata?.tool_call_id)" />
              </div>
            </template>
          </div>

          <!-- Regular Assistant Content -->
          <div v-else>
            <div v-if="isManualClearContextMessage(message)" class="manual-clear-context-divider">
              <span class="manual-clear-context-divider__line"></span>
              <span class="manual-clear-context-divider__label">{{
                $t('workflow.clearContextDivider')
              }}</span>
              <span class="manual-clear-context-divider__line"></span>
            </div>
            <div v-else-if="isContextSnapshotMessage(message)" class="context-snapshot-card">
              <div
                class="context-snapshot-card__header"
                @click="$emit('toggle-expand', getContextSnapshotExpandId(message))">
                <cs name="archive" size="14px" class="context-snapshot-card__icon" />
                <span class="context-snapshot-card__title">{{
                  getContextSnapshotTitle(message)
                }}</span>
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
              <div class="reasoning-header" @click="toggleReasoningForMessage(message)">
                <cs
                  name="reasoning"
                  size="14px"
                  class="reasoning-icon"
                  :class="{
                    rotating:
                      isRunning &&
                      !hasThoughtCompleted(message) &&
                      !isReasoningExpandedForMessage(message) &&
                      message === lastAssistantMessage
                  }" />
                <span class="reasoning-text">
                  <template v-if="isReasoningExpandedForMessage(message)">
                    {{ $t('workflow.thinkingExpanded') || 'Thinking Process' }}
                  </template>
                  <template
                    v-else-if="
                      isRunning && !hasThoughtCompleted(message) && message === lastAssistantMessage
                    ">
                    {{ $t('workflow.thinking') || 'Thinking...' }}
                  </template>
                  <template v-else>
                    {{ $t('workflow.thoughtCompleted') || 'Thought Complete' }}
                  </template>
                </span>
                <span class="reasoning-toggle">
                  {{ isReasoningExpandedForMessage(message) ? '▲' : '▼' }}
                </span>
              </div>
              <div v-if="isReasoningExpandedForMessage(message)" class="reasoning-content">
                {{ message.reasoning || message.message }}
              </div>
            </div>
            <el-alert
              v-if="!isContextSnapshotMessage(message) && shouldShowErrorAlert(message)"
              type="error"
              :closable="false"
              show-icon
              class="workflow-error-alert">
              <template #title>{{ getErrorAlertTitle(message) }}</template>
              <div class="workflow-error-alert__body">
                <MarkdownSimple :content="getErrorAlertContent(message)" />
              </div>
            </el-alert>
            <MarkdownSimple
              v-else-if="!isContextSnapshotMessage(message) && getParsedMessage(message).content"
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
                  <cs
                    :name="call.icon || 'tool'"
                    size="14px"
                    class="tool-type-icon" />
                  <span class="tool-name">{{ call.action }}</span>
                  <span class="tool-target">{{ call.target }}</span>
                </div>
                <div class="tool-line summary">
                  <span class="corner-icon">⎿</span>
                  <span class="summary-text">{{ call.summary }}</span>
                </div>
                <div
                  v-if="
                    call.toolName === 'complete_workflow' && call.completionSummary
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
    <div v-if="shouldShowStandaloneStreamingChat" class="message assistant chatting">
      <div class="content-container">
        <div class="ai-content chat">
          <div v-if="chatState.reasoning" class="reasoning-container">
            <div
              class="reasoning-header"
              @click="$emit('toggle-reasoning', STREAMING_REASONING_ID)">
              <cs
                name="reasoning"
                size="14px"
                class="reasoning-icon"
                :class="{
                  rotating:
                    !hasStreamingThoughtCompleted && !isReasoningExpanded(STREAMING_REASONING_ID)
                }" />
              <span class="reasoning-text">
                {{
                  isReasoningExpanded(STREAMING_REASONING_ID)
                    ? $t('workflow.thinkingExpanded') || 'Thinking Process'
                    : hasStreamingThoughtCompleted
                      ? $t('workflow.thoughtCompleted') || 'Thought Complete'
                      : $t('workflow.thinking') || 'Thinking...'
                }}
              </span>
              <span class="reasoning-toggle">
                {{ isReasoningExpanded(STREAMING_REASONING_ID) ? '▲' : '▼' }}
              </span>
            </div>
            <div v-if="isReasoningExpanded(STREAMING_REASONING_ID)" class="reasoning-content">
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
      <div
        v-for="item in queuedMessages"
        :key="item.id"
        class="queued-item"
        :class="{ 'queued-item--processing': item.status === 'preparing_attachments' }">
        <div class="queued-item-main">
          <cs
            :name="item.icon || 'clock'"
            size="12px"
            class="queued-icon"
            :class="{ 'cs-spin': item.status === 'preparing_attachments' }" />
          <div class="queued-content">
            <span v-if="item.content" class="queued-text">{{ item.content }}</span>
            <div v-if="item.attachments?.length > 0" class="queued-attachments">
              <div
                v-for="(attachment, attachmentIndex) in item.attachments"
                :key="`${item.id}_attachment_${attachment.id || attachmentIndex}`"
                class="queued-attachment-item">
                <el-image
                  v-if="attachment.type === 'image' && (attachment.url || attachment.sourceUrl)"
                  :src="attachment.url || attachment.sourceUrl"
                  :preview-src-list="[attachment.url || attachment.sourceUrl]"
                  :initial-index="0"
                  fit="cover"
                  class="queued-attachment-image"
                  preview-teleported />
                <span v-else class="queued-attachment-name">{{ attachment.name }}</span>
              </div>
            </div>
            <span v-if="item.statusText" class="queued-status-text">{{ item.statusText }}</span>
          </div>
        </div>
        <button
          v-if="canRemoveQueuedMessage(item)"
          type="button"
          class="queued-remove"
          @click="$emit('remove-queued-message', item.id)">
          <cs name="close" size="12px" />
        </button>
      </div>
    </div>
    </template>
  </div>
</template>

<script setup>
import { computed, ref, nextTick, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { writeClipboard } from '@/libs/clipboard'
import { showMessage } from '@/libs/util'
import hljs from 'highlight.js'
import {
  excludeLeadingManualClearContextMarkers,
  isWorkflowCompletionMessage,
  isWorkflowMessagePendingApproval,
  isWorkflowToolAwaitingExecution,
  shouldRenderSubAgentCard
} from '@/composables/workflow/messageProjectionRules'
import { isWorkflowMcpTool } from '@/composables/workflow/toolClassification'
import { normalizeUsageSummary } from '@/composables/workflow/usageSummary'
import { shouldShowInlineApprovalForMessage } from './approvalVisibility'
import WorkflowCostAnalysis from './WorkflowCostAnalysis.vue'
import ApprovalDialog from './ApprovalDialog.vue'
import FilePreviewDiff from './FilePreviewDiff.vue'
import MarkdownSimple from './MarkdownSimple.vue'
import { useWorkflowStore } from '@/stores/workflow'

const workflowStore = useWorkflowStore()
const { t } = useI18n()
const CUSTOM_ASK_USER_VALUE = '__custom__'
const USER_MESSAGE_COLLAPSED_LINE_COUNT = 4
const STREAMING_REASONING_ID = '__streaming_reasoning__'

const props = defineProps({
  messages: {
    type: Array,
    default: () => []
  },
  isLoading: {
    type: Boolean,
    default: false
  },
  hiddenEarlierMessageCount: {
    type: Number,
    default: 0
  },
  hiddenCompletedTaskGroupCount: {
    type: Number,
    default: 0
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
  waitReason: {
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
  askUserSubmitting: {
    type: Boolean,
    default: false
  },
  pendingCount: {
    type: Number,
    default: 0
  },
  pendingApprovals: {
    type: Array,
    default: () => []
  },
  pendingApprovalIds: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits([
  'toggle-expand',
  'toggle-reasoning',
  'scroll-bottom',
  'reveal-earlier-messages',
  'reveal-earlier-task-group',
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
const approvedSubmissionIds = new Set()
const userMessageOverflowMap = ref({})
const userMessageCollapsedHeightMap = ref({})
const isRevealingEarlierTaskGroup = ref(false)
const AUTO_SCROLL_THRESHOLD = 64
const shouldAutoScroll = ref(true)
let userMessageResizeObserver = null
let messageContentResizeObserver = null
let scrollFrameId = null
let scrollCorrectionFrameId = null
let observedMessageListWidth = 0
let scrollScheduled = false
let componentUnmounted = false
let pendingScrollForce = false
let pendingScrollFrameBudget = 0
let userMessageMeasureScheduled = false
let userMessageMeasureFrameId = null

const isNearBottom = el => {
  if (!el) return true
  return el.scrollHeight - el.scrollTop - el.clientHeight <= AUTO_SCROLL_THRESHOLD
}

const canRemoveQueuedMessage = item => item?.removable !== false

const handleScroll = () => {
  shouldAutoScroll.value = isNearBottom(messagesRef.value)
}

const revealEarlierTaskGroup = () => {
  if (isRevealingEarlierTaskGroup.value || props.hiddenCompletedTaskGroupCount <= 0) return

  const container = messagesRef.value
  const previousScrollHeight = container?.scrollHeight || 0
  const previousScrollTop = container?.scrollTop || 0

  isRevealingEarlierTaskGroup.value = true
  emit('reveal-earlier-task-group', () => {
    nextTick(() => {
      if (container) {
        const nextScrollHeight = container.scrollHeight
        container.scrollTop = previousScrollTop + (nextScrollHeight - previousScrollHeight)
      }
      isRevealingEarlierTaskGroup.value = false
    })
  })
}

const revealEarlierMessages = () => {
  if (isRevealingEarlierTaskGroup.value || props.hiddenEarlierMessageCount <= 0) return

  const container = messagesRef.value
  const previousScrollHeight = container?.scrollHeight || 0
  const previousScrollTop = container?.scrollTop || 0

  isRevealingEarlierTaskGroup.value = true
  emit('reveal-earlier-messages', () => {
    nextTick(() => {
      if (container) {
        const nextScrollHeight = container.scrollHeight
        container.scrollTop = previousScrollTop + (nextScrollHeight - previousScrollHeight)
      }
      isRevealingEarlierTaskGroup.value = false
    })
  })
}

const isHiddenSystemObservation = message => {
  const uiVisibility = message?.metadata?.ui_visibility
  if (uiVisibility === 'hide') return true
  if (message?.metadata?.message_kind === 'runtime_observation') {
    return false
  }
  if (message?.metadata?.error_type === 'SubAgentInterrupted') return true
  if (message?.role !== 'user') return false
  if ((message.stepType || '').toLowerCase() !== 'observe') return false
  if (getAskUserResponseItems(message).length > 0) return false
  return props.removeSystemReminder(message.message || '').trim() === ''
}

const getWorkflowMessageKind = message => message?.messageKind
const getWorkflowMessageSubtype = message => message?.messageSubtype

const isManualClearContextMessage = message =>
  message?.role === 'system' &&
  getWorkflowMessageKind(message) === 'summary' &&
  getWorkflowMessageSubtype(message) === 'manual_clear_context'

const isContextSnapshotMessage = message =>
  message?.role === 'system' &&
  getWorkflowMessageKind(message) === 'summary' &&
  !isManualClearContextMessage(message)

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

const jsonSnapshotSectionText = value => {
  if (typeof value === 'string') return value.trim()
  if (Array.isArray(value)) {
    return value
      .map(item => {
        if (typeof item === 'string') return item.trim()
        if (item && typeof item === 'object') {
          return Object.entries(item)
            .map(([key, entry]) => `${key}: ${entry}`)
            .join('\n')
            .trim()
        }
        return String(item || '').trim()
      })
      .filter(Boolean)
      .join('\n')
      .trim()
  }
  if (value && typeof value === 'object') {
    return Object.entries(value)
      .map(([key, entry]) => {
        if (Array.isArray(entry)) {
          const lines = entry
            .map(item => (typeof item === 'string' ? item.trim() : JSON.stringify(item)))
            .filter(Boolean)
          return lines.length ? `${key}:\n${lines.join('\n')}` : ''
        }
        return `${key}: ${entry}`
      })
      .filter(Boolean)
      .join('\n')
      .trim()
  }
  return ''
}

const getContextSnapshotV2Kind = snapshot => {
  if (snapshot?.schema_version !== 2) return ''
  return ['pressure_handoff', 'completed_task_rollup'].includes(snapshot?.kind)
    ? snapshot.kind
    : ''
}

const snapshotEntryText = value => {
  if (typeof value === 'string') return value.trim()
  if (typeof value === 'number') return String(value)
  if (value && typeof value === 'object') return JSON.stringify(value)
  return ''
}

const snapshotMarkdownList = values => {
  if (!Array.isArray(values)) return ''
  return values
    .map(snapshotEntryText)
    .filter(Boolean)
    .map(value => `- ${value}`)
    .join('\n')
}

const getBoundaryOpenItemKindLabel = kind => {
  const labels = {
    verification: 'workflow.contextSnapshot.openItemKinds.verification',
    decision: 'workflow.contextSnapshot.openItemKinds.decision',
    remediation: 'workflow.contextSnapshot.openItemKinds.remediation'
  }
  return labels[kind] ? t(labels[kind]) : String(kind || '')
}

const formatBoundaryOpenItems = items => {
  if (!Array.isArray(items)) return ''
  return items
    .map(item => {
      const summary = typeof item?.summary === 'string' ? item.summary.trim() : ''
      return summary ? `- **${getBoundaryOpenItemKindLabel(item.kind)}:** ${summary}` : ''
    })
    .filter(Boolean)
    .join('\n')
}

const getReviewRoundStatusLabel = review => {
  const status =
    review?.resolution_status || (review?.verdict?.approved === true ? 'approved_review' : 'open')
  const key = {
    approved_review: 'approved',
    verified_by_re_review: 'verified',
    open: 'open'
  }[status]
  return key ? t(`workflow.contextSnapshot.reviewStatus.${key}`) : String(status)
}

const formatReviewRound = review => {
  if (!review || typeof review !== 'object') return ''

  const title = t('workflow.contextSnapshot.reviewRound', {
    id: snapshotEntryText(review.id) || '?',
    status: getReviewRoundStatusLabel(review)
  })
  const verdictSummary = snapshotEntryText(review.verdict?.summary)
  const findings = snapshotMarkdownList(review.findings)
  const requiredFixes = snapshotMarkdownList(review.required_fixes)
  const evidence = snapshotMarkdownList(review.evidence_refs)
  const sections = [
    verdictSummary,
    findings ? `**${t('workflow.contextSnapshot.findings')}**\n${findings}` : '',
    requiredFixes ? `**${t('workflow.contextSnapshot.requiredFixes')}**\n${requiredFixes}` : '',
    evidence ? `**${t('workflow.contextSnapshot.evidence')}**\n${evidence}` : ''
  ].filter(Boolean)

  return [`#### ${title}`, ...sections].join('\n\n')
}

const formatV2ContextSnapshot = snapshot => {
  const kind = getContextSnapshotV2Kind(snapshot)
  if (!kind) return ''

  const sections = [
    kind === 'pressure_handoff'
      ? [t('workflow.contextSnapshot.userDirectives'), snapshotMarkdownList(snapshot.user_directives)]
      : null,
    [t('workflow.contextSnapshot.confirmedFacts'), snapshotMarkdownList(snapshot.confirmed_facts)],
    kind === 'pressure_handoff'
      ? [t('workflow.contextSnapshot.boundaryOpenItems'), formatBoundaryOpenItems(snapshot.boundary_open_items)]
      : [
          t('workflow.contextSnapshot.unresolvedCarryovers'),
          snapshotMarkdownList(snapshot.unresolved_carryovers)
        ],
    [t('workflow.contextSnapshot.completedWork'), snapshotMarkdownList(snapshot.completed_work)],
    [t('workflow.contextSnapshot.fileChanges'), snapshotMarkdownList(snapshot.file_changes)],
    [
      t('workflow.contextSnapshot.constraintsAndGuards'),
      snapshotMarkdownList(snapshot.constraints_and_guards)
    ],
    [
      t('workflow.contextSnapshot.reviewRounds'),
      Array.isArray(snapshot.review_rounds)
        ? snapshot.review_rounds.map(formatReviewRound).filter(Boolean).join('\n\n')
        : ''
    ]
  ].filter(section => section?.[1])

  return sections
    .map(([title, value]) => `### ${title}\n\n${value}`)
    .join('\n\n')
    .trim()
}

const formatContextSnapshotForDisplay = message => {
  const content = getContextSnapshotContent(message)
  if (!content) return content

  if (content.trim().startsWith('{')) {
    try {
      const parsed = JSON.parse(content)
      const v2Display = formatV2ContextSnapshot(parsed)
      if (v2Display) return v2Display

      const sections = [
        ['Overall Goal', jsonSnapshotSectionText(parsed.overall_goal)],
        ['Previous Tasks', jsonSnapshotSectionText(parsed.prev_tasks)],
        ['Key Knowledge', jsonSnapshotSectionText(parsed.key_knowledge)],
        ['Error Log', jsonSnapshotSectionText(parsed.error_log)],
        ['File System State', jsonSnapshotSectionText(parsed.file_system_state)],
        ['Recent Actions', jsonSnapshotSectionText(parsed.recent_actions)],
        ['Task State', jsonSnapshotSectionText(parsed.task_state)]
      ].filter(([, value]) => value)

      if (!sections.length) return content

      return sections
        .map(([title, value]) => `### ${title}\n\n${value}`)
        .join('\n\n')
        .trim()
    } catch {
      return content
    }
  }

  if (!content.includes('<state_snapshot')) return content

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

const getContextSnapshotTitle = message => {
  const content = getContextSnapshotContent(message)

  if (content.trim().startsWith('{')) {
    try {
      const parsed = JSON.parse(content)
      const kind = getContextSnapshotV2Kind(parsed)
      if (kind === 'pressure_handoff') {
        return t('workflow.contextSnapshot.pressureHandoff')
      }
      if (kind === 'completed_task_rollup') {
        return t('workflow.contextSnapshot.completedTaskRollup')
      }

      const taskState = jsonSnapshotSectionText(parsed.task_state)
      if (taskState) {
        return `Context Snapshot · ${taskState.split('\n')[0]}`
      }
    } catch {
      // Fall back to default title.
    }
  }

  return 'Previous Context Snapshot'
}

const getMessageToolName = message => {
  return String(
    message?.metadata?.tool_name ||
      message?.metadata?.tool_call?.name ||
      message?.metadata?.tool_call?.function?.name ||
      ''
  ).toLowerCase()
}

const getToolDetailClass = message => {
  const toolName =
    getMessageToolName(message) || String(message?.toolName || message?.name || '').toLowerCase()
  if (!toolName) return ''
  const normalizedToolName = toolName.replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '')
  return normalizedToolName
}

const getRawToolCallArguments = message => {
  const toolCall = message?.metadata?.tool_call || {}
  return toolCall.function?.arguments ?? toolCall.arguments
}

const getToolCallArguments = message => {
  const rawArgs = getRawToolCallArguments(message)
  if (typeof rawArgs === 'string') {
    try {
      return JSON.parse(rawArgs)
    } catch {
      return null
    }
  }
  return typeof rawArgs === 'object' && rawArgs !== null ? rawArgs : null
}

const isMcpToolMessage = message => isWorkflowMcpTool(getMessageToolName(message))

const formatJsonIfValid = value => {
  if (typeof value !== 'string') {
    try {
      return JSON.stringify(value, null, 2)
    } catch {
      return String(value ?? '')
    }
  }

  try {
    return JSON.stringify(JSON.parse(value), null, 2)
  } catch {
    return value
  }
}

const getFormattedMcpToolArguments = message => {
  if (!isMcpToolMessage(message)) return ''

  const rawArgs = getRawToolCallArguments(message)
  if (rawArgs === undefined || rawArgs === null || rawArgs === '') return ''
  return formatJsonIfValid(rawArgs)
}

const getMcpToolContentForDisplay = message => {
  const content = props.removeSystemReminder(message?.message || '')
  return isMcpToolMessage(message) ? formatJsonIfValid(content) : content
}

const isBashToolCall = message =>
  String(message?.name || getMessageToolName(message)).toLowerCase() === 'bash'

const TOOL_DURATION_EXCLUDED_NAMES = new Set(['ask_user', 'submit_plan', 'submit_result', 'skill'])

const getToolExecutionDurationLabel = message => {
  const toolName = getMessageToolName(message)
  const durationMs = Number(message?.metadata?.duration_ms)
  if (
    TOOL_DURATION_EXCLUDED_NAMES.has(toolName) ||
    toolName.startsWith('todo_') ||
    !Number.isFinite(durationMs) ||
    durationMs < 0
  ) {
    return ''
  }

  const totalSeconds = Math.max(1, Math.floor(durationMs / 1000))
  if (durationMs <= 60_000) return `${totalSeconds}s`

  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return seconds ? `${minutes}m${seconds}s` : `${minutes}m`
}

const getShellExecutionPlan = message => {
  const plan = message?.metadata?.execution_plan
  return plan && typeof plan === 'object' ? plan : null
}

const getShellExecutionRouteLabel = message => {
  const plan = getShellExecutionPlan(message)
  if (!plan) return ''
  const routeKey = String(plan.backend_origin || '').toLowerCase()
  const routeTranslationKey = routeKey
    ? `workflow.approval.executionRoutes.${routeKey}`
    : ''
  const backend = plan.backend ? t(`workflow.approval.executionBackends.${plan.backend}`) : ''
  const route = routeTranslationKey ? t(routeTranslationKey) : ''
  return [backend, route].filter(value => value && value !== routeTranslationKey).join(' · ')
}

const getBashCommand = message => {
  const args = message?.args || getToolCallArguments(message) || {}
  return String(args.command || message?.toolDisplay?.target || message?.target || '').trim()
}

const isDuplicateBashCommandContent = message => {
  if (!isBashToolCall(message)) return false

  const command = getBashCommand(message).replace(/\r\n/g, '\n').trim()
  const rawContent = String(props.removeSystemReminder(message?.message || ''))
    .replace(/\r\n/g, '\n')
    .trim()
  return !!command && rawContent === command
}

const getToolTitleTarget = message => message?.toolDisplay?.target || message?.target || ''

const getHighlightedBashCommand = message => {
  const command = getBashCommand(message)
  if (!command) return ''

  try {
    return hljs.highlight(command, { language: 'bash' }).value
  } catch {
    return command.replace(/[&<>"']/g, char => {
      const entities = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }
      return entities[char]
    })
  }
}

const decodeCompatJsonPayload = value => {
  if (typeof value !== 'string') return value
  const trimmed = value.trim()
  if (!trimmed) return value
  const looksLikeJson =
    trimmed.startsWith('{') ||
    trimmed.startsWith('[') ||
    (trimmed.startsWith('"') && (trimmed.includes('{') || trimmed.includes('[')))
  if (!looksLikeJson) return value

  let current = value
  for (let depth = 0; depth < 2; depth += 1) {
    if (typeof current !== 'string') break
    try {
      current = JSON.parse(current)
    } catch {
      break
    }
  }
  return current
}

const isStructuredDiffPayload = value => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const hasPath =
    typeof value.file_path === 'string' ||
    typeof value.path === 'string' ||
    typeof value.display_path === 'string'
  const hasEditFields =
    value.old_string !== undefined || value.new_string !== undefined || value.content !== undefined
  return hasPath && hasEditFields
}

const normalizeStructuredDiffPayload = value => {
  const decoded = decodeCompatJsonPayload(value)
  if (Array.isArray(decoded)) {
    return isStructuredDiffPayload(decoded[0]) ? decoded[0] : null
  }
  return isStructuredDiffPayload(decoded) ? decoded : null
}

const normalizeDiffPayload = message => {
  const structuredDetails = normalizeStructuredDiffPayload(message?.metadata?.details)
  if (structuredDetails) {
    return structuredDetails
  }

  const toolName = getMessageToolName(message)
  if (
    ['edit_file', 'write_file', 'plan_edit_note', 'plan_write_note', 'submit_plan'].includes(
      toolName
    )
  ) {
    const args = normalizeStructuredDiffPayload(getToolCallArguments(message))
    if (args) {
      return args
    }
  }

  return null
}

const getDiffContextData = message => normalizeDiffPayload(message)
const getDiffFilePath = message => {
  const payload = normalizeDiffPayload(message)
  return payload?.display_path || payload?.file_path || payload?.path || ''
}
const getDiffOldContent = message => {
  const payload = normalizeDiffPayload(message)
  return payload?.old_string || ''
}
const getDiffNewContent = message => {
  const payload = normalizeDiffPayload(message)
  return payload?.new_string ?? payload?.content ?? ''
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

const isFinishTaskMessage = message => isWorkflowCompletionMessage(message)

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
  message?.role === 'assistant' && message?.metadata?.message_kind === 'completion_report'

const isThinkOnlyAssistantMessage = message => {
  if (message?.role !== 'assistant') return false
  const content = props.removeSystemReminder(message?.message || '').trim()
  const reasoning = String(message?.reasoning || '').trim()
  return !content && !!reasoning
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

const COLLAPSIBLE_READ_TOOL_NAMES = new Set(['read_file', 'list_dir', 'web_fetch'])
const COLLAPSIBLE_SEARCH_TOOL_NAMES = new Set(['grep', 'glob', 'web_search'])
const COLLAPSIBLE_COMMAND_TOOL_NAMES = new Set(['bash'])
const COLLAPSIBLE_MUTATION_TOOL_NAMES = new Set(['edit_file', 'write_file'])
const NON_COLLAPSIBLE_TOOL_NAMES = new Set([
  'ask_user',
  'submit_plan',
  'complete_workflow',
  'sub_agent_run',
  'sub_agent_stop'
])
const TODO_TOOL_NAMES = new Set(['todo_create', 'todo_list', 'todo_update', 'todo_get'])
const TOOL_GROUP_LABEL_KEYS = {
  bash: 'workflow.toolGroups.runCommand',
  edit_file: 'workflow.toolGroups.editFile',
  glob: 'workflow.toolGroups.fileSearch',
  grep: 'workflow.toolGroups.fileSearch',
  list_dir: 'workflow.toolGroups.readFile',
  read_file: 'workflow.toolGroups.readFile',
  skill: 'workflow.toolGroups.useSkill',
  sub_agent_output: 'workflow.toolGroups.fetchSubAgentResult',
  web_fetch: 'workflow.toolGroups.readWeb',
  web_search: 'workflow.toolGroups.fileSearch',
  write_file: 'workflow.toolGroups.editFile'
}

const isCollapsedToolGroupMessage = message => message?.metadata?.message_kind === 'tool_group'

const getCollapsedToolGroupExpandId = (messages, index) => {
  const first = messages[0]
  const firstId = first?.displayId || first?.id || `tool_group_${index}`
  const firstToolCallId = String(first?.metadata?.tool_call_id || '').trim()
  return `tool_group:${firstToolCallId || firstId}`
}

const getToolGroupThoughtSummary = count => {
  if (!count) return ''
  return t('workflow.toolGroups.thoughtChangeCount', { count })
}

const getToolGroupErrorSummary = count => {
  if (!count) return ''
  return t('workflow.toolGroups.errorCount', { count })
}

const isToolGroupErrorMessage = message => !!(message?.toolDisplay?.isError || message?.isRejected)

const truncateToolGroupText = (value, maxLength = 48) => {
  const text = String(value || '')
    .replace(/\s+/g, ' ')
    .trim()
  if (!text) return ''
  return text.length > maxLength ? `${text.slice(0, maxLength - 3)}...` : text
}

const getReadOnlyToolCategory = toolName => {
  if (COLLAPSIBLE_READ_TOOL_NAMES.has(toolName)) return 'read'
  if (COLLAPSIBLE_SEARCH_TOOL_NAMES.has(toolName)) return 'search'
  return null
}

const getToolGroupLabel = message => {
  const toolName = getMessageToolName(message)
  if (TODO_TOOL_NAMES.has(toolName)) return t('workflow.toolGroups.taskChanges')
  if (isWorkflowMcpTool(toolName)) return t('workflow.toolGroups.callMcp')
  return t(TOOL_GROUP_LABEL_KEYS[toolName] || 'workflow.toolGroups.useTool')
}

const buildToolGroupSummary = messages => {
  const counts = new Map()

  messages.forEach(message => {
    const label = getToolGroupLabel(message)
    if (!label) return
    counts.set(label, (counts.get(label) || 0) + 1)
  })

  return truncateToolGroupText(
    Array.from(counts, ([label, count]) => `${label} x${count}`).join(' · '),
    120
  )
}

const isToolWaitingApproval = message => isApprovalPending(message)

const isCollapsibleToolMessage = message => {
  if (message?.role !== 'tool') return false
  const toolName = getMessageToolName(message)
  return !!toolName && !NON_COLLAPSIBLE_TOOL_NAMES.has(toolName) && !isToolWaitingApproval(message)
}

const isCollapsibleReadOnlyToolMessage = message => {
  const toolName = getMessageToolName(message)
  return (
    isCollapsibleToolMessage(message) &&
    !TODO_TOOL_NAMES.has(toolName) &&
    getReadOnlyToolCategory(toolName) !== null
  )
}

const isCollapsibleTodoToolMessage = message =>
  isCollapsibleToolMessage(message) && TODO_TOOL_NAMES.has(getMessageToolName(message))

const isCollapsibleCommandToolMessage = message =>
  isCollapsibleToolMessage(message) &&
  COLLAPSIBLE_COMMAND_TOOL_NAMES.has(getMessageToolName(message))

const isCollapsibleMutationToolMessage = message =>
  isCollapsibleToolMessage(message) &&
  COLLAPSIBLE_MUTATION_TOOL_NAMES.has(getMessageToolName(message))

const isCollapsibleMcpToolMessage = message => {
  const toolName = getMessageToolName(message)
  return isCollapsibleToolMessage(message) && isWorkflowMcpTool(toolName)
}

const isCollapsibleSubAgentResultToolMessage = message =>
  isCollapsibleToolMessage(message) && getMessageToolName(message) === 'sub_agent_output'

const getCollapsibleToolGroupKind = message => {
  if (isCollapsibleReadOnlyToolMessage(message)) return 'readonly_tools'
  if (isCollapsibleTodoToolMessage(message)) return 'todo_tools'
  if (isCollapsibleCommandToolMessage(message)) return 'command_tools'
  if (isCollapsibleMutationToolMessage(message)) return 'mutation_tools'
  if (isCollapsibleSubAgentResultToolMessage(message)) return 'sub_agent_result_tools'
  if (isCollapsibleMcpToolMessage(message)) return 'mcp_tools'
  return isCollapsibleToolMessage(message) ? 'other_tools' : null
}

const getToolCallId = call => String(call?.id || call?.tool_call_id || '').trim()

const buildPendingToolGroupItem = (call, groupOrder) => {
  const toolCallId = getToolCallId(call)
  const toolName = String(call?.toolName || '').trim()
  const argumentsValue = call?.arguments ?? {}

  return {
    id: `pending_tool:${toolCallId}`,
    groupOrder,
    displayId: `pending_tool:${toolCallId}`,
    role: 'tool',
    message: '',
    isRejected: !!call?.isRejected,
    isApproved: false,
    metadata: {
      tool_call_id: toolCallId,
      tool_name: toolName,
      tool_call: {
        id: toolCallId,
        function: {
          name: toolName,
          arguments: argumentsValue
        }
      },
      approval_status: 'approved',
      execution_status: 'running'
    },
    toolDisplay: {
      icon: call?.icon || 'tool',
      toolType: call?.toolType || 'tool-system',
      action: call?.action || toolName,
      target: call?.target || '',
      summary: call?.summary || '',
      isError: !!call?.isRejected
    }
  }
}

const projectPendingToolGroups = messages => {
  const projected = []

  messages.forEach((message, index) => {
    const pendingCalls = Array.isArray(message?.pendingToolCalls) ? message.pendingToolCalls : []
    const groupedTools = pendingCalls
      .map((call, callIndex) =>
        buildPendingToolGroupItem(
          call,
          call.groupOrder ?? index + (callIndex + 1) / (pendingCalls.length + 1)
        )
      )
      .filter(tool => getCollapsibleToolGroupKind(tool))

    if (groupedTools.length === 0) {
      projected.push(message)
      return
    }

    const groupedToolIds = new Set(groupedTools.map(tool => tool.metadata.tool_call_id))
    const remainingPendingCalls = pendingCalls.filter(call => !groupedToolIds.has(getToolCallId(call)))
    const hasAssistantContent = Boolean(
      String(message?.message || '').trim() || String(message?.reasoning || '').trim()
    )

    if (message?.role !== 'assistant' || hasAssistantContent || remainingPendingCalls.length > 0) {
      projected.push({
        ...message,
        pendingToolCalls: remainingPendingCalls
      })
    }

    projected.push(buildToolGroupMessage(groupedTools, index, [], true))
  })

  return projected
}

const TOOL_GROUP_ICONS = {
  command_tools: 'bash',
  mcp_tools: 'mcp',
  mutation_tools: 'edit',
  readonly_tools: 'search',
  sub_agent_result_tools: 'skill-relation-chart',
  todo_tools: 'todo'
}

const getToolGroupIcon = (kind, messages) => {
  if (messages.some(isCollapsibleMutationToolMessage)) return TOOL_GROUP_ICONS.mutation_tools
  return TOOL_GROUP_ICONS[kind] || 'tool'
}

const buildToolGroupMessage = (messages, index, thoughts = [], isOngoing = false) => {
  const kinds = new Set(messages.map(getCollapsibleToolGroupKind).filter(Boolean))
  const [singleKind] = kinds
  const kind = messages.length > 0 && kinds.size === 1 ? singleKind : 'mixed_tools'
  const thoughtCount = thoughts.length
  const errorCount = messages.filter(isToolGroupErrorMessage).length
  const seedMessage = messages[0] || thoughts[0] || {}
  const firstActivity = [...thoughts, ...messages].reduce((earliest, item) => {
    if (!earliest) return item
    return (item?.groupOrder ?? index) < (earliest?.groupOrder ?? index) ? item : earliest
  }, null)

  return {
    ...seedMessage,
    role: 'assistant',
    displayId: getCollapsedToolGroupExpandId([firstActivity || seedMessage], index),
    metadata: {
      ...(seedMessage?.metadata || {}),
      message_kind: 'tool_group',
      tool_group_kind: kind,
      tool_group_thought_count: thoughtCount,
      tool_group_is_ongoing: isOngoing
    },
    groupDisplay: {
      icon: getToolGroupIcon(kind, messages),
      thoughtSummary: getToolGroupThoughtSummary(thoughtCount),
      errorSummary: getToolGroupErrorSummary(errorCount),
      summary: buildToolGroupSummary(messages)
    },
    groupedThoughts: thoughts,
    groupedTools: messages
  }
}

const isToolGroupBoundaryMessage = message => {
  if (!message) return true
  if (isCollapsedToolGroupMessage(message)) return false
  if (message.role === 'user') return true
  if (isContextSnapshotMessage(message) || isManualClearContextMessage(message)) return true
  if (isCompletionReportMessage(message) || isFinishTaskMessage(message)) return true
  if (isExplorationBatchMessage(message)) return true
  if (message.role === 'assistant') {
    return (
      !isThinkOnlyAssistantMessage(message) && !!props.removeSystemReminder(message?.message || '').trim()
    )
  }
  if (message.role === 'tool') {
    if (!getCollapsibleToolGroupKind(message)) return true
  }
  return false
}

const isToolGroupOngoingBoundaryMessage = message => {
  if (!message) return false
  if (isCollapsedToolGroupMessage(message)) return false
  if (message.role === 'user') return true
  if (isContextSnapshotMessage(message) || isManualClearContextMessage(message)) return true
  if (isCompletionReportMessage(message) || isFinishTaskMessage(message)) return true
  if (isExplorationBatchMessage(message)) return true
  if (message.role === 'assistant') {
    return (
      !isThinkOnlyAssistantMessage(message) && !!props.removeSystemReminder(message?.message || '').trim()
    )
  }
  if (message.role === 'tool') {
    if (isApprovalPending(message)) return true
    if (!getCollapsibleToolGroupKind(message)) return true
  }
  return false
}

const hasOngoingToolGroupAfter = (messages, startIndex) => {
  for (let index = startIndex; index < messages.length; index += 1) {
    if (isToolGroupOngoingBoundaryMessage(messages[index])) return false
  }
  return true
}

const isStandaloneOngoingThoughtRun = (messages, startIndex) => {
  for (let index = startIndex; index < messages.length; index += 1) {
    const message = messages[index]
    if (isToolGroupOngoingBoundaryMessage(message)) return false
    if (getCollapsibleToolGroupKind(message) || isCollapsedToolGroupMessage(message)) return false
    if (!isThinkOnlyAssistantMessage(message)) return false
  }
  return true
}

const buildGroupedThoughtItem = (message, index, order = index) => ({
  ...message,
  displayId: `${message?.displayId || message?.id || `thought_${index}`}:tool_group_thought`,
  groupOrder: order,
  sourceMessage: message
})

const getToolActivitySeedKind = message =>
  isThinkOnlyAssistantMessage(message) ||
  getCollapsibleToolGroupKind(message) ||
  isCollapsedToolGroupMessage(message)

const collectContiguousToolMessages = (messages, startIndex) => {
  const tools = []
  let nextIndex = startIndex
  while (nextIndex < messages.length && getCollapsibleToolGroupKind(messages[nextIndex])) {
    tools.push(messages[nextIndex])
    nextIndex += 1
  }
  return { tools, nextIndex }
}

const collectToolActivityMessage = (message, index, thoughts, tools) => {
  if (isThinkOnlyAssistantMessage(message)) {
    thoughts.push(buildGroupedThoughtItem(message, index, index))
    return
  }

  if (isCollapsedToolGroupMessage(message)) {
    ;(message.groupedThoughts || []).forEach((thought, thoughtIndex) => {
      thoughts.push(buildGroupedThoughtItem(thought, `${index}_${thoughtIndex}`, index + thoughtIndex / 1000))
    })
    ;(message.groupedTools || []).forEach((tool, toolIndex) => {
      if (getCollapsibleToolGroupKind(tool)) {
        tools.push({ ...tool, groupOrder: tool.groupOrder ?? index + (toolIndex + 1) / 1000 })
      }
    })
    return
  }

  if (getCollapsibleToolGroupKind(message)) {
    tools.push({ ...message, groupOrder: message.groupOrder ?? index })
  }
}

const collapseToolActivityGroups = messages => {
  const collapsed = []

  for (let index = 0; index < messages.length; ) {
    const current = messages[index]

    if (getToolActivitySeedKind(current)) {
      if (getCollapsibleToolGroupKind(current)) {
        const contiguousGroup = collectContiguousToolMessages(messages, index)
        const nextMessage = messages[contiguousGroup.nextIndex]
        if (!isThinkOnlyAssistantMessage(nextMessage) && !isCollapsedToolGroupMessage(nextMessage)) {
          collapsed.push(
            buildToolGroupMessage(
              contiguousGroup.tools,
              index,
              [],
              hasOngoingToolGroupAfter(messages, contiguousGroup.nextIndex)
            )
          )
          index = contiguousGroup.nextIndex
          continue
        }
      }

      const thoughts = []
      const tools = []
      let nextIndex = index
      const currentIsPendingThought = isThinkOnlyAssistantMessage(current)

      if (currentIsPendingThought && isStandaloneOngoingThoughtRun(messages, index)) {
        while (
          nextIndex < messages.length &&
          isThinkOnlyAssistantMessage(messages[nextIndex]) &&
          isStandaloneOngoingThoughtRun(messages, nextIndex)
        ) {
          collectToolActivityMessage(messages[nextIndex], nextIndex, thoughts, tools)
          nextIndex += 1
        }
        collapsed.push(buildToolGroupMessage([], index, thoughts, true))
        index = nextIndex
        continue
      }

      while (nextIndex < messages.length && !isToolGroupBoundaryMessage(messages[nextIndex])) {
        collectToolActivityMessage(messages[nextIndex], nextIndex, thoughts, tools)
        nextIndex += 1
      }

      if (tools.length > 0) {
        collapsed.push(
          buildToolGroupMessage(tools, index, thoughts, hasOngoingToolGroupAfter(messages, nextIndex))
        )
        index = nextIndex
        continue
      }
    }

    collapsed.push(current)
    index += 1
  }

  return collapsed
}

const visibleMessages = computed(() =>
  excludeLeadingManualClearContextMarkers(
    collapseToolActivityGroups(
      projectPendingToolGroups(
        collapseAssistantCompletionPairs(
          collapseRepeatedFinishTaskErrors(
            props.messages.filter(
              message => !isHiddenSystemObservation(message) || isManualClearContextMessage(message)
            )
          )
        )
      )
    )
  )
)
const lastVisibleMessage = computed(
  () => visibleMessages.value[visibleMessages.value.length - 1] || null
)
const hasStreamingThoughtOnly = computed(
  () =>
    props.isChatting &&
    !String(props.chatState?.content || '').trim() &&
    !!String(props.chatState?.reasoning || '').trim()
)
const isStreamingThoughtMergedIntoToolGroup = message =>
  hasStreamingThoughtOnly.value &&
  message === lastVisibleMessage.value &&
  isCollapsedToolGroupMessage(message) &&
  (message.groupedTools?.length || 0) > 0
const getCollapsedToolGroupThoughtSummary = message => {
  const groupedThoughtCount = Array.isArray(message?.groupedThoughts)
    ? message.groupedThoughts.length
    : Number(message?.metadata?.tool_group_thought_count || 0)
  const streamingThoughtCount = isStreamingThoughtMergedIntoToolGroup(message) ? 1 : 0
  return getToolGroupThoughtSummary(groupedThoughtCount + streamingThoughtCount)
}
const getStreamingThoughtGroupOrder = message => {
  const groupedItems = [...(message?.groupedThoughts || []), ...(message?.groupedTools || [])]
  const latestOrder = Math.max(
    -1,
    ...groupedItems.map(item => Number(item?.groupOrder)).filter(Number.isFinite)
  )
  return latestOrder + 1
}
const shouldShowStandaloneStreamingChat = computed(
  () =>
    props.isChatting &&
    !!(props.chatState?.content || props.chatState?.reasoning) &&
    !isStreamingThoughtMergedIntoToolGroup(lastVisibleMessage.value)
)
const isReasoningExpandedForMessage = message => {
  const messageId = String(message?.displayId || message?.id || '')
  return !!messageId && props.isReasoningExpanded(messageId)
}
const toggleReasoningForMessage = message => {
  const messageId = String(message?.displayId || message?.id || '')
  if (messageId) emit('toggle-reasoning', messageId)
}
const pendingApprovalIdSet = computed(() => {
  const ids = (props.pendingApprovalIds || []).map(id => String(id || '').trim()).filter(Boolean)
  return new Set(ids)
})
const getVisibleMessageIndex = message =>
  visibleMessages.value.findIndex(item => item.displayId === message?.displayId)
const getMessageToolCallId = message => String(message?.metadata?.tool_call_id || '').trim()

const hasSubsequentVisibleOutput = message => {
  const index = getVisibleMessageIndex(message)
  if (index === -1) return false

  return visibleMessages.value.slice(index + 1).some(item => item.role !== 'user')
}

const hasStreamingThoughtCompleted = computed(
  () => props.chatState?.reasoningStatus === 'done'
)

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

const isApprovalPending = message =>
  isWorkflowMessagePendingApproval(message, pendingApprovalIdSet.value)

const isApprovalInFlight = message =>
  !!props.isApprovalSubmitting(props.currentWorkflowId, message?.metadata?.tool_call_id)

const isToolAwaitingExecution = message => {
  const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
  const hasApprovedSubmission =
    approvedSubmissionIds.has(toolCallId) && isApprovalInFlight(message)
  return isWorkflowToolAwaitingExecution(message, hasApprovedSubmission)
}

const isToolMessageExpanded = message =>
  isApprovalPending(message) || props.isMessageExpanded(message)

const TOOL_GROUP_RUNNING_STATUSES = new Set([
  'running',
  'pending',
  'queued',
  'waiting',
  'awaiting_execution',
  'approval_submitted'
])

const isToolGroupItemRunning = tool => {
  const executionStatus = String(tool?.metadata?.execution_status || '').toLowerCase()
  if (TOOL_GROUP_RUNNING_STATUSES.has(executionStatus)) return true
  if (isToolAwaitingExecution(tool)) return true
  const toolCallId = String(tool?.metadata?.tool_call_id || '').trim()
  return !!toolCallId && approvedSubmissionIds.has(toolCallId) && isApprovalInFlight(tool)
}

const isCollapsedToolGroupRunning = message => {
  if (message?.metadata?.tool_group_is_ongoing) return true
  return Array.isArray(message?.groupedTools) && message.groupedTools.some(isToolGroupItemRunning)
}

const getToolSummaryText = message =>
  isToolAwaitingExecution(message)
    ? t('workflow.awaitingExecution')
    : message?.toolDisplay?.summary || ''

const shouldShowApprovalDialog = message =>
  shouldShowInlineApprovalForMessage({
    message,
    isPending: isApprovalPending(message),
    isSubmitting: isApprovalInFlight(message)
  })

const shouldShowRunningPlaceholder = message => {
  const meta = message?.metadata || {}
  const toolCallId = String(meta.tool_call_id || '').trim()
  if (!toolCallId) return false

  const executionStatus = String(meta.execution_status || '').toLowerCase()
  if (!isToolAwaitingExecution(message) && executionStatus !== 'running') return false
  if (workflowStore.getToolStream(toolCallId).length > 0) return false
  if (props.shouldShowToolRawContent(message)) return false
  return true
}

const getRunningPlaceholderText = message =>
  isToolAwaitingExecution(message)
    ? t('workflow.awaitingExecution')
    : message?.toolDisplay?.summary || t('workflow.executing')

const shouldShowErrorAlert = message => {
  if (!message?.isError) return false
  if (message?.role === 'tool') return false
  return !!getErrorAlertContent(message)
}

const getErrorAlertTitle = message => {
  const rawContent = props.removeSystemReminder(message?.message || '').trim()
  if (/^critical error:/i.test(rawContent)) {
    return 'Critical Error'
  }

  const rawType = String(message?.metadata?.error_type || message?.errorType || '').trim()
  if (rawType) {
    return rawType.replace(/([a-z0-9])([A-Z])/g, '$1 $2')
  }

  return t('common.error') || 'Error'
}

const getErrorAlertContent = message => {
  const parsed = props.getParsedMessage(message)
  const rawContent = String(
    parsed?.content || props.removeSystemReminder(message?.message || '')
  ).trim()

  return rawContent
    .replace(/^critical error:\s*/i, '')
    .replace(/^\[?error\]?:\s*/i, '')
    .trim()
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

const sanitizeReasoningContent = content =>
  String(content || '')
    .replace(/^\s*<(?:think|thinking)(?:\s+class="[^"]*")?>\s*/i, '')
    .replace(/\s*<\/(?:think|thinking)>\s*$/i, '')

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

const getVisiblePendingApprovalIds = () => {
  const orderedIds = []
  const seen = new Set()

  for (const message of visibleMessages.value || []) {
    const toolCallId = getMessageToolCallId(message)
    if (!toolCallId || seen.has(toolCallId)) continue
    if (!isApprovalPending(message)) continue
    if (!shouldShowApprovalDialog(message)) continue

    seen.add(toolCallId)
    orderedIds.push(toolCallId)
  }

  return orderedIds
}

const inlineBulkApprovalCount = computed(() => getVisiblePendingApprovalIds().length)

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

const onApproveTool = toolCallId => {
  if (toolCallId) approvedSubmissionIds.add(toolCallId)
  emit('approve-tool', toolCallId)
}

const onApproveAllTool = toolCallId => {
  if (toolCallId) approvedSubmissionIds.add(toolCallId)
  emit('approve-all-tool', toolCallId)
}

const onRejectTool = toolCallId => {
  approvedSubmissionIds.delete(toolCallId)
  emit('reject-tool', toolCallId, getApprovalDraft(toolCallId))
}

const onApproveAllPending = toolCallId => {
  for (const pendingToolCallId of getVisiblePendingApprovalIds()) {
    approvedSubmissionIds.add(pendingToolCallId)
  }
  emit('approve-all-pending', {
    startingToolCallId: toolCallId,
    orderedToolCallIds: getVisiblePendingApprovalIds()
  })
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

const getFinishTaskUsageSummary = message => {
  if (!message?.isApproved || message?.toolDisplay?.isError) return null
  return normalizeUsageSummary(message?.metadata?.usage_summary)
}

const getFinishTaskCostExpandId = message => `${message?.displayId || message?.id || 'finish'}:cost`

const isFinishTaskCostExpanded = message =>
  props.isMessageExpanded({
    displayId: getFinishTaskCostExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const getVisibleUserContent = message => props.removeSystemReminder(message?.message || '')

const getUserMessageExpandId = message => `${message?.displayId || message?.id || 'user'}:user`

const getUserMessageCollapsedMaxHeight = el => {
  if (!el || typeof window === 'undefined') return 0

  const styles = window.getComputedStyle(el)
  const wrapperStyles = window.getComputedStyle(el.parentElement || el)
  const fontSize = Number.parseFloat(styles.fontSize) || 14
  const lineHeight =
    Number.parseFloat(styles.lineHeight) ||
    Number.parseFloat(styles.getPropertyValue('--user-message-line-height-multiplier')) *
      fontSize ||
    fontSize * 1.6
  const safeBottom =
    Number.parseFloat(wrapperStyles.getPropertyValue('--user-message-toggle-safe-bottom')) || 0

  return lineHeight * USER_MESSAGE_COLLAPSED_LINE_COUNT + safeBottom
}

const getUserMessageNaturalHeight = el => {
  if (!el || typeof window === 'undefined' || typeof document === 'undefined') return 0

  const styles = window.getComputedStyle(el)
  const wrapperStyles = window.getComputedStyle(el.parentElement || el)
  const safeRight =
    Number.parseFloat(wrapperStyles.getPropertyValue('--user-message-toggle-safe-right')) || 0
  const measureEl = document.createElement('pre')
  measureEl.textContent = el.textContent || ''
  measureEl.style.position = 'absolute'
  measureEl.style.visibility = 'hidden'
  measureEl.style.pointerEvents = 'none'
  measureEl.style.zIndex = '-1'
  measureEl.style.margin = '0'
  measureEl.style.padding = '0'
  measureEl.style.border = '0'
  measureEl.style.maxHeight = 'none'
  measureEl.style.overflow = 'visible'
  measureEl.style.whiteSpace = styles.whiteSpace
  measureEl.style.wordBreak = styles.wordBreak
  measureEl.style.font = styles.font
  measureEl.style.lineHeight = styles.lineHeight
  measureEl.style.letterSpacing = styles.letterSpacing
  measureEl.style.boxSizing = styles.boxSizing
  measureEl.style.display = 'block'
  measureEl.style.width = `${Math.max(el.clientWidth - safeRight, 0)}px`

  document.body.appendChild(measureEl)
  const naturalHeight = measureEl.scrollHeight
  document.body.removeChild(measureEl)

  return naturalHeight
}

const updateUserMessageOverflowMap = overflowMap => {
  const current = userMessageOverflowMap.value
  const currentKeys = Object.keys(current)
  const nextKeys = Object.keys(overflowMap)

  if (
    currentKeys.length === nextKeys.length &&
    nextKeys.every(key => current[key] === overflowMap[key])
  ) {
    return
  }

  userMessageOverflowMap.value = overflowMap
}

const updateUserMessageCollapsedHeightMap = heightMap => {
  const current = userMessageCollapsedHeightMap.value
  const currentKeys = Object.keys(current)
  const nextKeys = Object.keys(heightMap)

  if (
    currentKeys.length === nextKeys.length &&
    nextKeys.every(key => current[key] === heightMap[key])
  ) {
    return
  }

  userMessageCollapsedHeightMap.value = heightMap
}

const measureUserMessageOverflow = () => {
  const overflowMap = {}
  const collapsedHeightMap = {}
  const container = messagesRef.value
  if (!container) {
    updateUserMessageOverflowMap(overflowMap)
    updateUserMessageCollapsedHeightMap(collapsedHeightMap)
    return
  }

  const elements = container.querySelectorAll('[data-user-expand-id]')
  for (const el of elements) {
    const expandId = el.getAttribute('data-user-expand-id')
    if (!expandId) continue

    const collapsedMaxHeight = getUserMessageCollapsedMaxHeight(el)
    overflowMap[expandId] = getUserMessageNaturalHeight(el) > collapsedMaxHeight
    collapsedHeightMap[expandId] = collapsedMaxHeight > 0 ? `${collapsedMaxHeight}px` : undefined
  }

  updateUserMessageOverflowMap(overflowMap)
  updateUserMessageCollapsedHeightMap(collapsedHeightMap)
}

const scheduleMeasureUserMessageOverflow = () => {
  if (componentUnmounted || userMessageMeasureScheduled) return
  userMessageMeasureScheduled = true

  nextTick(() => {
    if (componentUnmounted) {
      userMessageMeasureScheduled = false
      return
    }
    if (typeof requestAnimationFrame === 'undefined') {
      userMessageMeasureScheduled = false
      measureUserMessageOverflow()
      return
    }

    userMessageMeasureFrameId = requestAnimationFrame(() => {
      if (componentUnmounted) return
      userMessageMeasureFrameId = requestAnimationFrame(() => {
        userMessageMeasureScheduled = false
        userMessageMeasureFrameId = null
        if (componentUnmounted) return
        measureUserMessageOverflow()
      })
    })
  })
}

const isUserMessageExpanded = message =>
  props.isMessageExpanded({
    displayId: getUserMessageExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const isExpandableUserMessage = message => {
  if (!message || getAskUserResponseItems(message).length > 0) return false
  return !!userMessageOverflowMap.value[getUserMessageExpandId(message)]
}

const getUserMessageCollapsedStyle = message => {
  if (!message || isUserMessageExpanded(message)) return undefined

  const maxHeight = userMessageCollapsedHeightMap.value[getUserMessageExpandId(message)]
  return maxHeight ? { maxHeight } : undefined
}

const getMessageSubAgentId = message => {
  const meta = message?.metadata || {}
  if (meta.sub_agent_id || meta.subAgentId) return meta.sub_agent_id || meta.subAgentId
  return meta.data?.sub_agent_id || meta.data?.subAgentId || null
}

const getChoiceGroups = message =>
  props.parseChoiceContent(props.removeSystemReminder(message.message || '')).groups || []

// Keep delegated-task rendering aligned with the centralized projection rule.
// Final review pending is persisted on the completion-tool observation rather
// than a literal `sub_agent_run` row, so card visibility must not be narrowed
// locally during future template refactors.
const isSubAgentRunMessage = message => shouldRenderSubAgentCard(message)

const getSubAgentCardExpandId = message => `${message?.displayId || message?.id || 'sub-agent'}:card`
const getSubAgentCostExpandId = message => `${message?.displayId || message?.id || 'sub-agent'}:cost`

const isSubAgentCardExpanded = message =>
  props.isMessageExpanded({
    displayId: getSubAgentCardExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const getSubAgentUsageSummary = message => {
  const card = message?.subAgentCard
  const terminal = ['completed', 'failed', 'cancelled', 'interrupted'].includes(
    String(card?.status || '').toLowerCase()
  )
  return terminal ? card?.usageSummary || null : null
}

const isSubAgentCostExpanded = message =>
  props.isMessageExpanded({
    displayId: getSubAgentCostExpandId(message),
    metadata: {},
    toolDisplay: {}
  })

const getSubAgentStatusLabel = message => {
  const status = String(message?.subAgentCard?.status || 'running').toLowerCase()
  if (['completed', 'success'].includes(status)) return t('workflow.subAgent.statusCompleted')
  if (['failed', 'error'].includes(status)) return t('workflow.subAgent.statusFailed')
  if (['cancelled', 'interrupted', 'stopped'].includes(status)) {
    return t('workflow.subAgent.statusCancelled')
  }
  return t('workflow.subAgent.statusRunning')
}

const subagentIsRunning = message =>
  String(message?.subAgentCard?.status || '').toLowerCase() === 'running'

const getSubAgentLiveContext = message => {
  const card = message?.subAgentCard || {}
  const hasContextPercent =
    card.contextPercent !== null && card.contextPercent !== undefined && card.contextPercent !== ''
  const contextPercent = hasContextPercent ? Number(card.contextPercent) : NaN

  if (Number.isFinite(contextPercent) && contextPercent >= 0) {
    return `${Math.min(contextPercent, 100)}% ctx`
  }

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

const getSubAgentStatusBadgeClass = message => {
  const status = String(message?.subAgentCard?.status || 'running').toLowerCase()
  if (['completed', 'success'].includes(status)) {
    return 'tool-group__thought-count sub-agent-card__toggle-status--completed'
  }
  if (['failed', 'error'].includes(status)) return 'tool-group__error-count'
  if (['cancelled', 'interrupted', 'stopped'].includes(status)) {
    return 'tool-group__thought-count sub-agent-card__toggle-status--cancelled'
  }
  return 'tool-group__thought-count'
}

const getSubAgentResultPreview = message => {
  const result = props
    .removeSystemReminder(message?.subAgentCard?.result || '')
    .replace(/\s+/g, ' ')
  if (!result) return ''
  return result.length > 96 ? `${result.slice(0, 96)}...` : result
}

const copySubAgentContent = async content => {
  try {
    await writeClipboard(String(content ?? ''))
    showMessage(t('common.copied'), 'success')
  } catch (error) {
    console.error('Failed to copy sub-agent content:', error)
    showMessage(t('common.operationFailed', { error: String(error) }), 'error')
  }
}

const copySubAgentTask = message => copySubAgentContent(message?.subAgentCard?.task)
const copySubAgentResult = message => copySubAgentContent(message?.subAgentCard?.result)

const getSubAgentTaskExpandId = message => `${message?.displayId || message?.id || ''}:task`
const getSubAgentResultExpandId = message => `${message?.displayId || message?.id || ''}:result`

const isSubAgentTaskExpanded = message => {
  return props.isMessageExpanded({
    displayId: getSubAgentTaskExpandId(message),
    metadata: {},
    toolDisplay: {}
  })
}

const isSubAgentResultExpanded = message => {
  return props.isMessageExpanded({
    displayId: getSubAgentResultExpandId(message),
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

const getMessageIdentity = message => ({
  toolCallId: String(message?.metadata?.tool_call_id || '').trim(),
  displayId: String(message?.displayId || '').trim(),
  id: String(message?.id || '').trim()
})

const isSameMessageIdentity = (left, right) => {
  if (!left || !right) return false

  const leftIdentity = getMessageIdentity(left)
  const rightIdentity = getMessageIdentity(right)

  if (leftIdentity.toolCallId && rightIdentity.toolCallId) {
    return leftIdentity.toolCallId === rightIdentity.toolCallId
  }
  if (leftIdentity.displayId && rightIdentity.displayId) {
    return leftIdentity.displayId === rightIdentity.displayId
  }
  if (leftIdentity.id && rightIdentity.id) {
    return leftIdentity.id === rightIdentity.id
  }

  return left === right
}

const isAskUserToolMessage = message => {
  return message?.role === 'tool' && getMessageToolName(message) === 'ask_user'
}

const latestPendingAskUserMessage = computed(() => {
  for (let i = props.messages.length - 1; i >= 0; i -= 1) {
    const message = props.messages[i]
    if (!isAskUserToolMessage(message)) continue
    if (!getChoiceGroups(message).length) continue
    return message
  }
  return null
})

const isAskUserWaitActive = computed(() => props.waitReason === 'user_input')

const canAnswerAskUser = message => {
  if (!getChoiceGroups(message).length) return false
  if (!isAskUserWaitActive.value) return false

  const latestMessage = latestPendingAskUserMessage.value
  if (!latestMessage) return false

  return isSameMessageIdentity(message, latestMessage)
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

const performScrollToBottom = (force = false, frameBudget = 3) => {
  if (componentUnmounted) return
  const el = messagesRef.value
  if (!el) return
  if (!force && !shouldAutoScroll.value && !isNearBottom(el)) return

  pendingScrollForce = pendingScrollForce || force
  pendingScrollFrameBudget = Math.max(pendingScrollFrameBudget, frameBudget)
  if (scrollScheduled) return
  scrollScheduled = true

  nextTick(() => {
    if (componentUnmounted) {
      scrollScheduled = false
      return
    }
    scrollFrameId = requestAnimationFrame(() => {
      scrollScheduled = false
      scrollFrameId = null
      if (componentUnmounted) return

      const currentEl = messagesRef.value
      const currentForce = pendingScrollForce
      const currentFrameBudget = pendingScrollFrameBudget
      pendingScrollForce = false
      pendingScrollFrameBudget = 0

      if (!currentEl) return
      if (!currentForce && !shouldAutoScroll.value && !isNearBottom(currentEl)) return

      const target = currentEl.scrollHeight - currentEl.clientHeight
      currentEl.scrollTop = Math.max(0, target)
      shouldAutoScroll.value = true

      if (currentFrameBudget <= 1) return

      scrollCorrectionFrameId = requestAnimationFrame(() => {
        scrollCorrectionFrameId = null
        if (componentUnmounted) return
        const remaining = currentEl.scrollHeight - currentEl.scrollTop - currentEl.clientHeight
        if (remaining > 2) {
          performScrollToBottom(true, currentFrameBudget - 1)
        }
      })
    })
  })
}

const scrollToBottom = (force = false) => {
  performScrollToBottom(force)
}

const streamingLayoutState = computed(() => {
  const blocks = Array.isArray(props.chatState?.blocks) ? props.chatState.blocks : []
  const lastBlock = blocks[blocks.length - 1]
  return [
    props.isChatting ? 1 : 0,
    props.chatState?.content?.length || 0,
    props.chatState?.reasoning?.length || 0,
    blocks.length,
    lastBlock?.content?.length || 0,
    props.chatState?.retryInfo?.nextRetryIn ?? ''
  ]
})

watch(
  visibleMessages,
  () => {
    performScrollToBottom()
    scheduleMeasureUserMessageOverflow()
    syncMessageContentResizeObserver()
  },
  { flush: 'post' }
)

watch(
  streamingLayoutState,
  () => {
    performScrollToBottom()
    syncMessageContentResizeObserver()
  },
  { flush: 'post' }
)

watch(
  () => props.currentWorkflowId,
  () => {
    isRevealingEarlierTaskGroup.value = false
    shouldAutoScroll.value = true
    userMessageOverflowMap.value = {}
    userMessageCollapsedHeightMap.value = {}
    scheduleMeasureUserMessageOverflow()
  }
)

const syncMessageContentResizeObserver = () => {
  const container = messagesRef.value
  if (!messageContentResizeObserver || !container) return

  messageContentResizeObserver.disconnect()
  Array.from(container.children).forEach(child => messageContentResizeObserver.observe(child))
}

onMounted(() => {
  componentUnmounted = false
  if (typeof ResizeObserver !== 'undefined') {
    messageContentResizeObserver = new ResizeObserver(() => {
      performScrollToBottom()
    })
    userMessageResizeObserver = new ResizeObserver(entries => {
      const nextWidth = entries[0]?.contentRect?.width || messagesRef.value?.clientWidth || 0
      if (nextWidth === observedMessageListWidth) return
      observedMessageListWidth = nextWidth
      scheduleMeasureUserMessageOverflow()
    })
    if (messagesRef.value) {
      observedMessageListWidth = messagesRef.value.clientWidth
      userMessageResizeObserver.observe(messagesRef.value)
      syncMessageContentResizeObserver()
    }
  } else if (typeof window !== 'undefined') {
    window.addEventListener('resize', scheduleMeasureUserMessageOverflow)
  }

  scheduleMeasureUserMessageOverflow()
})

onBeforeUnmount(() => {
  componentUnmounted = true
  scrollScheduled = false
  userMessageMeasureScheduled = false
  if (userMessageResizeObserver) {
    userMessageResizeObserver.disconnect()
    userMessageResizeObserver = null
  }
  if (messageContentResizeObserver) {
    messageContentResizeObserver.disconnect()
    messageContentResizeObserver = null
  }
  if (typeof ResizeObserver === 'undefined' && typeof window !== 'undefined') {
    window.removeEventListener('resize', scheduleMeasureUserMessageOverflow)
  }
  if (scrollFrameId !== null) {
    cancelAnimationFrame(scrollFrameId)
    scrollFrameId = null
  }
  if (scrollCorrectionFrameId !== null) {
    cancelAnimationFrame(scrollCorrectionFrameId)
    scrollCorrectionFrameId = null
  }
  if (userMessageMeasureFrameId !== null) {
    cancelAnimationFrame(userMessageMeasureFrameId)
    userMessageMeasureFrameId = null
  }
})

defineExpose({
  scrollToBottom,
  messagesRef
})
</script>

<style scoped lang="scss">
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

.workflow-error-alert {
  margin-top: var(--cs-space-sm);
  border: 1px solid var(--el-color-danger-light-5);

  :deep(.el-alert__content) {
    width: 100%;
  }
}

.workflow-error-alert__body {
  margin-top: var(--cs-space-xs);
  white-space: pre-wrap;
  word-break: break-word;
}

.user-message-wrap {
  position: relative;
  --user-message-line-height-multiplier: 1.6;
  --user-message-toggle-size: var(--cs-size-lg);
  --user-message-toggle-right: var(--cs-space);
  --user-message-toggle-bottom: var(--cs-space);
  --user-message-toggle-safe-right: calc(var(--cs-size-xl) + var(--cs-space-sm));
  --user-message-toggle-safe-bottom: calc(var(--cs-size-xl) + var(--cs-space-sm));
  --user-message-collapse-fade-height: 20px;
  --user-message-collapse-fade-inset-x: var(--cs-space-xs);
  --user-message-collapse-fade-radius: var(--cs-border-radius-lg);
}

.user-message-wrap.is-expandable {
  cursor: pointer;
}

.user-message-wrap.is-collapsed::after {
  content: '';
  position: absolute;
  left: var(--user-message-collapse-fade-inset-x);
  right: var(--user-message-collapse-fade-inset-x);
  bottom: 1px;
  height: calc(var(--user-message-collapse-fade-height) + var(--cs-space-xs));
  border-radius: 0 0 var(--user-message-collapse-fade-radius)
    var(--user-message-collapse-fade-radius);
  background: linear-gradient(to top, var(--cs-bg-color-light) 40%, transparent 100%);
  pointer-events: none;
}

.tool-diff-view {
  margin-top: var(--cs-space-xs);
}

.tool-running-placeholder {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
  padding: var(--cs-space-sm) var(--cs-space-md);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-sm);
}

.tool-running-placeholder__icon {
  color: var(--el-color-primary);
  flex-shrink: 0;
}

.tool-running-placeholder__text {
  min-width: 0;
  word-break: break-word;
}

.manual-clear-context-divider {
  display: flex;
  align-items: center;
  gap: var(--cs-space-sm);
  margin: var(--cs-space) 0;
  color: var(--cs-text-color-secondary);
}

.manual-clear-context-divider__line {
  flex: 1;
  height: 1px;
  background: var(--cs-border-color);
}

.manual-clear-context-divider__label {
  flex-shrink: 0;
  font-size: var(--cs-font-size-xs);
  color: var(--cs-text-color-secondary);
}

.workflow-message-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: var(--cs-space-xs);
  margin-bottom: var(--cs-space-sm);
}

.workflow-message-attachment-item {
  display: flex;
}

.workflow-message-attachment-image {
  width: 88px;
  height: 88px;
  border-radius: var(--cs-border-radius-md);
  overflow: hidden;
  border: 1px solid var(--cs-border-color);
}

.history-window-indicator {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--cs-space-xs);
  margin: 0 var(--cs-space) var(--cs-space) 0;
  padding: var(--cs-space-xs) var(--cs-space-sm);
  border: 1px dashed var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  background: var(--cs-bg-color);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  cursor: pointer;
  transition:
    background-color 0.2s ease,
    color 0.2s ease,
    border-color 0.2s ease;

  &:hover {
    color: var(--cs-text-color-primary);
    background: var(--cs-hover-bg-color);
    border-color: var(--el-color-primary-light-5);
  }
}

.queued-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: var(--cs-size-xl);
  height: var(--cs-size-xl);
  border: none;
  border-radius: var(--cs-border-radius);
  background: transparent;
  color: var(--cs-text-color-secondary);
  cursor: pointer;
  transition:
    color 0.2s ease,
    background-color 0.2s ease;
}

.queued-remove:hover {
  color: var(--cs-text-color-primary);
  background: var(--cs-hover-bg-color);
}

.queued-status-text {
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  line-height: 1.4;
}

.queued-list {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-xs);
}

.queued-item {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--cs-space-xs);
  margin-left: auto;
  max-width: min(80%, 720px);
  padding: var(--cs-space-sm) var(--cs-space);
  border-radius: var(--cs-border-radius-lg);
  background: var(--cs-hover-bg-color);
}

.queued-item--processing {
  border: 1px solid var(--el-color-primary-light-5);
  background: var(--cs-bg-color);
}

.queued-item-main {
  display: flex;
  align-items: flex-start;
  gap: var(--cs-space-xs);
  min-width: 0;
  flex: 1;
}

.queued-icon {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--cs-text-color-secondary);
}

.queued-content {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-2xs);
  min-width: 0;
}

.queued-text {
  color: var(--cs-text-color-primary);
  font-size: var(--cs-font-size-sm);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.queued-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: var(--cs-space-xs);
}

.queued-attachment-item {
  flex-shrink: 0;
}

.queued-attachment-image {
  width: 72px;
  height: 72px;
  border-radius: var(--cs-border-radius-md);
  object-fit: cover;
}

.queued-attachment-name {
  display: inline-flex;
  max-width: 180px;
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  line-height: 1.4;
  word-break: break-word;
}

.simple-text {
  margin: 0;
  display: block;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: break-word;
  line-height: calc(1em * var(--user-message-line-height-multiplier));
  max-height: none;
  overflow: visible;
  padding-right: 0;
  padding-bottom: 0;

  &.is-collapsed {
    position: relative;
    overflow: hidden;
    padding-right: var(--user-message-toggle-safe-right);
    padding-bottom: var(--user-message-toggle-safe-bottom);
  }
}

.user-message-toggle {
  position: absolute;
  right: var(--user-message-toggle-right);
  bottom: var(--user-message-toggle-bottom);
  z-index: 1;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  width: var(--cs-size-xl);
  height: var(--cs-size-xl);
  padding: 0;
  border: 0;
  border-radius: var(--cs-border-radius);
  background: var(--cs-bg-color-light);
  color: var(--cs-text-color-secondary);
  cursor: pointer;
}

.user-message-toggle__icon {
  transition: transform 0.2s ease;
}

.user-message-toggle__icon.expanded {
  transform: rotate(180deg);
}

.tool-group__thought-count.sub-agent-card__toggle-status--completed {
  background: color-mix(in srgb, var(--el-color-success) 12%, transparent);
  color: var(--el-color-success);
}

.tool-group__thought-count.sub-agent-card__toggle-status--cancelled {
  background: color-mix(in srgb, var(--cs-text-color-secondary) 12%, transparent);
  color: var(--cs-text-color-secondary);
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

.tool-group__summary {
  align-items: center !important;
  gap: var(--cs-space-xs) !important;
  min-width: 0;
}

.tool-group__thought-count,
.tool-group__error-count {
  flex: 0 0 auto;
  padding: 1px var(--cs-space-xs);
  border-radius: var(--cs-border-radius-sm);
  font-size: var(--cs-font-size-xs);
  font-weight: 600;
  line-height: 1.6;
}

.tool-group__thought-count {
  background: color-mix(in srgb, var(--el-color-primary) 12%, transparent);
  color: var(--el-color-primary);
}

.tool-group__error-count {
  background: color-mix(in srgb, var(--el-color-danger) 12%, transparent);
  color: var(--el-color-danger);
}

.tool-group__summary-text {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-group__expand-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: var(--cs-size-md);
  height: var(--cs-size-md);
  margin-left: 10px;
  padding: 0;
  border: 0;
  border-radius: var(--cs-border-radius);
  background: transparent;
  color: var(--cs-text-color-secondary);
  cursor: pointer;
  opacity: 0;
  transition:
    opacity 0.2s ease,
    transform 0.2s ease,
    background-color 0.2s ease;
}

.tool-group__summary:hover .tool-group__expand-button,
.tool-group__summary:focus-within .tool-group__expand-button {
  opacity: 1;
  transform: rotate(-90deg);
}

.tool-group__expand-button:hover {
  background: var(--cs-hover-bg-color);
  transform: rotate(-90deg)
}

.tool-group__summary.expanded .tool-group__expand-button {
  transform: rotate(0deg);
}

.collapsed-tool-group__body {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm);
  margin-bottom: 0;
  padding-top: var(--cs-space-sm);
  animation: tool-expand-enter 0.16s ease-out both;
  will-change: opacity, transform;
}

.shell-execution-route-badge {
  flex: 0 0 auto;
  padding: 2px var(--cs-space-xs);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  white-space: nowrap;
}

.mcp-tool-arguments {
  margin-bottom: var(--cs-space-sm);
}

.mcp-tool-arguments__label {
  margin-bottom: var(--cs-space-2xs);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  font-weight: 600;
}

.tool-detail--expanded {
  animation: tool-expand-enter 0.16s ease-out both;
  will-change: opacity, transform;
}

@keyframes tool-expand-enter {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .collapsed-tool-group__body,
  .tool-detail--expanded {
    animation: none;
    will-change: auto;
  }
}

.collapsed-tool-group__thought .reasoning-content {
  background: none !important;
}

.choice-options--readonly {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-2xs);
}
</style>
