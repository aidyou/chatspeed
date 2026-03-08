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
    <Sortable v-if="agents.length > 0" class="list" item-key="id" :list="agents" :options="{
      animation: 150,
      ghostClass: 'ghost',
      dragClass: 'drag',
      draggable: '.draggable',
      forceFallback: true,
      bubbleScroll: true
    }" @end="onDragEnd">
      <template #item="{ element }">
        <div class="item draggable" :key="element.id">
          <div class="label">
            <avatar :text="element.name" :size="20" />
            {{ element.name }}
          </div>

          <div class="value">
            <el-tooltip :content="$t('settings.agent.edit')" placement="top" :hide-after="0" :enterable="false"
              transition="none">
              <div class="icon" @click="editAgent(element.id)" @mousedown.stop>
                <cs name="edit" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip :content="$t('settings.agent.copy')" placement="top" :hide-after="0" :enterable="false"
              transition="none">
              <div class="icon" @click="copyAgent(element.id)" @mousedown.stop>
                <cs name="copy" size="16px" color="secondary" />
              </div>
            </el-tooltip>
            <el-tooltip :content="$t('settings.agent.delete')" placement="top" :hide-after="0" :enterable="false"
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

  <el-dialog v-model="agentDialogVisible" width="640px" class="agent-edit-dialog" :show-close="false"
    :close-on-click-modal="false" :close-on-press-escape="false" @closed="onAgentDialogClose">
    <el-form :model="agentForm" :rules="agentRules" ref="formRef" label-width="100px">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="$t('settings.agent.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.agent.name')" prop="name">
            <el-input v-model="agentForm.name" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.description')" prop="description">
            <el-input v-model="agentForm.description" type="textarea" :rows="2" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.systemPrompt')" prop="systemPrompt">
            <el-input v-model="agentForm.systemPrompt" type="textarea" :rows="5" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.planningPrompt')" prop="planningPrompt">
            <el-input v-model="agentForm.planningPrompt" type="textarea" :rows="5"
              :placeholder="$t('settings.agent.planningPromptPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.finalAudit')" prop="finalAudit">
            <el-switch v-model="agentForm.finalAudit" />
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.models')" name="models">
          <div class="models-layout">
            <el-row :gutter="12">
              <el-col :span="12" v-for="role in modelRoles" :key="role.key">
                <div class="model-item-compact">
                  <div class="header">
                    <span class="title">{{ $t(`settings.agent.${role.key}Model`) }}</span>
                    <el-radio-group v-model="modelModes[role.key]" size="small">
                      <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
                      <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
                    </el-radio-group>
                  </div>
                  <div class="body">
                    <div class="selectors-row">
                      <template v-if="modelModes[role.key] === 'provider'">
                        <el-select v-model="agentForm[role.key + 'Model'].id" size="small" filterable
                          @change="onModelIdChange(role.key)" style="width: 100px">
                          <el-option v-for="provider in modelStore.getAvailableProviders" :key="provider.id"
                            :label="provider.name" :value="provider.id" />
                        </el-select>
                        <el-select v-model="agentForm[role.key + 'Model'].model" size="small" filterable
                          :disabled="!agentForm[role.key + 'Model'].id" style="flex: 1">
                          <el-option v-for="model in getModelList(role.key)" :key="model.id"
                            :label="model.name || model.id" :value="model.id" />
                        </el-select>
                      </template>
                      <template v-else>
                        <el-select v-model="proxyGroups[role.key]" size="small" filterable
                          @change="onProxyGroupChange(role.key)" style="width: 100px">
                          <el-option v-for="group in proxyGroupStore.list" :key="group.name" :label="group.name"
                            :value="group.name" />
                        </el-select>
                        <el-select v-model="proxyAliases[role.key]" size="small" filterable
                          :disabled="!proxyGroups[role.key]" @change="val => onProxyAliasChange(role.key, val)"
                          style="flex: 1">
                          <el-option v-for="alias in getProxyAliases(proxyGroups[role.key])" :key="alias" :label="alias"
                            :value="alias" />
                        </el-select>
                      </template>
                    </div>
                    <div class="params-row" style="margin-top: 8px; padding: 0 4px;">
                      <span class="param-label">{{ $t('settings.agent.temperature') }}</span>
                      <el-slider v-model="agentForm[role.key + 'Model'].temperature" :min="-0.1" :max="2" :step="0.1"
                        size="small" style="flex: 1; margin-left: 12px;" />
                      <span class="param-value" style="font-size: 11px; min-width: 24px; text-align: right;">{{
                        (agentForm[role.key + 'Model']?.temperature ?? -0.1) < 0 ? 'Off' : agentForm[role.key + 'Model'
                        ]?.temperature?.toFixed(1) || '0.0' }}</span>
                    </div>
                    <div class="params-row compact-params" style="margin-top: 4px;">
                      <div class="param-item">
                        <span class="param-label">{{ $t('settings.model.contextSize') }}</span>
                        <el-input-number v-model="agentForm[role.key + 'Model'].contextSize" :min="1024" :max="2000000" :step="1024"
                          size="small" controls-position="right" style="width: 80px" />
                      </div>
                      <div class="param-item">
                        <span class="param-label">{{ $t('settings.model.maxTokens') }}</span>
                        <el-input-number v-model="agentForm[role.key + 'Model'].maxTokens" :min="0" :max="128000" :step="1024"
                          size="small" controls-position="right" style="width: 80px" />
                      </div>
                    </div>
                  </div>
                </div>
              </el-col>
            </el-row>
          </div>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.toolsLabel')" name="tools">
          <el-form-item :label="$t('settings.agent.approvalLevel')" prop="approvalLevel">
            <el-select v-model="agentForm.approvalLevel" style="width: 100%">
              <el-option :label="$t('settings.agent.approvalLevelDefault')" value="default" />
              <el-option :label="$t('settings.agent.approvalLevelSmart')" value="smart" />
              <el-option :label="$t('settings.agent.approvalLevelFull')" value="full" class="danger-option" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.availableTools')" prop="availableTools">
            <el-select v-model="agentForm.availableTools" :placeholder="$t('settings.agent.selectAvailableTools')"
              multiple filterable>
              <el-option v-for="tool in sortedAvailableTools" :key="tool.id" :label="tool.name" :value="tool.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.autoApprove')" prop="autoApprove">
            <el-select v-model="agentForm.autoApprove" :placeholder="$t('settings.agent.selectAutoApproveTools')"
              multiple filterable>
              <el-option v-for="tool in autoApproveOptions" :key="tool.id" :label="tool.name" :value="tool.id" />
            </el-select>
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.security')" name="security">
          <div class="security-group">
            <div class="shell-policy-header">
              <h3>{{ $t('settings.agent.authorizedPaths') }}</h3>
              <div class="shell-policy-actions">
                <el-button type="primary" size="small" @click="addAuthorizedPath">
                  {{ $t('settings.agent.authorizedPathsAdd') }}
                </el-button>
              </div>
            </div>
            <p class="security-tip">{{ $t('settings.agent.authorizedPathsTip') }}</p>
            <div class="shell-policy-list">
              <div v-for="(path, index) in agentForm.allowedPaths" :key="index" class="shell-policy-item">
                <el-input v-model="agentForm.allowedPaths[index]" size="small" readonly style="flex: 1" />
                <el-button type="danger" size="small" circle @click="removeAuthorizedPath(index)">
                  <cs name="trash" size="12px" />
                </el-button>
              </div>
            </div>
          </div>

          <div v-if="agentForm.availableTools.includes('bash')" class="security-group" style="margin-top: 24px;">
            <div class="shell-policy-header">
              <h3>{{ $t('settings.agent.shellPolicy') }}</h3>
              <div class="shell-policy-actions">
                <el-button type="primary" size="small" @click="addShellPolicyRule">
                  {{ $t('settings.agent.shellPolicyAdd') }}
                </el-button>
                <el-button type="info" size="small" @click="importDefaultShellPolicies" plain>
                  {{ $t('settings.agent.shellPolicyImportDefault') }}
                </el-button>
                <el-button v-if="agentForm.shellPolicy && agentForm.shellPolicy.length > 0" type="danger" size="small"
                  @click="clearShellPolicyRules" plain>
                  {{ $t('settings.agent.shellPolicyClear') }}
                </el-button>
              </div>
            </div>
            <div class="shell-policy-list" ref="shellPolicyListRef">
              <div v-for="(rule, index) in agentForm.shellPolicy" :key="index" class="shell-policy-item">
                <el-input v-model="rule.pattern" size="small" :placeholder="$t('settings.agent.shellPolicyPattern')"
                  style="flex: 1" />
                <el-select v-model="rule.decision" size="small" style="width: 130px">
                  <el-option :label="$t('settings.agent.shellDecisionAllow')" value="allow" />
                  <el-option :label="$t('settings.agent.shellDecisionReview')" value="review" />
                  <el-option :label="$t('settings.agent.shellDecisionDeny')" value="deny" />
                </el-select>
                <el-button type="danger" size="small" circle @click="removeShellPolicyRule(index)">
                  <cs name="trash" size="12px" />
                </el-button>
              </div>
            </div>
          </div>
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
import { computed, ref, onMounted, reactive, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'
import { Sortable } from 'sortablejs-vue3'
import { open } from '@tauri-apps/plugin-dialog'

import { showMessage } from '@/libs/util'
import { useModelStore } from '@/stores/model'
import { useAgentStore } from '@/stores/agent'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { useSettingStore } from '@/stores/setting'

const { t } = useI18n()

const modelStore = useModelStore()
const agentStore = useAgentStore()
const proxyGroupStore = useProxyGroupStore()
const settingStore = useSettingStore()
const { agents, availableTools } = storeToRefs(agentStore)

const formRef = ref(null)
const shellPolicyListRef = ref(null)
const agentDialogVisible = ref(false)
const editId = ref(null)
const activeTab = ref('basic')

const modelRoles = [
  { key: 'plan' },
  { key: 'act' },
  { key: 'vision' },
  { key: 'coding' },
  { key: 'copywriting' },
  { key: 'browsing' }
]

const READ_ONLY_TOOLS = ['read_file', 'grep', 'glob', 'web_fetch', 'todo_list', 'list_dir']
const CORE_MANAGEMENT_TOOLS = ['task', 'task_output', 'task_stop', 'todo_create', 'todo_list', 'todo_update', 'todo_get', 'skill', 'ask_user', 'finish_task', 'submit_plan']

const defaultFormData = {
  name: '',
  description: '',
  systemPrompt: '',
  planningPrompt: '',
  availableTools: [],
  autoApprove: [],
  shellPolicy: [],
  allowedPaths: [],
  planModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  actModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  visionModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  codingModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  copywritingModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  browsingModel: { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 },
  maxContexts: 128000,
  finalAudit: false,
  approvalLevel: 'default'
}

const agentForm = ref({ ...defaultFormData })

// Model config temporary state
const modelModes = reactive({ plan: 'provider', act: 'provider', vision: 'provider', coding: 'provider', copywriting: 'provider', browsing: 'provider' })
const proxyGroups = reactive({ plan: '', act: '', vision: '', coding: '', copywriting: '', browsing: '' })
const proxyAliases = reactive({ plan: '', act: '', vision: '', coding: '', copywriting: '', browsing: '' })

// Computed property: available tools sorted by name, filtered to exclude core management tools
const sortedAvailableTools = computed(() => {
  return [...availableTools.value]
    .filter(t => !CORE_MANAGEMENT_TOOLS.includes(t.id))
    .sort((a, b) => {
      return a.name.localeCompare(b.name, 'zh-Hans')
    })
})

// Computed property: auto-approve tool options (filtered and sorted)
const autoApproveOptions = computed(() => {
  if (!agentForm.value || !agentForm.value.availableTools) return []
  return sortedAvailableTools.value.filter(t =>
    agentForm.value.availableTools.includes(t.id) && t.id !== 'bash'
  )
})

// Tool ID to name mapping
const toolNameMap = computed(() => {
  const map = {}
  availableTools.value.forEach(tool => {
    map[tool.id] = tool.name
  })
  return map
})

// Function to sort tool IDs by their names
const sortToolIdsByName = (toolIds) => {
  if (!toolIds || !Array.isArray(toolIds)) return []
  return [...toolIds].sort((a, b) => {
    const nameA = toolNameMap.value[a] || ''
    const nameB = toolNameMap.value[b] || ''
    return nameA.localeCompare(nameB, 'zh-Hans')
  })
}

// Watch for availableTools array changes to maintain sorting
watch(() => agentForm.value.availableTools, (newVal) => {
  if (!newVal || !Array.isArray(newVal)) return

  const sorted = sortToolIdsByName(newVal)
  // Check if sorting is needed
  let needsSorting = false
  if (sorted.length !== newVal.length) {
    needsSorting = true
  } else {
    for (let i = 0; i < sorted.length; i++) {
      if (sorted[i] !== newVal[i]) {
        needsSorting = true
        break
      }
    }
  }

  if (needsSorting) {
    // Use nextTick to avoid modifying data during render
    nextTick(() => {
      agentForm.value.availableTools = sorted
    })
  }
}, { deep: true })

// Watch for autoApprove array changes to maintain sorting
watch(() => agentForm.value.autoApprove, (newVal) => {
  if (!newVal || !Array.isArray(newVal)) return

  const sorted = sortToolIdsByName(newVal)
  // Check if sorting is needed
  let needsSorting = false
  if (sorted.length !== newVal.length) {
    needsSorting = true
  } else {
    for (let i = 0; i < sorted.length; i++) {
      if (sorted[i] !== newVal[i]) {
        needsSorting = true
        break
      }
    }
  }

  if (needsSorting) {
    // Use nextTick to avoid modifying data during render
    nextTick(() => {
      agentForm.value.autoApprove = sorted
    })
  }
}, { deep: true })

const DEFAULT_SHELL_POLICIES = [
  { pattern: '^ls($| .*)', decision: 'allow' },
  { pattern: '^pwd$', decision: 'allow' },
  { pattern: '^cat .*', decision: 'allow' },
  { pattern: '^git status$', decision: 'allow' },
  { pattern: '^git log($| .*)', decision: 'allow' },
  { pattern: '^git diff($| .*)', decision: 'allow' },
  { pattern: '^grep .*', decision: 'allow' },
  { pattern: '^find .*', decision: 'allow' },
  { pattern: '^file($| .*)', decision: 'allow' },
  { pattern: '^stat($| .*)', decision: 'allow' },
  { pattern: '^head($| .*)', decision: 'allow' },
  { pattern: '^tail($| .*)', decision: 'allow' },
  { pattern: '^wc($| .*)', decision: 'allow' },
  { pattern: '^du($| .*)', decision: 'allow' },
  { pattern: '^df($| .*)', decision: 'allow' },
  { pattern: '^ps($| .*)', decision: 'allow' },
  { pattern: '^free($| .*)', decision: 'allow' },
  { pattern: '^uname($| .*)', decision: 'allow' },
  { pattern: '^whoami$', decision: 'allow' },
  { pattern: '^id($| .*)', decision: 'allow' },
  { pattern: '^env$', decision: 'allow' },
  { pattern: '^printenv($| .*)', decision: 'allow' },
  { pattern: '^date($| .*)', decision: 'allow' },
  { pattern: '^cal($| .*)', decision: 'allow' },
  { pattern: '^which($| .*)', decision: 'allow' },
  { pattern: '^whereis($| .*)', decision: 'allow' },
  { pattern: '^type($| .*)', decision: 'allow' },
  { pattern: '^command($| .*)', decision: 'allow' },
  { pattern: '^hostname$', decision: 'allow' },
  { pattern: '^nproc$', decision: 'allow' },
  { pattern: '^lscpu$', decision: 'allow' },
  { pattern: '^lsmod$', decision: 'allow' },
  { pattern: '^lsusb$', decision: 'allow' },
  { pattern: '^lspci$', decision: 'allow' },
  { pattern: '^lsblk($| .*)', decision: 'allow' },
  { pattern: '^blkid($| .*)', decision: 'allow' },
  { pattern: '^mount($| .*)', decision: 'allow' },
  { pattern: '^getfacl($| .*)', decision: 'allow' },
  { pattern: '^md5sum($| .*)', decision: 'allow' },
  { pattern: '^sha256sum($| .*)', decision: 'allow' },
  { pattern: '^base64($| .*)', decision: 'allow' },
  { pattern: '^hexdump($| .*)', decision: 'allow' },
  { pattern: '^od($| .*)', decision: 'allow' },
  { pattern: '^git show($| .*)', decision: 'allow' },
  { pattern: '^git branch($| .*)', decision: 'allow' },
  { pattern: '^git remote($| .*)', decision: 'allow' },
  { pattern: '^git tag($| .*)', decision: 'allow' },
  { pattern: '^git rev-parse($| .*)', decision: 'allow' },
  { pattern: '^git config --list($| .*)', decision: 'allow' },
  { pattern: '^docker ps($| .*)', decision: 'allow' },
  { pattern: '^docker images($| .*)', decision: 'allow' },
  { pattern: '^docker inspect($| .*)', decision: 'allow' },
  { pattern: '^systemctl status($| .*)', decision: 'allow' },
  { pattern: '^iptables -L($| .*)', decision: 'allow' },
  { pattern: '^ufw status($| .*)', decision: 'allow' },
  { pattern: '^ss($| .*)', decision: 'allow' },
  { pattern: '^netstat($| .*)', decision: 'allow' },
  { pattern: '^ping($| .*)', decision: 'allow' },
  { pattern: '^traceroute($| .*)', decision: 'allow' },
  { pattern: '^dig($| .*)', decision: 'allow' },
  { pattern: '^nslookup($| .*)', decision: 'allow' },
  { pattern: '^tar -t.*', decision: 'allow' },
  { pattern: '^zip -l($| .*)', decision: 'allow' },
  { pattern: '^unzip -l($| .*)', decision: 'allow' },
]


const addShellPolicyRule = () => {
  if (!agentForm.value.shellPolicy) agentForm.value.shellPolicy = []
  agentForm.value.shellPolicy.push({ pattern: '', decision: 'review' })

  // Use setTimeout to avoid ResizeObserver loop errors
  // Wait for Vue's DOM update to complete
  nextTick(() => {
    // Use requestAnimationFrame to ensure DOM is fully rendered
    requestAnimationFrame(() => {
      if (shellPolicyListRef.value) {
        // Scroll to bottom
        shellPolicyListRef.value.scrollTop = shellPolicyListRef.value.scrollHeight

        // Focus the pattern input field of the last rule
        // Use another microtask to ensure scrolling is complete
        setTimeout(() => {
          const patternInputs = shellPolicyListRef.value.querySelectorAll('.shell-policy-item .el-input:first-child input')
          if (patternInputs.length > 0) {
            const lastPatternInput = patternInputs[patternInputs.length - 1]
            lastPatternInput.focus()
          }
        }, 0)
      }
    })
  })
}

const addAuthorizedPath = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.agent.selectDirectory')
    })
    if (selected) {
      if (!agentForm.value.allowedPaths) agentForm.value.allowedPaths = []
      if (!agentForm.value.allowedPaths.includes(selected)) {
        agentForm.value.allowedPaths.push(selected)
      }
    }
  } catch (error) {
    console.error('Failed to open directory dialog:', error)
  }
}

const removeAuthorizedPath = index => {
  agentForm.value.allowedPaths.splice(index, 1)
}

const removeShellPolicyRule = index => {
  agentForm.value.shellPolicy.splice(index, 1)
}

const clearShellPolicyRules = () => {
  ElMessageBox.confirm(
    t('settings.agent.shellPolicyClearConfirm'),
    t('settings.agent.shellPolicyClearTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  ).then(() => {
    agentForm.value.shellPolicy = []
  })
}

const importDefaultShellPolicies = () => {
  ElMessageBox.confirm(
    t('settings.agent.shellPolicyImportDefaultConfirm'),
    t('settings.agent.shellPolicyImportDefaultTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'info'
    }
  ).then(() => {
    if (!agentForm.value.shellPolicy) agentForm.value.shellPolicy = []
    // Add default policies if not already present
    DEFAULT_SHELL_POLICIES.forEach(defaultRule => {
      const exists = agentForm.value.shellPolicy.some(rule =>
        rule.pattern === defaultRule.pattern && rule.decision === defaultRule.decision
      )
      if (!exists) {
        agentForm.value.shellPolicy.push({ ...defaultRule })
      }
    })
  })
}

const agentRules = {
  name: [{ required: true, message: t('settings.agent.nameRequired') }],
  systemPrompt: [{ required: true, message: t('settings.agent.systemPromptRequired') }]
}

const getModelList = key => {
  const id = agentForm.value[key + 'Model']?.id
  return id ? modelStore.getModelProviderById(id)?.models || [] : []
}

const onModelIdChange = key => {
  agentForm.value[key + 'Model'].model = ''
}

const getProxyAliases = groupName => {
  if (!groupName) return []
  const groupData = settingStore.settings.chatCompletionProxy[groupName]
  return groupData ? Object.keys(groupData) : []
}

const onProxyGroupChange = key => {
  proxyAliases[key] = ''
}

const onProxyAliasChange = (key, value) => {
  agentForm.value[key + 'Model'].model = `${proxyGroups[key]}@${value}`
}

const parseModelField = (field, key) => {
  if (field && field.id === 0 && field.model?.includes('@')) {
    modelModes[key] = 'proxy'
    const [group, ...rest] = field.model.split('@')
    proxyGroups[key] = group
    proxyAliases[key] = rest.join('@')
  } else {
    modelModes[key] = 'provider'
    proxyGroups[key] = ''
    proxyAliases[key] = ''
  }
}

const editAgent = async id => {
  formRef.value?.resetFields()
  activeTab.value = 'basic'

  if (id) {
    try {
      const agentData = await agentStore.getAgent(id)
      if (!agentData) return
      editId.value = id
      agentForm.value = { ...defaultFormData, ...agentData }

      // Ensure tool arrays are sorted by name
      if (agentForm.value.availableTools && Array.isArray(agentForm.value.availableTools)) {
        agentForm.value.availableTools = sortToolIdsByName(agentForm.value.availableTools)
      }
      if (agentForm.value.autoApprove && Array.isArray(agentForm.value.autoApprove)) {
        agentForm.value.autoApprove = sortToolIdsByName(agentForm.value.autoApprove)
      }

      // Unpack unified 'models' JSON field if it exists
      if (agentData.models) {
        try {
          const modelsObj = JSON.parse(agentData.models)
          modelRoles.forEach(role => {
            if (modelsObj[role.key]) {
              agentForm.value[role.key + 'Model'] = modelsObj[role.key]
              // Ensure temperature exists
              if (agentForm.value[role.key + 'Model'].temperature === undefined) {
                agentForm.value[role.key + 'Model'].temperature = -0.1
              }
            }
          })
        } catch (e) { console.error(e) }
      }

      // Unpack 'shellPolicy' JSON field if it exists
      if (agentData.shellPolicy) {
        try {
          // Handle both stringified JSON and already parsed array
          if (typeof agentData.shellPolicy === 'string' && agentData.shellPolicy.trim()) {
            const policyObj = JSON.parse(agentData.shellPolicy)
            if (Array.isArray(policyObj)) {
              agentForm.value.shellPolicy = policyObj
            }
          } else if (Array.isArray(agentData.shellPolicy)) {
            // Already an array, use directly
            agentForm.value.shellPolicy = agentData.shellPolicy
          }
        } catch (e) {
          console.error('Failed to parse shellPolicy JSON:', e)
          // Fallback to default policies
          agentForm.value.shellPolicy = [...DEFAULT_SHELL_POLICIES]
        }
      } else {
        // No shell policy, use defaults
        agentForm.value.shellPolicy = [...DEFAULT_SHELL_POLICIES]
      }

      // Unpack 'allowedPaths' JSON field if it exists
      const rawPaths = agentData.allowed_paths || agentData.allowedPaths;
      if (rawPaths) {
        try {
          if (typeof rawPaths === 'string' && rawPaths.trim()) {
            const pathsObj = JSON.parse(rawPaths)
            if (Array.isArray(pathsObj)) {
              agentForm.value.allowedPaths = pathsObj
            }
          } else if (Array.isArray(rawPaths)) {
            agentForm.value.allowedPaths = rawPaths
          }
        } catch (e) {
          console.error('Failed to parse allowedPaths JSON:', e)
          agentForm.value.allowedPaths = []
        }
      } else {
        agentForm.value.allowedPaths = []
      }

      modelRoles.forEach(role => parseModelField(agentForm.value[role.key + 'Model'], role.key))
    } catch (error) { showMessage(t('settings.agent.fetchFailed'), 'error') }
  } else {
    editId.value = null
    agentForm.value = { ...defaultFormData }
    modelRoles.forEach(role => modelModes[role.key] = 'provider')
    agentForm.value.availableTools = availableTools.value.map(tool => tool.id)
    agentForm.value.autoApprove = availableTools.value.filter(tool => READ_ONLY_TOOLS.includes(tool.id)).map(tool => tool.id)
    agentForm.value.shellPolicy = [...DEFAULT_SHELL_POLICIES]
    agentForm.value.allowedPaths = []
  }

  agentDialogVisible.value = true
}

const copyAgent = async id => {
  try {
    const agentData = await agentStore.getAgent(id)
    if (!agentData) return
    agentForm.value = { ...defaultFormData, ...agentData }
    editId.value = null
    if (agentData.models) {
      try {
        const modelsObj = JSON.parse(agentData.models)
        modelRoles.forEach(role => { if (modelsObj[role.key]) agentForm.value[role.key + 'Model'] = modelsObj[role.key] })
      } catch (e) { console.error(e) }
    }

    // Unpack 'shellPolicy' JSON field if it exists
    if (agentData.shellPolicy) {
      try {
        // Handle both stringified JSON and already parsed array
        if (typeof agentData.shellPolicy === 'string' && agentData.shellPolicy.trim()) {
          const policyObj = JSON.parse(agentData.shellPolicy)
          if (Array.isArray(policyObj)) {
            agentForm.value.shellPolicy = policyObj
          }
        } else if (Array.isArray(agentData.shellPolicy)) {
          // Already an array, use directly
          agentForm.value.shellPolicy = agentData.shellPolicy
        }
      } catch (e) {
        console.error('Failed to parse shellPolicy JSON during copy:', e)
        // Fallback to default policies
        agentForm.value.shellPolicy = [...DEFAULT_SHELL_POLICIES]
      }
    } else {
      // No shell policy, use defaults
      agentForm.value.shellPolicy = [...DEFAULT_SHELL_POLICIES]
    }

    // Unpack 'allowedPaths' JSON field if it exists
    if (agentData.allowedPaths) {
      try {
        if (typeof agentData.allowedPaths === 'string' && agentData.allowedPaths.trim()) {
          const pathsObj = JSON.parse(agentData.allowedPaths)
          if (Array.isArray(pathsObj)) {
            agentForm.value.allowedPaths = pathsObj
          }
        } else if (Array.isArray(agentData.allowedPaths)) {
          agentForm.value.allowedPaths = agentData.allowedPaths
        }
      } catch (e) {
        console.error('Failed to parse allowedPaths JSON during copy:', e)
        agentForm.value.allowedPaths = []
      }
    } else {
      agentForm.value.allowedPaths = []
    }

    modelRoles.forEach(role => parseModelField(agentForm.value[role.key + 'Model'], role.key))
    agentDialogVisible.value = true
  } catch (error) { showMessage(t('settings.agent.fetchFailed'), 'error') }
}

const updateAgent = () => {
  formRef.value.validate(async valid => {
    if (valid) {
      const finalForm = JSON.parse(JSON.stringify(agentForm.value))

      // 1. Filter out empty shell policy rules
      if (finalForm.shellPolicy && Array.isArray(finalForm.shellPolicy)) {
        finalForm.shellPolicy = finalForm.shellPolicy.filter(rule =>
          rule.pattern && rule.pattern.trim() !== ''
        )
      }

      // 2. Prepare models
      modelRoles.forEach(role => {
        if (modelModes[role.key] === 'proxy') {
          finalForm[role.key + 'Model'].id = 0
          finalForm[role.key + 'Model'].model = `${proxyGroups[role.key]}@${proxyAliases[role.key]}`
        }
      })

      try {
        await agentStore.saveAgent({ ...finalForm, id: editId.value })
        showMessage(t(editId.value ? 'settings.agent.updateSuccess' : 'settings.agent.addSuccess'), 'success')
        agentDialogVisible.value = false
        // Refresh the agents list from the store to update the UI
        await agentStore.fetchAgents()
      } catch (error) { showMessage(t('settings.agent.saveFailed'), 'error') }
    }
  })
}

const deleteAgent = id => {
  ElMessageBox.confirm(t('settings.agent.deleteConfirm'), t('settings.agent.deleteTitle'), {
    confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel'), type: 'warning'
  }).then(async () => {
    try {
      await agentStore.deleteAgent(id)
      showMessage(t('settings.agent.deleteSuccess'), 'success')
    } catch (error) { showMessage(t('settings.agent.deleteFailed'), 'error') }
  })
}

const onDragEnd = () => {
  agentStore.updateAgentOrder(agents.value).catch(() => {
    showMessage(t('settings.agent.reorderFailed'), 'error')
    agentStore.fetchAgents()
  })
}

const onAgentDialogClose = () => {
  // Reset active tab to basic when dialog closes
  activeTab.value = 'basic'
  // Clear form validation errors
  formRef.value?.resetFields()
}

onMounted(() => {
  modelStore.updateModelStore()
  proxyGroupStore.getList()
})
</script>

<style lang="scss">
.agent-edit-dialog {
  .el-dialog__header {
    display: none;
  }

  .el-tabs__nav-wrap:after {
    background-color: var(--cs-border-color);
  }

  .models-layout {
    padding: 4px;
  }

  .model-item-compact {
    margin-bottom: 12px;
    padding: 8px;
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    background-color: var(--cs-bg-color-light);

    .header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 8px;

      .title {
        font-weight: bold;
        font-size: 13px;
        color: var(--cs-text-color-primary);
      }
    }

    .body {
      display: flex;
      flex-direction: column;

      .selectors-row {
        display: flex;
        gap: 4px;
      }

      .params-row {
        display: flex;
        align-items: center;
        gap: 8px;

        &.compact-params {
          justify-content: space-between;
          padding: 0 4px;

          .param-item {
            display: flex;
            align-items: center;
            gap: 4px;
          }
        }

        .param-label {
          font-size: 11px;
          color: var(--cs-text-color-secondary);
          white-space: nowrap;
        }
      }
    }
  }

  .danger-option {
    color: var(--el-color-danger) !important;
    font-weight: bold;
  }

  .security-group {
    margin-bottom: var(--cs-space-lg);
    display: block;

    .shell-policy-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: var(--cs-space-sm);

      h3 {
        margin: 0;
        font-size: var(--cs-font-size-md);
        color: var(--cs-text-color-primary);
      }

      .shell-policy-actions {
        display: flex;
        gap: var(--cs-space-sm);
        align-items: center;
      }
    }
  }

  .security-tip {
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    margin-bottom: 12px;
    margin-top: -8px;
    line-height: 1.4;
  }

  .shell-policy-list {
    max-height: 300px;
    overflow-y: auto;
    padding-right: 4px;
    margin-top: var(--cs-space-sm);

    /* Custom scrollbar */
    &::-webkit-scrollbar {
      width: 6px;
    }

    &::-webkit-scrollbar-track {
      background: var(--cs-bg-color-light);
      border-radius: 3px;
    }

    &::-webkit-scrollbar-thumb {
      background: var(--cs-border-color);
      border-radius: 3px;

      &:hover {
        background: var(--cs-text-color-secondary);
      }
    }

    .shell-policy-item {
      display: flex;
      gap: var(--cs-space-sm);
      margin-bottom: var(--cs-space-sm);
      align-items: center;
    }
  }
}
</style>
