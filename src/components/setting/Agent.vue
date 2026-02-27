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
    :close-on-press-escape="false"
    @closed="onAgentDialogClose">
    <el-form :model="agentForm" :rules="agentRules" ref="formRef" label-width="120px">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="$t('settings.agent.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.agent.name')" prop="name">
            <el-input v-model="agentForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.agentType')" prop="agentType">
            <el-radio-group v-model="agentForm.agentType">
              <el-radio-button value="autonomous">{{
                $t('settings.agent.autonomousMode')
              }}</el-radio-button>
              <el-radio-button value="planning">{{
                $t('settings.agent.planningMode')
              }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.description')" prop="description">
            <el-input v-model="agentForm.description" type="textarea" :rows="3" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.systemPrompt')" prop="systemPrompt">
            <el-input v-model="agentForm.systemPrompt" type="textarea" :rows="6" />
          </el-form-item>
          <el-form-item
            v-if="agentForm.agentType === 'planning'"
            :label="$t('settings.agent.planningPrompt')"
            prop="planningPrompt">
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
          <!-- Plan Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="planModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.planModel')" prop="planModel">
              <template v-if="planModelMode === 'provider'">
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
              </template>
              <template v-else>
                <el-select
                  v-model="planProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onPlanProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="planProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onPlanProxyAliasChange"
                  style="width: 45%"
                  :disabled="!planProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(planProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>

          <!-- Act Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="actModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.actModel')" prop="actModel">
              <template v-if="actModelMode === 'provider'">
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
              </template>
              <template v-else>
                <el-select
                  v-model="actProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onActProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="actProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onActProxyAliasChange"
                  style="width: 45%"
                  :disabled="!actProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(actProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>

          <!-- Vision Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="visionModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.visionModel')" prop="visionModel">
              <template v-if="visionModelMode === 'provider'">
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
              </template>
              <template v-else>
                <el-select
                  v-model="visionProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onVisionProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="visionProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onVisionProxyAliasChange"
                  style="width: 45%"
                  :disabled="!visionProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(visionProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>

          <!-- Coding Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="codingModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.codingModel')" prop="codingModel">
              <template v-if="codingModelMode === 'provider'">
                <el-select
                  v-model="agentForm.codingModel.id"
                  :placeholder="$t('settings.agent.selectProvider')"
                  filterable
                  @change="onCodingModelIdChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="provider in modelStore.getAvailableProviders"
                    :key="provider.id"
                    :label="provider.name"
                    :value="provider.id" />
                </el-select>
                <el-select
                  v-model="agentForm.codingModel.model"
                  :placeholder="$t('settings.agent.selectCodingModel')"
                  filterable
                  style="width: 45%"
                  :disabled="!agentForm.codingModel.id">
                  <el-option
                    v-for="model in codingModelList"
                    :key="model.id"
                    :label="model.name || model.id"
                    :value="model.id" />
                </el-select>
              </template>
              <template v-else>
                <el-select
                  v-model="codingProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onCodingProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="codingProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onCodingProxyAliasChange"
                  style="width: 45%"
                  :disabled="!codingProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(codingProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>

          <!-- Copywriting Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="copywritingModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.copywritingModel')" prop="copywritingModel">
              <template v-if="copywritingModelMode === 'provider'">
                <el-select
                  v-model="agentForm.copywritingModel.id"
                  :placeholder="$t('settings.agent.selectProvider')"
                  filterable
                  @change="onCopywritingModelIdChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="provider in modelStore.getAvailableProviders"
                    :key="provider.id"
                    :label="provider.name"
                    :value="provider.id" />
                </el-select>
                <el-select
                  v-model="agentForm.copywritingModel.model"
                  :placeholder="$t('settings.agent.selectCopywritingModel')"
                  filterable
                  style="width: 45%"
                  :disabled="!agentForm.copywritingModel.id">
                  <el-option
                    v-for="model in copywritingModelList"
                    :key="model.id"
                    :label="model.name || model.id"
                    :value="model.id" />
                </el-select>
              </template>
              <template v-else>
                <el-select
                  v-model="copywritingProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onCopywritingProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="copywritingProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onCopywritingProxyAliasChange"
                  style="width: 45%"
                  :disabled="!copywritingProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(copywritingProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>

          <!-- Browsing Model -->
          <div class="model-config-item">
            <div class="model-mode-selector">
              <el-radio-group v-model="browsingModelMode" size="small">
                <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
              </el-radio-group>
            </div>
            <el-form-item :label="$t('settings.agent.browsingModel')" prop="browsingModel">
              <template v-if="browsingModelMode === 'provider'">
                <el-select
                  v-model="agentForm.browsingModel.id"
                  :placeholder="$t('settings.agent.selectProvider')"
                  filterable
                  @change="onBrowsingModelIdChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="provider in modelStore.getAvailableProviders"
                    :key="provider.id"
                    :label="provider.name"
                    :value="provider.id" />
                </el-select>
                <el-select
                  v-model="agentForm.browsingModel.model"
                  :placeholder="$t('settings.agent.selectBrowsingModel')"
                  filterable
                  style="width: 45%"
                  :disabled="!agentForm.browsingModel.id">
                  <el-option
                    v-for="model in browsingModelList"
                    :key="model.id"
                    :label="model.name || model.id"
                    :value="model.id" />
                </el-select>
              </template>
              <template v-else>
                <el-select
                  v-model="browsingProxyGroup"
                  :placeholder="$t('settings.agent.group')"
                  filterable
                  @change="onBrowsingProxyGroupChange"
                  style="width: 45%; margin-right: var(--cs-space-sm)">
                  <el-option
                    v-for="group in proxyGroupStore.list"
                    :key="group.name"
                    :label="group.name"
                    :value="group.name" />
                </el-select>
                <el-select
                  v-model="browsingProxyAlias"
                  :placeholder="$t('settings.agent.aliasName')"
                  filterable
                  @change="onBrowsingProxyAliasChange"
                  style="width: 45%"
                  :disabled="!browsingProxyGroup">
                  <el-option
                    v-for="alias in getProxyAliases(browsingProxyGroup)"
                    :key="alias"
                    :label="alias"
                    :value="alias" />
                </el-select>
              </template>
            </el-form-item>
          </div>
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
import { useProxyGroupStore } from '@/stores/proxy_group'
import { useSettingStore } from '@/stores/setting'
import { FrontendAppError } from '@/libs/tauri'

const { t } = useI18n()

const modelStore = useModelStore()
const agentStore = useAgentStore()
const proxyGroupStore = useProxyGroupStore()
const settingStore = useSettingStore()
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
  codingModel: { id: '', model: '' },
  copywritingModel: { id: '', model: '' },
  browsingModel: { id: '', model: '' },
  maxContexts: 128000
}

// Reactive object to hold the form data for the agent
const agentForm = ref({ ...defaultFormData })

// Model modes: 'provider' or 'proxy'
const planModelMode = ref('provider')
const actModelMode = ref('provider')
const visionModelMode = ref('provider')
const codingModelMode = ref('provider')
const copywritingModelMode = ref('provider')
const browsingModelMode = ref('provider')

// Proxy temporary selections
const planProxyGroup = ref('')
const planProxyAlias = ref('')
const actProxyGroup = ref('')
const actProxyAlias = ref('')
const visionProxyGroup = ref('')
const visionProxyAlias = ref('')
const codingProxyGroup = ref('')
const codingProxyAlias = ref('')
const copywritingProxyGroup = ref('')
const copywritingProxyAlias = ref('')
const browsingProxyGroup = ref('')
const browsingProxyAlias = ref('')

// Validation rules for the agent form
const agentRules = {
  name: [{ required: true, message: t('settings.agent.nameRequired') }],
  systemPrompt: [{ required: true, message: t('settings.agent.systemPromptRequired') }]
}

// Computed properties for dependent model dropdowns
const planModelList = computed(() => {
  if (agentForm.value.planModel?.id && typeof agentForm.value.planModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.planModel.id)?.models || []
  }
  return []
})

const actModelList = computed(() => {
  if (agentForm.value.actModel?.id && typeof agentForm.value.actModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.actModel.id)?.models || []
  }
  return []
})

const visionModelList = computed(() => {
  if (agentForm.value.visionModel?.id && typeof agentForm.value.visionModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.visionModel.id)?.models || []
  }
  return []
})

const codingModelList = computed(() => {
  if (agentForm.value.codingModel?.id && typeof agentForm.value.codingModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.codingModel.id)?.models || []
  }
  return []
})

const copywritingModelList = computed(() => {
  if (agentForm.value.copywritingModel?.id && typeof agentForm.value.copywritingModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.copywritingModel.id)?.models || []
  }
  return []
})

const browsingModelList = computed(() => {
  if (agentForm.value.browsingModel?.id && typeof agentForm.value.browsingModel.id === 'number') {
    return modelStore.getModelProviderById(agentForm.value.browsingModel.id)?.models || []
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
const onCodingModelIdChange = () => {
  agentForm.value.codingModel.model = ''
}
const onCopywritingModelIdChange = () => {
  agentForm.value.copywritingModel.model = ''
}
const onBrowsingModelIdChange = () => {
  agentForm.value.browsingModel.model = ''
}

// Proxy handlers
const getProxyAliases = groupName => {
  if (!groupName) return []
  const groupData = settingStore.settings.chatCompletionProxy[groupName]
  return groupData ? Object.keys(groupData) : []
}

const onPlanProxyGroupChange = () => {
  planProxyAlias.value = ''
}
const onActProxyGroupChange = () => {
  actProxyAlias.value = ''
}
const onVisionProxyGroupChange = () => {
  visionProxyAlias.value = ''
}
const onCodingProxyGroupChange = () => {
  codingProxyAlias.value = ''
}
const onCopywritingProxyGroupChange = () => {
  copywritingProxyAlias.value = ''
}
const onBrowsingProxyGroupChange = () => {
  browsingProxyAlias.value = ''
}

const onPlanProxyAliasChange = value => {
  agentForm.value.planModel.model = `${planProxyGroup.value}@${value}`
}
const onActProxyAliasChange = value => {
  agentForm.value.actModel.model = `${actProxyGroup.value}@${value}`
}
const onVisionProxyAliasChange = value => {
  agentForm.value.visionModel.model = `${visionProxyGroup.value}@${value}`
}
const onCodingProxyAliasChange = value => {
  agentForm.value.codingModel.model = `${codingProxyGroup.value}@${value}`
}
const onCopywritingProxyAliasChange = value => {
  agentForm.value.copywritingModel.model = `${copywritingProxyGroup.value}@${value}`
}
const onBrowsingProxyAliasChange = value => {
  agentForm.value.browsingModel.model = `${browsingProxyGroup.value}@${value}`
}

/**
 * Parses a model field into mode and temporary proxy variables
 */
const parseModelField = (field, modeRef, groupRef, aliasRef) => {
  if (field && field.id === 0 && field.model.includes('@')) {
    modeRef.value = 'proxy'
    const [group, ...rest] = field.model.split('@')
    groupRef.value = group
    aliasRef.value = rest.join('@')
  } else {
    modeRef.value = 'provider'
    groupRef.value = ''
    aliasRef.value = ''
  }
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
      agentForm.value = { ...defaultFormData, ...agentData }

      // Unpack unified 'models' JSON field if it exists
      if (agentData.models) {
        try {
          const modelsObj = JSON.parse(agentData.models)
          if (modelsObj.plan) agentForm.value.planModel = modelsObj.plan
          if (modelsObj.act) agentForm.value.actModel = modelsObj.act
          if (modelsObj.vision) agentForm.value.visionModel = modelsObj.vision
          if (modelsObj.coding) agentForm.value.codingModel = modelsObj.coding
          if (modelsObj.copywriting) agentForm.value.copywritingModel = modelsObj.copywriting
          if (modelsObj.browsing) agentForm.value.browsingModel = modelsObj.browsing
        } catch (e) {
          console.error('Failed to parse models JSON:', e)
        }
      }

      // Parse model modes for UI
      parseModelField(agentForm.value.planModel, planModelMode, planProxyGroup, planProxyAlias)
      parseModelField(agentForm.value.actModel, actModelMode, actProxyGroup, actProxyAlias)
      parseModelField(agentForm.value.visionModel, visionModelMode, visionProxyGroup, visionProxyAlias)
      parseModelField(agentForm.value.codingModel, codingModelMode, codingProxyGroup, codingProxyAlias)
      parseModelField(agentForm.value.copywritingModel, copywritingModelMode, copywritingProxyGroup, copywritingProxyAlias)
      parseModelField(agentForm.value.browsingModel, browsingModelMode, browsingProxyGroup, browsingProxyAlias)
    } catch (error) {
      if (error instanceof FrontendAppError) {
        showMessage(t('settings.agent.fetchFailed', { error: error.toFormattedString() }), 'error')
        console.error('Error fetching agent:', error.originalError)
      } else {
        showMessage(
          t('settings.agent.fetchFailed', { error: error.message || String(error) }),
          'error'
        )
        console.error('Error fetching agent:', error)
      }
      return
    }
  } else {
    editId.value = null
    agentForm.value = { ...defaultFormData }
    planModelMode.value = 'provider'
    actModelMode.value = 'provider'
    visionModelMode.value = 'provider'
    codingModelMode.value = 'provider'
    copywritingModelMode.value = 'provider'
    browsingModelMode.value = 'provider'
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
    const agentData = await agentStore.getAgent(id)
    if (!agentData) return
    
    agentForm.value = { ...defaultFormData, ...agentData }
    editId.value = null // Ensure editId is cleared for copy

    // Unpack unified 'models' JSON field if it exists
    if (agentData.models) {
      try {
        const modelsObj = JSON.parse(agentData.models)
        if (modelsObj.plan) agentForm.value.planModel = modelsObj.plan
        if (modelsObj.act) agentForm.value.actModel = modelsObj.act
        if (modelsObj.vision) agentForm.value.visionModel = modelsObj.vision
        if (modelsObj.coding) agentForm.value.codingModel = modelsObj.coding
        if (modelsObj.copywriting) agentForm.value.copywritingModel = modelsObj.copywriting
        if (modelsObj.browsing) agentForm.value.browsingModel = modelsObj.browsing
      } catch (e) {
        console.error('Failed to parse models JSON:', e)
      }
    }

    // Parse model modes for the copy
    parseModelField(agentForm.value.planModel, planModelMode, planProxyGroup, planProxyAlias)
    parseModelField(agentForm.value.actModel, actModelMode, actProxyGroup, actProxyAlias)
    parseModelField(agentForm.value.visionModel, visionModelMode, visionProxyGroup, visionProxyAlias)
    parseModelField(agentForm.value.codingModel, codingModelMode, codingProxyGroup, codingProxyAlias)
    parseModelField(agentForm.value.copywritingModel, copywritingModelMode, copywritingProxyGroup, copywritingProxyAlias)
    parseModelField(agentForm.value.browsingModel, browsingModelMode, browsingProxyGroup, browsingProxyAlias)

    agentDialogVisible.value = true
  } catch (error) {
    if (error instanceof FrontendAppError) {
      showMessage(
        t('settings.agent.fetchFailed', {
          error: error.toFormattedString()
        }),
        'error'
      )
      console.error('Error copying agent:', error.originalError)
    } else {
      showMessage(
        t('settings.agent.fetchFailed', { error: error.message || String(error) }),
        'error'
      )
      console.error('Error copying agent:', error)
    }
  }
}

/**
 * Validates the form and updates or adds an agent based on the current form data.
 */
const updateAgent = () => {
  formRef.value.validate(async valid => {
    if (valid) {
      // Final data preparation: Ensure ID is 0 for proxy mode
      const finalForm = JSON.parse(JSON.stringify(agentForm.value))
      
      const prepareModel = (field, mode, group, alias) => {
        if (mode === 'proxy') {
          field.id = 0
          field.model = `${group}@${alias}`
        }
      }

      prepareModel(finalForm.planModel, planModelMode.value, planProxyGroup.value, planProxyAlias.value)
      prepareModel(finalForm.actModel, actModelMode.value, actProxyGroup.value, actProxyAlias.value)
      prepareModel(finalForm.visionModel, visionModelMode.value, visionProxyGroup.value, visionProxyAlias.value)
      prepareModel(finalForm.codingModel, codingModelMode.value, codingProxyGroup.value, codingProxyAlias.value)
      prepareModel(finalForm.copywritingModel, copywritingModelMode.value, copywritingProxyGroup.value, copywritingProxyAlias.value)
      prepareModel(finalForm.browsingModel, browsingModelMode.value, browsingProxyGroup.value, browsingProxyAlias.value)

      // Consolidate all into 'models' field
      finalForm.models = JSON.stringify({
        plan: finalForm.planModel,
        act: finalForm.actModel,
        vision: finalForm.visionModel,
        coding: finalForm.codingModel,
        copywriting: finalForm.copywritingModel,
        browsing: finalForm.browsingModel
      })

      try {
        await agentStore.saveAgent({ ...finalForm, id: editId.value })
        showMessage(
          t(editId.value ? 'settings.agent.updateSuccess' : 'settings.agent.addSuccess'),
          'success'
        )
        agentDialogVisible.value = false
      } catch (error) {
        if (error instanceof FrontendAppError) {
          showMessage(
            t('settings.agent.saveFailed', {
              error: error.toFormattedString()
            }),
            'error'
          )
          console.error('Error saving agent:', error.originalError)
        } else {
          showMessage(
            t('settings.agent.saveFailed', { error: error.message || String(error) }),
            'error'
          )
          console.error('Error saving agent:', error)
        }
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
      if (error instanceof FrontendAppError) {
        showMessage(
          t('settings.agent.deleteFailed', {
            error: error.toFormattedString()
          }),
          'error'
        )
        console.error('Error deleting agent:', error.originalError)
      } else {
        showMessage(
          t('settings.agent.deleteFailed', { error: error.message || String(error) }),
          'error'
        )
        console.error('Error deleting agent:', error)
      }
    }
  })
}

/**
 * Handles the end of a drag event to reorder agents.
 */
const onDragEnd = () => {
  agentStore.updateAgentOrder(agents.value).catch(error => {
    if (error instanceof FrontendAppError) {
      showMessage(
        t('settings.agent.reorderFailed', {
          error: error.toFormattedString()
        }),
        'error'
      )
      console.error('Error reordering agents:', error.originalError)
    } else {
      showMessage(
        t('settings.agent.reorderFailed', { error: error.message || String(error) }),
        'error'
      )
      console.error('Error reordering agents:', error)
    }
    // Revert visual change by fetching the original order
    agentStore.fetchAgents()
  })
}

// Load models when component is mounted
onMounted(() => {
  modelStore.updateModelStore()
  proxyGroupStore.getList()
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

    .model-config-item {
      margin-bottom: var(--cs-space-md);
      padding: var(--cs-space-sm);
      border: 1px solid var(--cs-border-color);
      border-radius: var(--cs-border-radius-md);
      background-color: var(--cs-bg-color-light);

      .model-mode-selector {
        margin-bottom: var(--cs-space-sm);
        display: flex;
        justify-content: center;
      }

      .el-form-item {
        margin-bottom: 0;
      }
    }
  }
}
</style>
