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
              :class="{ active: wf.id === workflowStore.currentWorkflowId }">
              {{ wf.title || wf.userQuery }}
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

        <div class="todo-list-wrapper">
          <TodoList
            :items="
              currentWorkflow?.todoList || [
                { title: '正在规划路径', status: 'completed' },
                { title: '正在规划任务', status: 'running' },
                { title: '正在规划任务详情', status: 'pending' },
                { title: '正在规划任务详情内容', status: 'pending' },
                { title: '正在规划任务详情内容内容', status: 'pending' }
              ]
            " />
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
                <div class="agent-selector-wrap" :class="{ disabled: currentWorkflow?.id }">
                  <AgentSelector v-model="selectedAgent" :disabled="currentWorkflow?.id" />
                </div>
                <div class="icons">
                  <el-tooltip
                    content="Auto-approve tools (excluding interactive tools)"
                    placement="top">
                    <label class="icon-btn upperLayer" :class="{ active: autoApproveTools }">
                      <cs name="tool" class="small" />
                    </label>
                  </el-tooltip>
                </div>
              </div>
              <div class="icons">
                <cs name="stop" @click="onStop" v-if="workflowStore.isRunning" />
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
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'
import Titlebar from '@/components/window/Titlebar.vue'
import Markdown from '@/components/chat/Markdown.vue'
import AgentSelector from '@/components/workflow/AgentSelector.vue'
import TodoList from '@/components/workflow/TodoList.vue'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

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

const filteredWorkflows = computed(() => {
  if (!searchQuery.value) return workflows.value
  return workflows.value.filter(wf =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.value.toLowerCase())
  )
})

const canSendMessage = computed(
  () => inputMessage.value.trim() !== '' && !workflowStore.isRunning && selectedAgent.value
)

onMounted(() => {
  workflowStore.loadWorkflows()
  agentStore.fetchAgents().then(() => {
    if (agentStore.agents.length > 0) {
      selectedAgent.value = agentStore.agents[0]
    }
  })
  windowStore.initWorkflowWindowAlwaysOnTop()
})

const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  windowStore.setWorkflowSidebarShow(!sidebarCollapsed.value)
}

const selectWorkflow = id => {
  workflowStore.selectWorkflow(id)
}

const onSendMessage = () => {
  if (!canSendMessage.value) return

  if (!workflowStore.currentWorkflowId) {
    workflowStore.createWorkflow(inputMessage.value, selectedAgent.value.id)
  } else {
    workflowStore.addMessageToQueue({ role: 'user', message: inputMessage.value })
  }
  inputMessage.value = ''
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
  // Logic to stop workflow execution
  console.log('Stopping workflow')
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
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

            &:hover {
              background-color: var(--cs-hover-bg-color);
            }

            &.active {
              background-color: var(--cs-active-bg-color);
              color: var(--el-color-primary);
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
</style>
