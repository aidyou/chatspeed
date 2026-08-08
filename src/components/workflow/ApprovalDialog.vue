<template>
  <el-dialog
    v-if="!inline"
    :model-value="modelValue"
    :title="t('workflow.approval.title')"
    width="560px"
    append-to-body
    class="approval-dialog"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :show-close="false"
    @update:model-value="value => emit('update:modelValue', value)">
    <div class="approval-content">
      <div class="details-box">
        <div v-if="isShellAction" class="shell-view">
          <div class="shell-command-frame">
            <span class="shell-command-frame__prompt" aria-hidden="true">$</span>
            <MarkdownSimple :content="shellMarkdown" class-name="approval-markdown" />
          </div>
          <div v-if="shellExecutionRows.length" class="shell-execution-plan">
            <div v-for="row in shellExecutionRows" :key="row.label" class="shell-execution-plan__row">
              <span class="shell-execution-plan__label">{{ row.label }}</span>
              <span class="shell-execution-plan__value">{{ row.value }}</span>
            </div>
          </div>
        </div>
        <pre v-else class="details-text">{{ detailPayload.detailsText }}</pre>
      </div>
    </div>
    <template #footer>
      <el-button @click="onReject" :loading="loading">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" @click="onApprove" :loading="loading">{{ $t('common.confirm') }}</el-button>
    </template>
  </el-dialog>

  <div v-if="inline" class="approval-inline-panel" :class="{ 'diff-dialog': isEditAction }">
    <div class="approval-content">
      <div class="details-box" :class="{ 'plan-details-box': isPlanApproval }">
        <div v-if="isPlanApproval" class="plan-details-actions">
          <button
            type="button"
            class="copy-icon-button"
            :title="$t('common.copy')"
            :aria-label="$t('common.copy')"
            @click="copyPlanMarkdown">
            <cs name="copy" />
          </button>
        </div>
        <div v-if="isEditAction" class="diff-view">
          <FilePreviewDiff
            :file-path="filePath"
            :old-content="detailsObject?.old_string || ''"
            :new-content="detailsObject?.new_string ?? detailsObject?.content ?? ''"
            :context-data="detailsObject || null" />
        </div>
        <div v-else-if="isShellAction" class="shell-view">
          <div class="shell-command-frame">
            <span class="shell-command-frame__prompt" aria-hidden="true">$</span>
            <MarkdownSimple :content="shellMarkdown" class-name="approval-markdown" />
          </div>
          <div v-if="shellExecutionRows.length" class="shell-execution-plan">
            <div v-for="row in shellExecutionRows" :key="row.label" class="shell-execution-plan__row">
              <span class="shell-execution-plan__label">{{ row.label }}</span>
              <span class="shell-execution-plan__value">{{ row.value }}</span>
            </div>
          </div>
        </div>
        <div v-else-if="isMarkdownAction" class="markdown-view">
          <MarkdownSimple :content="markdownContent" class-name="approval-markdown" />
        </div>
        <pre v-else class="details-text">{{ detailPayload.detailsText }}</pre>
      </div>
      <div class="rejection-note-box">
        <div class="note-header">{{ $t('workflow.approval.rejectionMessageLabel') }}</div>
        <el-input
          :model-value="rejectionMessage"
          type="textarea"
          :autosize="{ minRows: 1, maxRows: 6 }"
          resize="none"
          :placeholder="$t('workflow.approval.rejectionMessagePlaceholder')"
          @update:model-value="value => emit('update:rejectionMessage', value)" />
      </div>
      <div class="dialog-footer inline-footer">
        <el-button @click="onReject" :loading="loading" round>{{ $t('common.reject') }}</el-button>
        <el-button type="primary" @click="onApprove" :loading="loading" round>{{
          $t('common.approve')
        }}</el-button>
        <el-button
          v-if="!isPlanApproval"
          type="primary"
          @click="onApproveAll"
          :loading="loading"
          round
          >{{ $t('common.approveAndAddToWhitelist') }}</el-button
        >
        <el-button
          v-if="!isPlanApproval && pendingCount > 1"
          type="warning"
          @click="onApproveAllPending"
          :loading="loading"
          round
          >{{ $t('common.approveAll') }} ({{ pendingCount }})</el-button
        >
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import MarkdownSimple from '@/components/workflow/MarkdownSimple.vue'
import FilePreviewDiff from '@/components/workflow/FilePreviewDiff.vue'
import { formatPlanApprovalMarkdown } from '@/composables/workflow/planApproval'
import { writeClipboard } from '@/libs/clipboard'
import { showMessage } from '@/libs/util'

const props = defineProps({
  modelValue: Boolean,
  inline: {
    type: Boolean,
    default: false
  },
  toolName: String,
  target: {
    type: String,
    default: ''
  },
  details: {
    type: [String, Object, Array],
    default: ''
  },
  rejectionMessage: {
    type: String,
    default: ''
  },
  displayType: {
    type: String,
    default: ''
  },
  loading: Boolean,
  pendingCount: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits([
  'update:modelValue',
  'update:rejectionMessage',
  'approve',
  'approveAll',
  'approveAllPending',
  'reject'
])

const { t } = useI18n()

/**
 * Decode approval details that older records persisted as nested JSON strings.
 * This compatibility boundary is presentation-only. It must never supply tool
 * identity, approval state, or execution state; those come from structured
 * metadata and the current pending approval collection.
 */
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

const normalizeDetailsPayload = value => {
  if (value == null || value === '') {
    return { detailsObject: null, detailsText: '' }
  }

  if (typeof value === 'string') {
    const parsed = decodeCompatJsonPayload(value)
    const detailsObject = Array.isArray(parsed) ? parsed[0] || null : parsed
    return {
      detailsObject: detailsObject && typeof detailsObject === 'object' ? detailsObject : null,
      detailsText:
        detailsObject && typeof detailsObject === 'object'
          ? JSON.stringify(detailsObject, null, 2)
          : String(parsed ?? value)
    }
  }

  if (Array.isArray(value)) {
    const first = value[0] || null
    return {
      detailsObject: first && typeof first === 'object' ? first : null,
      detailsText: JSON.stringify(value, null, 2)
    }
  }

  if (typeof value === 'object') {
    return {
      detailsObject: value,
      detailsText: JSON.stringify(value, null, 2)
    }
  }

  return { detailsObject: null, detailsText: String(value) }
}

const normalizedToolName = computed(() => (props.toolName || '').toLowerCase().trim())
const detailPayload = computed(() => normalizeDetailsPayload(props.details))
const detailsObject = computed(() => detailPayload.value.detailsObject)

const isEditAction = computed(() => {
  if (props.displayType === 'diff') {
    return true
  }
  const toolName = normalizedToolName.value
  if (toolName === 'edit_file' || toolName === 'write_file') {
    return true
  }
  return false
})

const isShellAction = computed(() => normalizedToolName.value === 'bash')
const isPlanApproval = computed(() => normalizedToolName.value === 'submit_plan')
const isMarkdownAction = computed(() => {
  if (props.displayType === 'markdown') {
    return true
  }
  return isPlanApproval.value
})

const planMarkdown = computed(() => {
  const details = detailsObject.value
  return formatPlanApprovalMarkdown(details, t) || detailPayload.value.detailsText
})

const markdownContent = computed(() =>
  isPlanApproval.value ? planMarkdown.value : detailPayload.value.detailsText
)

const filePath = computed(() => {
  if (!isEditAction.value) return ''
  const data = detailsObject.value
  return data?.display_path || data?.file_path || data?.path || ''
})

// const visible = computed({
//   get: () => props.modelValue,
//   set: val => emit('update:modelValue', val)
// })

const rejectionMessage = computed(() => props.rejectionMessage || '')

const stripMarkdownFences = value => {
  if (typeof value !== 'string') return ''
  const trimmed = value.trim()
  if (!trimmed.startsWith('```')) return trimmed
  const withoutFence = trimmed.replace(/^```[a-zA-Z0-9_-]*\n?/, '').replace(/```$/, '')
  return withoutFence.trim()
}

const extractShellCommand = payload => {
  if (!payload) return ''

  const directCommand = payload.detailsObject?.command
  if (typeof directCommand === 'string' && directCommand.trim()) {
    return directCommand.trim()
  }

  const rawText = stripMarkdownFences(payload.detailsText || '')
  if (!rawText) return ''

  try {
    const parsed = JSON.parse(rawText)
    if (typeof parsed === 'string') {
      return parsed.trim()
    }
    if (parsed && typeof parsed.command === 'string') {
      return parsed.command.trim()
    }
  } catch {
    const braceStart = rawText.indexOf('{')
    const braceEnd = rawText.lastIndexOf('}')
    if (braceStart >= 0 && braceEnd > braceStart) {
      try {
        const parsed = JSON.parse(rawText.slice(braceStart, braceEnd + 1))
        if (parsed && typeof parsed.command === 'string') {
          return parsed.command.trim()
        }
      } catch {
        // Fall through to use the plain text as the command.
      }
    }
  }

  return rawText
}

const shellCommand = computed(() => extractShellCommand(detailPayload.value))
const shellMarkdown = computed(() => `\`\`\`bash\n${shellCommand.value || ''}\n\`\`\``)
const SHELL_EXECUTION_BACKEND_TRANSLATION_KEYS = {
  docker: 'workflow.approval.executionBackends.docker',
  host: 'workflow.approval.executionBackends.host',
  msb: 'workflow.approval.executionBackends.msb'
}
const SHELL_FALLBACK_REASON_TRANSLATION_KEYS = {
  ambiguous_route: 'workflow.approval.fallbackReasons.ambiguous_route',
  host_only_mode: 'workflow.approval.fallbackReasons.host_only_mode',
  missing_image: 'workflow.approval.fallbackReasons.missing_image',
  profile_unavailable: 'workflow.approval.fallbackReasons.profile_unavailable',
  runtime_unavailable: 'workflow.approval.fallbackReasons.runtime_unavailable',
  sandbox_config_missing: 'workflow.approval.fallbackReasons.sandbox_config_missing'
}
const formatShellValue = (value, translationKeys = null) => {
  if (value == null || value === '') return ''
  if (typeof value === 'string') {
    const translationKey = translationKeys?.[value]
    return translationKey ? t(translationKey) : value
  }
  if (Array.isArray(value)) return value.join(', ')
  if (typeof value === 'object') return JSON.stringify(value)
  return String(value)
}
const shellExecutionRows = computed(() => {
  const details = detailsObject.value?.execution_plan || detailsObject.value || {}
  const rows = [
    [
      'workflow.approval.executionBackend',
      details.execution_backend || details.backend,
      SHELL_EXECUTION_BACKEND_TRANSLATION_KEYS
    ],
    ['workflow.approval.runtime', details.runtime],
    ['workflow.approval.profile', details.sandbox_profile || details.profile],
    ['workflow.approval.image', details.sandbox_image || details.image],
    ['workflow.approval.network', details.network?.mode],
    ['workflow.approval.workspaceAccess', details.workspace_access],
    ['workflow.approval.limits', details.limits],
    [
      'workflow.approval.requiredCommandPatterns',
      detailsObject.value?.required_command_patterns
    ],
    [
      'workflow.approval.matchedCommandUnits',
      detailsObject.value?.matched_command_units
    ],
    [
      'workflow.approval.fallbackReason',
      details.fallback_reason,
      SHELL_FALLBACK_REASON_TRANSLATION_KEYS
    ]
  ]
  return rows
    .map(([labelKey, value, translationKeys]) => ({
      label: t(labelKey),
      value: formatShellValue(value, translationKeys)
    }))
    .filter(row => row.value)
})

const copyPlanMarkdown = async () => {
  try {
    await writeClipboard(planMarkdown.value)
    showMessage(t('common.copied'), 'success')
  } catch (error) {
    console.error('Failed to copy plan:', error)
    showMessage(t('common.operationFailed', { error: String(error) }), 'error')
  }
}

// const dialogWidth = computed(() => {
//   return isEditAction.value ? '90%' : '500px'
// })

const onApprove = () => {
  emit('approve')
}

const onApproveAll = () => {
  emit('approveAll')
}

const onApproveAllPending = () => {
  emit('approveAllPending')
}

const onReject = () => {
  emit('reject')
}
</script>

<style scoped lang="scss">
.approval-content {
  .details-box {
    background-color: var(--cs-bg-color-dark);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    padding: var(--cs-space-sm);
    margin-bottom: var(--cs-space-md);

    .details-text {
      margin: 0;
      white-space: pre-wrap;
      word-break: break-all;
      font-family: var(--cs-font-family-mono);
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-primary);
      max-height: min(28vh, 260px);
      overflow-y: auto;
    }

    .shell-view {
      display: grid;
      gap: var(--cs-space-sm);
      max-height: min(36vh, 320px);
      overflow: auto;
    }

    .markdown-view {
      max-height: min(36vh, 320px);
      overflow: auto;
    }

    .shell-command-frame {
      display: grid;
      grid-template-columns: auto minmax(0, 1fr);
      align-items: start;
      gap: var(--cs-space-sm);
      min-width: 0;

      &__prompt {
        color: var(--cs-color-primary);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size);
        font-weight: 700;
        line-height: 1.5;
        user-select: none;
      }

      :deep(.approval-markdown) {
        min-width: 0;
      }
    }

    .shell-view,
    .markdown-view {
      :deep(pre) {
        white-space: pre-wrap;
        word-break: break-word;
        overflow-wrap: anywhere;
        margin: 0;
      }

      :deep(pre code.hljs) {
        white-space: pre-wrap;
        word-break: break-word;
        overflow-wrap: anywhere;
        background: none;
        padding: var(--cs-space-sm);
      }
    }

    .shell-view .shell-command-frame {
      :deep(pre),
      :deep(pre code.hljs) {
        padding: 0;
        font-size: var(--cs-font-size);
        line-height: 1.5;
      }
    }

    &.plan-details-box {
      .plan-details-actions {
        display: flex;
        justify-content: flex-end;
        margin-bottom: var(--cs-space-xs);
      }

      .copy-icon-button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 26px;
        height: 26px;
        padding: 0;
        border: 0;
        border-radius: var(--cs-border-radius-sm);
        color: var(--cs-text-color-secondary);
        background: transparent;
        cursor: pointer;

        &:hover {
          color: var(--cs-text-color-primary);
          background: var(--cs-hover-bg-color);
        }
      }

      .markdown-view {
        max-height: none;
        overflow: visible;
      }
    }

    .shell-execution-plan {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: var(--cs-space-sm) var(--cs-space-md);
      padding-top: var(--cs-space-sm);
      border-top: 1px solid var(--cs-border-color);

      &__row {
        display: grid;
        gap: var(--cs-space-xxs);
        min-width: 0;
        font-size: var(--cs-font-size-sm);
      }

      &__label {
        color: var(--cs-text-color-secondary);
        font-size: var(--cs-font-size-xs);
        letter-spacing: 0.04em;
        line-height: 1.2;
        text-transform: uppercase;
      }

      &__value {
        overflow-wrap: anywhere;
        color: var(--cs-text-color-primary);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);
        line-height: 1.4;
      }
    }

    .diff-view {
      max-height: none;
      overflow: visible;
      border-radius: var(--cs-border-radius-sm);
      padding: 4px;

      .diff-file-path {
        margin-bottom: var(--cs-space-sm);
        color: var(--cs-text-color-secondary);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);
      }

      .diff-text {
        max-height: min(48vh, 520px);
        overflow: auto;
        background: var(--cs-bg-color-light);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);

        .diff-line {
          display: grid;
          grid-template-columns: 20px minmax(44px, auto) 16px minmax(0, 1fr);
          align-items: start;
          white-space: pre;

          &.added {
            background: var(--cs-diff-added-bg-color);
          }

          &.removed {
            background: var(--cs-diff-removed-bg-color);
          }

          &.context {
            color: var(--cs-text-color-primary);
          }
        }

        .diff-prefix,
        .diff-line-number,
        .diff-separator {
          user-select: none;
          opacity: 0.8;
          padding: 0 4px;
        }

        .diff-line.added .diff-prefix,
        .diff-line.removed .diff-prefix,
        .diff-line.context .diff-prefix {
          font-weight: 700;
          text-align: center;
        }

        .diff-line.added .diff-prefix::before {
          content: '+';
        }

        .diff-line.removed .diff-prefix::before {
          content: '-';
        }

        .diff-line.context .diff-prefix::before {
          content: ' ';
        }

        .diff-content {
          overflow-x: auto;
          padding-right: 8px;
        }

        :deep(.hljs) {
          background: transparent;
          padding: 0;
        }
      }
    }
  }

  .rejection-note-box {
    margin-bottom: var(--cs-space-sm);

    .note-header {
      font-size: var(--cs-font-size-xs);
      color: var(--cs-text-color-secondary);
      margin-bottom: var(--cs-space-xs);
      text-transform: uppercase;
    }
  }
}

.inline-footer {
  margin-top: var(--cs-space-sm);
}
</style>

<style lang="scss">
.approval-dialog.el-dialog {
  max-height: calc(100vh - var(--cs-titlebar-height, 32px) - 24px);
  display: flex;
  flex-direction: column;

  .el-dialog__body {
    overflow: hidden;
  }

  .approval-content {
    max-height: calc(100vh - var(--cs-titlebar-height, 32px) - 220px);
    overflow: auto;
  }
}

.diff-dialog-overlay.el-overlay {
  z-index: 2000 !important;
}

.diff-dialog-overlay .el-overlay-dialog {
  overflow: hidden !important;
  position: fixed !important;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
}

.diff-dialog.el-dialog {
  position: fixed !important;
  top: calc(var(--cs-titlebar-height, 32px) + 10px) !important;
  left: 5% !important;
  width: 90% !important;
  // height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  max-height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  margin: 0 !important;
  z-index: 2001 !important;
  display: flex;
  flex-direction: column;

  .el-dialog__header {
    flex-shrink: 0;
    height: auto;
  }

  .el-dialog__body {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;

    .approval-content {
      flex: 1;
      display: flex;
      flex-direction: column;
      overflow: hidden;
      min-height: 0;
      max-height: 100%;

      .details-box {
        // flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        min-height: 0;
        max-height: calc(100% - 60px);

        .diff-view {
          flex: 1;
          min-height: 0;
          max-height: 100%;
          overflow-y: auto;
        }
      }
    }
  }

  .el-dialog__footer {
    flex-shrink: 0;
    height: auto;
  }
}
</style>
