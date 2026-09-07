<template>
  <el-dialog
    :model-value="modelValue"
    :title="$t('settings.mcp.executeDialog.title')"
    width="680px"
    align-center
    class="mcp-execute-dialog"
    :close-on-click-modal="false"
    @update:model-value="value => emit('update:modelValue', value)"
    @open="handleOpen"
    @closed="handleClosed">
    <div class="execute-body">
      <!-- MCP server selector -->
      <div class="select-row">
        <div class="select-label">{{ $t('settings.mcp.executeDialog.server') }}</div>
        <el-select
          v-model="selectedServerId"
          filterable
          :placeholder="$t('settings.mcp.executeDialog.selectServer')"
          :disabled="running"
          style="width: 100%">
          <el-option
            v-for="server in runningServers"
            :key="server.id"
            :label="server.name"
            :value="server.id" />
        </el-select>
      </div>

      <!-- MCP tool selector -->
      <div class="select-row">
        <div class="select-label">{{ $t('settings.mcp.executeDialog.tool') }}</div>
        <el-select
          v-model="selectedToolName"
          filterable
          :placeholder="$t('settings.mcp.executeDialog.selectTool')"
          :disabled="!selectedServerId || running"
          :loading="loadingTools"
          style="width: 100%">
          <el-option
            v-for="tool in availableTools"
            :key="tool.name"
            :label="tool.name"
            :value="tool.name">
            <div class="tool-option">
              <div class="tool-option-name">{{ tool.name }}</div>
              <div class="tool-option-desc">{{ tool.description }}</div>
            </div>
          </el-option>
        </el-select>
      </div>

      <div v-if="selectedTool && selectedTool.description" class="tool-description">
        {{ selectedTool.description }}
      </div>

      <!-- Parameters -->
      <template v-if="selectedTool">
        <div class="params-title">
          {{ $t('settings.mcp.executeDialog.params') }}
        </div>
        <el-form v-if="paramDefs.length" label-position="top" class="params-form">
          <el-form-item
            v-for="def in paramDefs"
            :key="def.key"
            :label="def.key"
            :required="def.required">
            <template #label>
              <span class="param-label">
                <span>{{ def.key }}</span>
                <span v-if="def.required" class="required-star">*</span>
              </span>
            </template>

            <el-select
              v-if="def.enum"
              v-model="paramValues[def.key]"
              filterable
              :placeholder="def.description || def.key"
              style="width: 100%">
              <el-option
                v-for="option in def.enum"
                :key="String(option)"
                :label="String(option)"
                :value="option" />
            </el-select>

            <el-switch
              v-else-if="def.type === 'boolean'"
              v-model="paramValues[def.key]"
              :disabled="running" />

            <el-input-number
              v-else-if="def.type === 'number' || def.type === 'integer'"
              v-model="paramValues[def.key]"
              :min="def.minimum"
              :max="def.maximum"
              :step="def.type === 'integer' ? 1 : undefined"
              :controls="false"
              :disabled="running"
              style="width: 100%" />

            <el-input
              v-else-if="def.type === 'array' || def.type === 'object'"
              v-model="paramValues[def.key]"
              type="textarea"
              :rows="3"
              :disabled="running"
              :placeholder="def.type === 'array' ? '[...]' : '{...}'" />

            <el-input
              v-else
              v-model="paramValues[def.key]"
              :type="def.format === 'password' ? 'password' : 'text'"
              :disabled="running"
              :placeholder="def.description || def.key" />

            <div v-if="def.description" class="param-desc">{{ def.description }}</div>
          </el-form-item>
        </el-form>
        <div v-else class="params-empty">{{ $t('settings.mcp.executeDialog.noParams') }}</div>
      </template>
      <el-empty
        v-else
        :description="$t('settings.mcp.executeDialog.selectToolFirst')"
        :image-size="60" />

      <!-- Run button -->
      <div class="run-row">
        <el-button
          type="primary"
          :loading="running"
          :disabled="!selectedServerId || !selectedToolName"
          @click="run">
          <span>{{ $t('settings.mcp.executeDialog.run') }}</span>
        </el-button>
      </div>

      <!-- Result -->
      <div class="result-block">
        <div class="result-header">
          <span class="result-title">{{ $t('settings.mcp.executeDialog.result') }}</span>
          <el-button
            v-if="resultText"
            size="small"
            text
            type="primary"
            @click="copyResult">
            {{ $t('common.copy') }}
          </el-button>
        </div>
        <el-input
          v-if="resultText"
          type="textarea"
          :model-value="resultText"
          readonly
          :rows="10"
          resize="vertical"
          class="result-textarea" />
        <el-input
          v-else-if="errorText"
          type="textarea"
          :model-value="errorText"
          readonly
          :rows="6"
          resize="vertical"
          class="error-textarea" />
        <div v-else class="result-empty">{{ $t('settings.mcp.executeDialog.noResult') }}</div>
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { writeClipboard } from '@/libs/clipboard'
import { FrontendAppError } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useMcpStore } from '@/stores/mcp'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  server: { type: Object, default: null }
})
const emit = defineEmits(['update:modelValue'])

const { t } = useI18n()
const mcpStore = useMcpStore()

const selectedServerId = ref(null)
const selectedToolName = ref(null)
const loadingTools = ref(false)
const running = ref(false)
const resultText = ref('')
const errorText = ref('')
const paramValues = ref({})

const runningServers = computed(() =>
  mcpStore.servers.filter(server => server.status === 'running')
)

const selectedServer = computed(
  () => mcpStore.servers.find(server => server.id === selectedServerId.value) || null
)

const availableTools = computed(() => {
  const tools = mcpStore.serverTools[selectedServerId.value] || []
  const disabledTools = selectedServer.value?.config?.disabled_tools || []
  return tools.filter(tool => !disabledTools.includes(tool.name))
})

const selectedTool = computed(() => {
  if (!selectedToolName.value) return null
  const tools = mcpStore.serverTools[selectedServerId.value] || []
  return tools.find(tool => tool.name === selectedToolName.value) || null
})

const paramDefs = computed(() => {
  const tool = selectedTool.value
  const schema = tool?.inputSchema || tool?.input_schema
  if (!schema || typeof schema !== 'object') return []
  const properties = schema.properties || {}
  const required = Array.isArray(schema.required) ? schema.required : []
  return Object.entries(properties).map(([key, spec]) => ({
    key,
    type: typeof spec?.type === 'string' ? spec.type : 'string',
    required: required.includes(key),
    description: typeof spec?.description === 'string' ? spec.description : '',
    enum: Array.isArray(spec?.enum) ? spec.enum : null,
    default: spec && 'default' in spec ? spec.default : undefined,
    minimum: typeof spec?.minimum === 'number' ? spec.minimum : undefined,
    maximum: typeof spec?.maximum === 'number' ? spec.maximum : undefined,
    format: typeof spec?.format === 'string' ? spec.format : ''
  }))
})

const toolsFetching = new Set()

const ensureToolsFetched = async serverId => {
  if (!serverId || mcpStore.serverTools[serverId] || toolsFetching.has(serverId)) return
  toolsFetching.add(serverId)
  loadingTools.value = true
  try {
    await mcpStore.fetchMcpServerTools(serverId)
  } catch (e) {
    console.error('Error fetching MCP server tools:', e)
  } finally {
    toolsFetching.delete(serverId)
    loadingTools.value = false
  }
}

const resetParams = () => {
  const values = {}
  for (const def of paramDefs.value) {
    if (def.default !== undefined) {
      values[def.key] = def.default
    } else if (def.type === 'boolean') {
      values[def.key] = false
    } else {
      values[def.key] = ''
    }
  }
  paramValues.value = values
}

const resetResult = () => {
  resultText.value = ''
  errorText.value = ''
}

const handleOpen = async () => {
  resetResult()
  const runningIds = new Set(runningServers.value.map(server => server.id))
  const initialId =
    props.server?.id && runningIds.has(props.server.id)
      ? props.server.id
      : runningServers.value[0]?.id || null
  selectedServerId.value = initialId
  selectedToolName.value = null
  await ensureToolsFetched(initialId)
}

const handleClosed = () => {
  selectedServerId.value = null
  selectedToolName.value = null
  paramValues.value = {}
  resetResult()
}

watch(selectedServerId, async newId => {
  selectedToolName.value = null
  resetResult()
  await ensureToolsFetched(newId)
})

watch(selectedToolName, () => {
  resetParams()
  resetResult()
})

const buildArguments = () => {
  const args = {}
  for (const def of paramDefs.value) {
    const value = paramValues.value[def.key]
    if (def.type === 'boolean') {
      args[def.key] = !!value
    } else if (value !== '' && value !== null && value !== undefined) {
      args[def.key] = value
    }
  }
  return args
}

const validateRequired = () => {
  for (const def of paramDefs.value) {
    if (!def.required) continue
    const value = paramValues.value[def.key]
    if (value === '' || value === null || value === undefined) {
      return false
    }
  }
  return true
}

const formatResult = raw => {
  if (raw && typeof raw === 'object') {
    const content = raw.content
    if (Array.isArray(content)) {
      const texts = content
        .filter(item => item && item.type === 'text' && typeof item.text === 'string')
        .map(item => item.text)
      if (texts.length) return texts.join('\n')
    }
    return JSON.stringify(raw, null, 2)
  }
  return typeof raw === 'string' ? raw : JSON.stringify(raw, null, 2)
}

const run = async () => {
  if (!selectedServerId.value) {
    showMessage(t('settings.mcp.executeDialog.selectServerFirst'), 'warning')
    return
  }
  if (!selectedToolName.value) {
    showMessage(t('settings.mcp.executeDialog.selectToolFirst'), 'warning')
    return
  }
  if (!validateRequired()) {
    showMessage(t('settings.mcp.executeDialog.paramsRequired'), 'warning')
    return
  }

  running.value = true
  resetResult()
  try {
    const rawResult = await mcpStore.runMcpTool(
      selectedServerId.value,
      selectedToolName.value,
      buildArguments()
    )
    resultText.value = formatResult(rawResult)
    showMessage(t('settings.mcp.executeDialog.executeSuccess'), 'success')
  } catch (e) {
    errorText.value =
      e instanceof FrontendAppError ? e.toFormattedString() : e?.message || String(e)
    console.error('Error running MCP tool:', e)
  } finally {
    running.value = false
  }
}

const copyResult = async () => {
  try {
    await writeClipboard(resultText.value)
    showMessage(t('common.copied'), 'success')
  } catch (e) {
    console.error('Failed to copy result:', e)
    showMessage(t('common.operationFailed', { error: e?.message || String(e) }), 'error')
  }
}
</script>

<style lang="scss" scoped>
.execute-body {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space);

  .select-row {
    display: flex;
    align-items: center;
    gap: var(--cs-space);

    .select-label {
      flex-shrink: 0;
      width: 110px;
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-secondary);
    }
  }

  .tool-description {
    font-size: var(--cs-font-size-xs);
    color: var(--cs-text-color-secondary);
    line-height: 1.5;
    word-break: break-all;
  }

  .tool-option {
    display: flex;
    flex-direction: column;
    line-height: 1.4;

    .tool-option-name {
      font-weight: 500;
    }

    .tool-option-desc {
      font-size: var(--cs-font-size-xs);
      color: var(--cs-text-color-secondary);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      max-width: 480px;
    }
  }

  .params-title {
    font-size: var(--cs-font-size-sm);
    font-weight: 600;
    color: var(--cs-text-color-primary);
    margin-top: var(--cs-space-xs);
  }

  .params-form {
    max-height: 260px;
    overflow-y: auto;
    padding-right: var(--cs-space-xs);

    :deep(.el-form-item) {
      margin-bottom: var(--cs-space);
    }

    :deep(.el-form-item__label) {
      line-height: 1.4;
      padding-bottom: 4px;
    }

    .param-label {
      display: inline-flex;
      align-items: center;
      gap: 2px;
    }

    .required-star {
      color: var(--cs-error-color, var(--el-color-danger));
    }

    .param-desc {
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-secondary);
      line-height: 1.5;
      margin-top: 4px;
      word-break: break-all;
    }
  }

  .params-empty {
    font-size: var(--cs-font-size-sm);
    color: var(--cs-text-color-secondary);
    padding: var(--cs-space-sm) 0;
  }

  .run-row {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--cs-space-xs);
  }

  .result-block {
    .result-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: var(--cs-space-xs);

      .result-title {
        font-size: var(--cs-font-size-sm);
        font-weight: 600;
        color: var(--cs-text-color-primary);
      }
    }

    .result-textarea {
      :deep(textarea) {
        font-family: monospace;
        font-size: var(--cs-font-size-xs);
        line-height: 1.5;
      }
    }

    .error-textarea {
      :deep(textarea) {
        font-family: monospace;
        font-size: var(--cs-font-size-xs);
        line-height: 1.5;
        color: var(--cs-error-color, var(--el-color-danger));
      }
    }

    .result-empty {
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-tertiary);
      padding: var(--cs-space-sm) 0;
      text-align: center;
    }
  }
}
</style>
