<template>
  <div class="card">
    <div class="title">
      <span>{{ $t('settings.agent.title') }}</span>
      <el-tooltip :content="$t('settings.agent.add')" placement="left">
        <span class="icon" @click="editAgent()">
          <cs name="add" />
        </span>
      </el-tooltip>
    </div>
    <Sortable
      v-if="agents.length > 0"
      class="list"
      item-key="id"
      :list="agents"
      :options="{
        animation: 150,
        ghostClass: 'ghost',
        dragClass: 'drag',
        draggable: '.draggable',
        forceFallback: true,
        bubbleScroll: true
      }"
      @end="onDragEnd">
      <template #item="{ element }">
        <div class="item draggable" :key="element.id">
          <div class="label">
            <avatar :text="element.name" :size="20" />
            {{ element.name }}
          </div>

          <!-- manage icons -->
          <div class="value">
            <el-tooltip
              :content="$t('settings.agent.edit')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="editAgent(element.id)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.agent.copy')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="copyAgent(element.id)" @mousedown.stop>
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip
              :content="$t('settings.agent.delete')"
              placement="top"
              :hide-after="0"
              :enterable="false"
              transition="none">
              <div class="icon" @click="deleteAgent(element.id)" @mousedown.stop>
                <cs name="trash" size="16px" color="secondary" />
              </div>
            </el-tooltip>
          </div>
        </div>
      </template>
    </Sortable>
    <div class="list" v-else>
      <div class="item">
        <div class="label">{{ $t('settings.agent.noAgents') }}</div>
      </div>
    </div>
  </div>

  <!-- add/edit agent dialog -->
  <el-dialog
    v-model="agentDialogVisible"
    width="600px"
    class="agent-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false">
    <el-form :model="agentForm" :rules="agentRules" ref="formRef" label-width="120px">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="$t('settings.agent.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.agent.name')" prop="name">
            <el-input v-model="agentForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.agentType')" prop="agentType">
            <el-radio-group v-model="agentForm.agentType">
              <el-radio-button label="autonomous">{{ $t('settings.agent.autonomousMode') }}</el-radio-button>
              <el-radio-button label="planning">{{ $t('settings.agent.planningMode') }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.description')" prop="description">
            <el-input v-model="agentForm.description" type="textarea" :rows="3" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.systemPrompt')" prop="systemPrompt">
            <el-input v-model="agentForm.systemPrompt" type="textarea" :rows="6" />
          </el-form-item>
          <el-form-item v-if="agentForm.agentType === 'planning'" :label="$t('settings.agent.planningPrompt')" prop="planningPrompt">
            <el-input v-model="agentForm.planningPrompt" type="textarea" :rows="6" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.maxContexts')" prop="maxContexts">
            <el-input-number
              v-model="agentForm.maxContexts"
              :min="1000"
              :max="1000000"
              :step="1000"
              controls-position="right"
              style="width: 100%" />
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.models')" name="models">
          <el-form-item :label="$t('settings.agent.planModel')" prop="planModel">
            <el-select
              v-model="agentForm.planModel.id"
              :placeholder="$t('settings.agent.selectProvider')"
              filterable
              @change="onPlanModelIdChange"
              style="width: 45%; margin-right: var(--cs-space-sm)">
              <el-option
                v-for="provider in modelStore.getAvailableProviders"
                :key="provider.id"
                :label="provider.name"
                :value="provider.id" />
            </el-select>
            <el-select
              v-model="agentForm.planModel.model"
              :placeholder="$t('settings.agent.selectPlanModel')"
              filterable
              style="width: 45%"
              :disabled="!agentForm.planModel.id">
              <el-option
                v-for="model in planModelList"
                :key="model.id"
                :label="model.name || model.id"
                :value="model.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.actModel')" prop="actModel">
            <el-select
              v-model="agentForm.actModel.id"
              :placeholder="$t('settings.agent.selectProvider')"
              filterable
              @change="onActModelIdChange"
              style="width: 45%; margin-right: var(--cs-space-sm)">
              <el-option
                v-for="provider in modelStore.getAvailableProviders"
                :key="provider.id"
                :label="provider.name"
                :value="provider.id" />
            </el-select>
            <el-select
              v-model="agentForm.actModel.model"
              :placeholder="$t('settings.agent.selectActModel')"
              filterable
              style="width: 45%"
              :disabled="!agentForm.actModel.id">
              <el-option
                v-for="model in actModelList"
                :key="model.id"
                :label="model.name || model.id"
                :value="model.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.visionModel')" prop="visionModel">
            <el-select
              v-model="agentForm.visionModel.id"
              :placeholder="$t('settings.agent.selectProvider')"
              filterable
              @change="onVisionModelIdChange"
              style="width: 45%; margin-right: var(--cs-space-sm)">
              <el-option
                v-for="provider in modelStore.getAvailableProviders"
                :key="provider.id"
                :label="provider.name"
                :value="provider.id" />
            </el-select>
            <el-select
              v-model="agentForm.visionModel.model"
              :placeholder="$t('settings.agent.selectVisionModel')"
              filterable
              style="width: 45%"
              :disabled="!agentForm.visionModel.id">
              <el-option
                v-for="model in visionModelList"
                :key="model.id"
                :label="model.name || model.id"
                :value="model.id" />
            </el-select>
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.toolsLabel')" name="tools">
          <el-form-item :label="$t('settings.agent.availableTools')" prop="availableTools">
            <el-select
              v-model="agentForm.availableTools"
              :placeholder="$t('settings.agent.selectAvailableTools')"
              multiple
              filterable>
              <el-option
                v-for="tool in availableTools"
                :key="tool.id"
                :label="tool.name"
                :value="tool.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.autoApprove')" prop="autoApprove">
            <el-select
              v-model="agentForm.autoApprove"
              :placeholder="$t('settings.agent.selectAutoApproveTools')"
              multiple
              filterable>
              <el-option
                v-for="tool in availableTools.filter(t => agentForm.availableTools.includes(t.id))"
                :key="tool.id"
                :label="tool.name"
                :value="tool.id" />
            </el-select>
          </el-form-item>
        </el-tab-pane>
      </el-tabs>
    </el-form>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="agentDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateAgent">{{ $t('common.save') }}</el-button>
      </span>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'
import { Sortable } from 'sortablejs-vue3'

import { showMessage } from '@/libs/util'
import { useModelStore } from '@/stores/model'
import { useAgentStore } from '@/stores/agent'

const { t } = useI18n()

const modelStore = useModelStore()
const agentStore = useAgentStore()
const { agents, availableTools } = storeToRefs(agentStore)

const formRef = ref(null)
const agentDialogVisible = ref(false)
const editId = ref(null)
const activeTab = ref('basic')

const defaultFormData = {
  name: '',
  description: '',
  systemPrompt: '',
  agentType: 'autonomous',
  planningPrompt: `Please act as an expert project manager. Analyze the user's request and provide a clear, step-by-step plan to achieve the goal. The plan should be a list of tasks. For each task, describe what needs to be done and why it's necessary. Ensure the plan is logical, efficient, and covers all aspects of the request. Your final output should only be the plan itself, without any conversational text before or after it.`,
  availableTools: [],
  autoApprove: [],
  planModel: { id: '', model: '' },
  actModel: { id: '', model: '' },
  visionModel: { id: '', model: '' },
  maxContexts: 128000
}

// Reactive object to hold the form data for the agent
const agentForm = ref({ ...defaultFormData })

// Validation rules for the agent form
const agentRules = {
  name: [{ required: true, message: t('settings.agent.nameRequired') }],
  systemPrompt: [{ required: true, message: t('settings.agent.systemPromptRequired') }]
}

// Computed properties for dependent model dropdowns
const planModelList = computed(() => {
  if (agentForm.value.planModel?.id) {
    return modelStore.getModelProviderById(agentForm.value.planModel.id)?.models || []
  }
  return []
})

const actModelList = computed(() => {
  if (agentForm.value.actModel?.id) {
    return modelStore.getModelProviderById(agentForm.value.actModel.id)?.models || []
  }
  return []
})

const visionModelList = computed(() => {
  if (agentForm.value.visionModel?.id) {
    return modelStore.getModelProviderById(agentForm.value.visionModel.id)?.models || []
  }
  return []
})

// Handlers to reset model selection when provider changes
const onPlanModelIdChange = () => {
  agentForm.value.planModel.model = ''
}
const onActModelIdChange = () => {
  agentForm.value.actModel.model = ''
}
const onVisionModelIdChange = () => {
  agentForm.value.visionModel.model = ''
}

/**
 * Opens the agent dialog for editing or creating a new agent.
 * @param {string|null} id - The ID of the agent to edit, or null to create a new agent.
 */
const editAgent = async id => {
  formRef.value?.resetFields()
  activeTab.value = 'basic'

  if (id) {
    try {
      const agentData = await agentStore.getAgent(id)
      if (!agentData) {
        showMessage(t('settings.agent.notFound'), 'error')
        return
      }
      editId.value = id
      agentForm.value = agentData
    } catch (error) {
      showMessage(t('settings.agent.fetchFailed'), 'error')
      return
    }
  } else {
    editId.value = null
    agentForm.value = { ...defaultFormData }
    // Default auto-approve web tools for new agents
    agentForm.value.autoApprove = availableTools.value
      .filter(tool => tool.category === 'Web')
      .map(tool => tool.id)
  }

  agentDialogVisible.value = true
}

/**
 * Creates a copy of the specified agent and opens the dialog for editing.
 * @param {string} id - The ID of the agent to copy.
 */
const copyAgent = async id => {
  try {
    const agentToCopy = await agentStore.copyAgent(id)
    agentForm.value = agentToCopy
    editId.value = null // Ensure editId is cleared for copy
    agentDialogVisible.value = true
  } catch (error) {
    showMessage(t('settings.agent.fetchFailed'), 'error')
  }
}

/**
 * Validates the form and updates or adds an agent based on the current form data.
 */
const updateAgent = () => {
  formRef.value.validate(async valid => {
    if (valid) {
      try {
        await agentStore.saveAgent({ ...agentForm.value, id: editId.value })
        showMessage(
          t(editId.value ? 'settings.agent.updateSuccess' : 'settings.agent.addSuccess'),
          'success'
        )
        agentDialogVisible.value = false
      } catch (error) {
        showMessage(t('settings.agent.saveFailed', { error: error.message || error }), 'error')
      }
    } else {
      console.log('error submit!')
      return false
    }
  })
}

/**
 * Confirms and deletes the specified agent.
 * @param {string} id - The ID of the agent to delete.
 */
const deleteAgent = id => {
  ElMessageBox.confirm(t('settings.agent.deleteConfirm'), t('settings.agent.deleteTitle'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning'
  }).then(async () => {
    try {
      await agentStore.deleteAgent(id)
      showMessage(t('settings.agent.deleteSuccess'), 'success')
    } catch (error) {
      showMessage(t('settings.agent.deleteFailed', { error: error.message || error }), 'error')
    }
  })
}

/**
 * Handles the end of a drag event to reorder agents.
 */
const onDragEnd = () => {
  agentStore.updateAgentOrder(agents.value).catch(error => {
    showMessage(t('settings.agent.reorderFailed', { error: error.message || error }), 'error')
    // Revert visual change by fetching the original order
    agentStore.fetchAgents()
  })
}

// Load models when component is mounted
onMounted(() => {
  modelStore.updateModelStore()
})
</script>

<style lang="scss">
.ghost {
  background: rgba(255, 255, 255, 0.1);
}

.el-overlay {
  .agent-edit-dialog {
    .el-dialog__header {
      display: none;
    }

    .el-tabs__nav-wrap:after {
      background-color: var(--cs-border-color);
    }
  }
}
</style>
