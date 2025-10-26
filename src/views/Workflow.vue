<template>
  <div class="workflow-layout">
    <titlebar>
      <template #left>
        <el-tooltip
          :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)"
          placement="right"
          :hide-after="0"
          :enterable="false">
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
        </el-tooltip>
      </template>
      <template #center> </template>
      <template #right>
        <div class="icon-btn upperLayer pin-btn" @click="onPin" :class="{ active: isAlwaysOnTop }">
          <el-tooltip
            :content="$t(`common.${isAlwaysOnTop ? 'unpin' : 'pin'}`)"
            :hide-after="0"
            :enterable="false"
            placement="bottom">
            <cs name="pin" />
          </el-tooltip>
        </div>
      </template>
    </titlebar>

    <div class="workflow-main">
      <el-aside :width="sidebarWidth" :class="{ collapsed: sidebarCollapsed }" class="sidebar">
        <div class="sidebar-header upperLayer">
          <el-input
            v-model="searchQuery"
            :placeholder="$t('chat.searchChat')"
            :clearable="true"
            round>
            <template #prefix><cs name="search" /></template>
          </el-input>
        </div>
        <div v-show="!sidebarCollapsed" class="workflow-list">
          <div class="list">
            <div
              class="item"
              v-for="wf in filteredWorkflows"
              :key="wf.id"
              @click="selectWorkflow(wf.id)"
              @mouseenter="hoveredWorkflowIndex = wf.id"
              @mouseleave="hoveredWorkflowIndex = null"
              :class="{
                active: wf.id === currentWorkflowId,
                disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId
              }">
              <div class="workflow-title">{{ wf.title || wf.userQuery }}</div>
              <div class="workflow-status" v-if="wf.status">
                <span :class="['status-indicator', wf.status.toLowerCase()]"></span>
                {{ wf.status }}
              </div>
              <div class="icons" v-show="wf.id === hoveredWorkflowIndex">
                <div class="icon icon-edit" @click.stop="onEditWorkflow(wf.id)">
                  <cs name="edit" />
                </div>
                <div class="icon icon-delete" @click.stop="onDeleteWorkflow(wf.id)">
                  <cs name="delete" />
                </div>
              </div>
            </div>
          </div>
        </div>
      </el-aside>

      <!-- main container -->
      <el-container class="main-container">
        <div class="messages" ref="messagesRef">
          <div v-for="message in messages" :key="message.id" class="message" :class="message.role">
            <div class="avatar">
              <cs v-if="message.role === 'user'" name="talk" class="user-icon" />
              <cs v-else name="ai-common" />
            </div>
            <div class="content-container">
              <div class="content" v-if="message.role === 'user'">
                <pre class="simple-text">{{ message.message }}</pre>
              </div>
              <markdown
                v-else
                :content="message.message"
                :tool-calls="message.metadata?.tool_calls || []" />
            </div>
          </div>
        </div>

        <div class="todo-list-wrapper" v-if="todoList.length > 0">
          <TodoList :items="todoList" />
        </div>

        <!-- footer -->
        <el-footer class="input-container">
          <div class="input">
            <el-input
              ref="inputRef"
              v-model="inputMessage"
              type="textarea"
              :autosize="{ minRows: 1, maxRows: 10 }"
              :placeholder="$t('chat.inputMessagePlaceholder', { at: '@' })"
              @keydown.enter="onKeyEnter" />

            <div class="input-footer">
              <div class="footer-left">
                <div class="agent-selector-wrap" :class="{ disabled: currentWorkflowId }">
                  <AgentSelector
                    v-model="selectedAgent"
                    :agent="
                      currentWorkflow?.agentId
                        ? agentStore.agents.find(a => a.id === currentWorkflow.agentId)
                        : null
                    "
                    :disabled="!!currentWorkflowId" />
                </div>
                <div class="icons">
                  <el-tooltip
                    content="Auto-approve tools (excluding interactive tools)"
                    placement="top">
                    <label class="icon-btn upperLayer" :class="{ active: autoApproveTools }">
                      <cs name="tool" class="small" />
                    </label>
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('workflow.newWorkflow')"
                    :hide-after="0"
                    :enterable="false"
                    placement="top">
                    <label @click="createNewWorkflow" :class="{ disabled: isRunning }">
                      <cs name="new-chat" class="small" :class="{ disabled: isRunning }" />
                    </label>
                  </el-tooltip>
                </div>
              </div>
              <div class="icons">
                <cs name="stop" @click="onStop" v-if="isRunning" />
                <cs
                  v-else
                  name="send"
                  @click="onSendMessage"
                  :class="{ disabled: !canSendMessage }" />
              </div>
            </div>
          </div>
        </el-footer>
      </el-container>
    </div>

    <!-- edit workflow dialog -->
    <el-dialog
      v-model="editWorkflowDialogVisible"
      :title="$t('workflow.editWorkflowTitle')"
      :close-on-press-escape="false"
      width="50%">
      <el-form>
        <el-form-item :label="$t('workflow.workflowTitle')">
          <el-input v-model="editWorkflowTitle" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editWorkflowDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="onSaveEditWorkflow">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invokeWrapper } from '@/libs/tauri'

import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'

import Titlebar from '@/components/window/Titlebar.vue'
import Markdown from '@/components/chat/Markdown.vue'
import AgentSelector from '@/components/workflow/AgentSelector.vue'
import TodoList from '@/components/workflow/TodoList.vue'

// Import workflow engine
import { WorkflowEngine } from '@/pkg/workflow/engine'
import { WorkflowState } from '@/pkg/workflow/types'
import { getTodoListForWorkflow } from '@/pkg/workflow/tools/todoList'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

const unlistenFocusInput = ref(null)
const currentEngine = ref(null)
const osType = ref('') // To store OS type from backend
const hoveredWorkflowIndex = ref(null) // For workflow hover effects

// edit workflow dialog
const editWorkflowDialogVisible = ref(false)
const editWorkflowId = ref(null)
const editWorkflowTitle = ref('')

const sidebarCollapsed = ref(!windowStore.workflowSidebarShow)
const sidebarWidth = computed(() => (sidebarCollapsed.value ? '0px' : '200px'))
const searchQuery = ref('')
const inputMessage = ref('')
const selectedAgent = ref(null)
const autoApproveTools = ref(true)
const messagesRef = ref(null)
const inputRef = ref(null)
const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)

const workflows = computed(() => workflowStore.workflows)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const messages = computed(() => workflowStore.messages)
const isRunning = computed(() => workflowStore.isRunning)
const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)

// Get todo list from the todo manager
const todoList = computed(() => {
  if (!currentWorkflowId.value) return []
  return getTodoListForWorkflow(currentWorkflowId.value)
})

const filteredWorkflows = computed(() => {
  if (!searchQuery.value) return workflows.value
  return workflows.value.filter(wf =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.value.toLowerCase())
  )
})

const canSendMessage = computed(
  () => inputMessage.value.trim() !== '' && !isRunning.value && selectedAgent.value
)

const canSwitchWorkflow = computed(() => {
  // Can't switch if a workflow is currently running
  return !isRunning.value
})

// Watch for workflow changes to update UI
watch(currentWorkflow, newWorkflow => {
  if (newWorkflow) {
    // Scroll to bottom when new workflow is selected
    nextTick(() => {
      scrollToBottom()
    })
  }
})

// Watch for messages to scroll to bottom
watch(
  messages,
  () => {
    nextTick(() => {
      scrollToBottom()
    })
  },
  { deep: true }
)

const scrollToBottom = () => {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

onMounted(async () => {
  unlistenFocusInput.value = await listen('cs://workflow-focus-input', event => {
    if (event.payload && event.payload.windowLabel === settingStore.windowLabel) {
      if (inputRef.value) {
        inputRef.value.focus()
      }
    }
  })

  try {
    const osInfo = await invokeWrapper('get_os_info')
    osType.value = osInfo.os
  } catch (error) {
    console.error('Failed to get OS info:', error)
  }

  await workflowStore.loadWorkflows()
  await agentStore.fetchAgents()

  if (agentStore.agents.length > 0) {
    selectedAgent.value = agentStore.agents[0]
  }

  // Load the last workflow if available
  if (workflows.value.length > 0) {
    await selectWorkflow(workflows.value[0].id)
  }

  windowStore.initWorkflowWindowAlwaysOnTop()
  window.addEventListener('keydown', onGlobalKeyDown)
})

onBeforeUnmount(() => {
  if (currentEngine.value) {
    // Clean up current workflow engine if exists
    currentEngine.value = null
  }
  unlistenFocusInput.value()
  window.removeEventListener('keydown', onGlobalKeyDown)
})

const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  windowStore.setWorkflowSidebarShow(!sidebarCollapsed.value)
}

const selectWorkflow = async id => {
  if (!canSwitchWorkflow.value) {
    console.warn('Cannot switch workflow while another is running')
    return
  }

  // Clean up current engine
  currentEngine.value = null

  // Select the workflow in store
  await workflowStore.selectWorkflow(id)

  // Load the workflow engine if it exists
  if (workflowStore.currentWorkflow) {
    const agent = agentStore.agents.find(a => a.id === workflowStore.currentWorkflow.agentId)
    if (agent) {
      // Update the selected agent in the selector
      selectedAgent.value = agent

      try {
        currentEngine.value = await WorkflowEngine.load(agent, id)
      } catch (error) {
        console.error('Failed to load workflow:', error)
      }
    }
  }
}

const startNewWorkflow = async () => {
  if (!selectedAgent.value) {
    console.error('No agent selected')
    return
  }

  try {
    // Clean up current engine
    currentEngine.value = null

    // Start new workflow
    currentEngine.value = await WorkflowEngine.startNew(selectedAgent.value, inputMessage.value)

    // Update store with new workflow
    await workflowStore.loadWorkflows()
    await workflowStore.selectWorkflow(currentEngine.value.sessionId)

    inputMessage.value = ''
  } catch (error) {
    console.error('Failed to start new workflow:', error)
  }
}

const onSendMessage = () => {
  if (!canSendMessage.value) return

  if (!currentWorkflowId.value) {
    // Start new workflow
    startNewWorkflow()
  } else {
    // Add message to current workflow
    workflowStore.addMessageToQueue({ role: 'user', message: inputMessage.value })
    inputMessage.value = ''
  }
}

const onKeyEnter = event => {
  const shouldSend =
    settingStore.settings.sendMessageKey === 'Enter' ? !event.shiftKey : event.shiftKey
  if (shouldSend) {
    event.preventDefault()
    onSendMessage()
  }
}

const onStop = () => {
  if (currentEngine.value) {
    // Pause the workflow
    currentEngine.value.stateMachine.transition('PAUSE')
    workflowStore.setRunning(false)
  }
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
}

const onEditWorkflow = id => {
  editWorkflowId.value = id
  editWorkflowTitle.value = workflows.value.find(wf => wf.id === id)?.title || ''
  editWorkflowDialogVisible.value = true
}

const onSaveEditWorkflow = async () => {
  if (!editWorkflowId.value) return

  try {
    await invokeWrapper('update_workflow_title', {
      workflowId: editWorkflowId.value,
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

const onDeleteWorkflow = id => {
  ElMessageBox.confirm(t('workflow.confirmDeleteWorkflow'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel')
  }).then(async () => {
    try {
      await invokeWrapper('delete_workflow', { workflowId: id })

      // If deleting the current workflow, clear it
      if (id === currentWorkflowId.value) {
        currentEngine.value = null
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

const createNewWorkflow = () => {
  // Clear current workflow
  currentEngine.value = null
  workflowStore.clearCurrentWorkflow()

  // Clear input and focus
  inputMessage.value = ''
  nextTick(() => {
    if (inputRef.value) {
      inputRef.value.focus()
    }
  })
}

const onGlobalKeyDown = event => {
  // Use OS type from backend. `std::env::consts::OS` returns "macos" for macOS.
  const isMac = osType.value === 'macos'
  const modifierPressed = isMac ? event.metaKey : event.ctrlKey

  if (modifierPressed) {
    switch (event.key.toLowerCase()) {
      case 'n':
        event.preventDefault()
        createNewWorkflow()
        break
      case 'b':
        event.preventDefault()
        onToggleSidebar()
        break
    }
  }
}
</script>

<style lang="scss">
.workflow-layout {
  height: 100vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;

  .workflow-main {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: row;

    .sidebar {
      border-right: 1px solid var(--cs-border-color);
      display: flex;
      flex-direction: column;
      height: 100%;
      transition: width 0.3s ease;

      .sidebar-header {
        padding: 10px;
        flex-shrink: 0;

        .el-input {
          box-sizing: border-box;

          .el-input__wrapper {
            padding: 0;
            background: var(--cs-input-bg-color) !important;
            border-radius: var(--cs-border-radius-xxl);
            font-size: var(--cs-font-size-sm);
          }

          .el-input__prefix {
            display: flex;
            align-items: center;
            padding-left: var(--cs-space-sm);

            .cs {
              font-size: var(--cs-font-size-md);
              color: var(--cs-text-color-secondary);
            }
          }
        }
      }

      .workflow-list {
        flex: 1;
        overflow-y: auto;
        height: calc(100% - 60px);

        .list {
          .item {
            padding: 10px 15px;
            cursor: pointer;
            border-radius: 6px;
            margin-bottom: 2px;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
            transition: background-color 0.2s ease;
            display: flex;
            flex-direction: column;
            gap: 4px;
            position: relative;

            &:hover:not(.disabled) {
              background-color: var(--cs-hover-bg-color);
            }

            &.active {
              background-color: var(--cs-active-bg-color);
              color: var(--el-color-primary);
            }

            &.disabled {
              cursor: not-allowed;
              opacity: 0.6;
            }

            .workflow-title {
              font-weight: 500;
              overflow: hidden;
              text-overflow: ellipsis;
            }

            .workflow-status {
              display: flex;
              align-items: center;
              gap: 6px;
              font-size: var(--cs-font-size-xs);
              color: var(--cs-text-color-secondary);

              .status-indicator {
                width: 6px;
                height: 6px;
                border-radius: 50%;

                &.running {
                  background-color: var(--el-color-warning);
                  animation: pulse 1.5s ease-in-out infinite;
                }

                &.completed {
                  background-color: var(--el-color-success);
                }

                &.paused {
                  background-color: var(--el-color-info);
                }

                &.error {
                  background-color: var(--el-color-danger);
                }
              }
            }

            .icons {
              position: absolute;
              right: 10px;
              top: 50%;
              transform: translateY(-50%);
              display: flex;
              gap: 4px;
              opacity: 0;
              transition: opacity 0.2s ease;

              .icon {
                display: flex;
                align-items: center;
                justify-content: center;
                width: 24px;
                height: 24px;
                border-radius: var(--cs-border-radius-round);
                cursor: pointer;
                color: var(--cs-text-color-secondary);

                &:hover {
                  background-color: var(--cs-bg-color-light);
                  color: var(--cs-text-color-primary);
                }

                .cs {
                  font-size: var(--cs-font-size-sm);
                }
              }
            }

            &:hover .icons {
              opacity: 1;
            }
          }
        }
      }
    }

    .main-container {
      display: flex;
      flex-direction: column;
      flex: 1;
      overflow: hidden;
      height: 100%;

      .messages {
        flex: 1;
        overflow-y: auto;
        padding: 15px;
        scroll-behavior: smooth;

        .message {
          display: flex;
          margin-bottom: 15px;

          .avatar {
            flex-shrink: 0;
            width: 30px;
            height: 30px;
            display: flex;
            align-items: center;
            justify-content: center;
            margin-right: 12px;
            margin-top: 2px;

            .user-icon {
              color: var(--el-color-primary);
            }
          }

          .content-container {
            flex: 1;
            min-width: 0;
          }

          &.user {
            flex-direction: row-reverse;

            .avatar {
              margin-right: 0;
              margin-left: 12px;
            }

            .content {
              display: flex;
              justify-content: flex-end;

              .simple-text {
                background-color: var(--cs-bg-color-light);
                padding: 10px 15px;
                border-radius: 10px;
                max-width: 80%;
                border: 1px solid var(--cs-border-color);
                box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
                margin: 0;
              }
            }
          }
        }
      }

      .todo-list-wrapper {
        flex-shrink: 0;
        padding: 0 var(--cs-space) var(--cs-space-sm);
      }

      footer.input-container {
        flex-shrink: 0;
        background-color: transparent;
        padding: 0 var(--cs-space-sm) var(--cs-space-sm);
        height: unset;
        z-index: 1;

        .additional {
          display: flex;
          gap: 1px;
          margin-bottom: var(--cs-space-xs);

          .additional-item {
            display: flex;
            align-items: center;
            flex: 1;
            max-width: 50%;
            background-color: var(--cs-input-bg-color);
            border-radius: var(--cs-border-radius-xxl);
            padding: var(--cs-space-xs);
            box-sizing: border-box;

            .data {
              flex: 1;
              min-width: 0;

              .skill-item {
                padding: 0;
              }

              .message-text {
                padding-left: var(--cs-space);
                display: block;
                white-space: nowrap;
                overflow: hidden;
                text-overflow: ellipsis;
                color: var(--cs-text-color-secondary);
                font-size: var(--cs-font-size-sm);
                line-height: 1.5;
                position: relative;

                &:before {
                  position: absolute;
                  top: -3px;
                  left: 3px;
                }
              }
            }

            .close-btn {
              display: flex;
              align-items: center;
              justify-content: center;
              width: 24px;
              height: 24px;
              margin-left: var(--cs-space-xs);
              flex-shrink: 0;
              cursor: pointer;
              border-radius: var(--cs-border-radius-round);
              color: var(--cs-text-color-secondary);

              &:hover {
                background-color: var(--cs-bg-color-light);
              }
            }
          }
        }

        .input {
          display: flex;
          flex-direction: column;
          background-color: var(--cs-input-bg-color);
          border-radius: var(--cs-border-radius-lg);
          padding: var(--cs-space-sm) var(--cs-space) var(--cs-space-xs);

          .icons {
            display: flex;
            align-items: center;
            justify-content: center;
            padding: var(--cs-space-xs);
            cursor: pointer;
            gap: var(--cs-space-xs);

            .cs {
              font-size: var(--cs-font-size-xl) !important;
              color: var(--cs-text-color-secondary);

              &.small {
                font-size: var(--cs-font-size-md) !important;
              }

              &.cs-send:not(.disabled) {
                color: var(--cs-color-primary);
              }
            }

            label {
              font-size: var(--cs-font-size-sm);
              display: flex;
              align-items: center;
              justify-content: center;
              cursor: pointer;
              color: var(--cs-text-color-secondary);
              background-color: var(--cs-bg-color);
              border-radius: var(--cs-border-radius-lg);
              padding: var(--cs-space-xs) var(--cs-space-sm);
              border: 1px solid var(--cs-bg-color);

              &:not(.disabled):not(.default):hover,
              &.active {
                color: var(--cs-color-primary);

                .cs {
                  color: var(--cs-color-primary);
                }
              }

              &.active {
                border: 1px solid var(--cs-color-primary);
              }
            }
          }

          .el-textarea {
            flex-grow: 1;

            .el-textarea__inner {
              border: none;
              box-shadow: none;
              background: var(--cs-input-bg-color) !important;
              resize: none !important;
              color: var(--cs-text-color-primary);
              padding-left: var(--cs-space-xxs);
              padding-right: var(--cs-space-xxs);
            }
          }

          .input-footer {
            display: flex;
            flex-direction: row;
            align-items: center;
            justify-content: space-between;

            .footer-left {
              display: flex;
              flex-direction: row;
              justify-content: flex-start;
              align-items: center;

              .agent-selector-wrap {
                color: var(--cs-color-primary);
                background: var(--cs-bg-color);
                border: 1px solid var(--cs-color-primary);
                border-radius: var(--cs-border-radius-lg);
                padding: var(--cs-space-xs) var(--cs-space-sm);
                font-size: var(--cs-font-size-md);

                &.disabled {
                  border-color: var(--cs-border-color);
                  background: none;
                }
              }
            }
          }
        }
      }
    }
  }
}

.pin-btn {
  border-radius: var(--cs-border-radius-xs);
  color: var(--cs-text-color-secondary);

  &:hover .cs {
    color: var(--cs-color-primary) !important;
  }

  .cs {
    font-size: var(--cs-font-size-md) !important;
    transform: rotate(45deg);
    transition: all 0.3s ease-in-out;
  }

  &.active {
    .cs {
      color: var(--cs-color-primary);
      transform: rotate(0deg);
    }
  }
}

@keyframes pulse {
  0% {
    opacity: 1;
  }
  50% {
    opacity: 0.5;
  }
  100% {
    opacity: 1;
  }
}
</style>
