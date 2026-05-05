import i18n from '@/i18n'

const normalizeSlash = (value: string) => value.replace(/\\/g, '/')

const stripTrailingSlash = (value: string) => value.replace(/\/+$/, '')

const getPrimaryRoot = (roots?: string[]) => {
  const root = (roots || []).find(candidate => typeof candidate === 'string' && candidate.trim())
  return root ? stripTrailingSlash(normalizeSlash(root.trim())) : ''
}

export const formatDisplayPath = (path: string, roots?: string[]) => {
  if (!path || typeof path !== 'string') return path
  const normalized = normalizeSlash(path.trim())
  const primaryRoot = getPrimaryRoot(roots)

  if (primaryRoot) {
    if (normalized === primaryRoot) return '.'
    if (normalized.startsWith(`${primaryRoot}/`)) {
      return normalized.slice(primaryRoot.length + 1) || '.'
    }
    return normalized
  }

  if (!normalized.startsWith('/')) return normalized
  return normalized
}

export const normalizeToolDisplayText = (text: string, roots?: string[]) => {
  if (!text || typeof text !== 'string') return text
  const pathPattern = /(^|[\s("'`])((?:[A-Za-z]:)?\/[^\s"'`<>),:;]+)/g

  return text
    .replace(pathPattern, (_, prefix, match) => `${prefix}${formatDisplayPath(match, roots)}`)
    .replace(/\bfrom L(\d+)\b/g, 'L$1')
}

export const normalizeShellCommandForDisplay = (command: string, roots?: string[]) => {
  if (!command || typeof command !== 'string') return command
  return normalizeToolDisplayText(command, roots)
}

const EDIT_RESULT_TOOLS = new Set(['edit_file', 'write_file'])

type ToolStatusSummaryState = 'pending' | 'running' | 'rejected' | 'success' | 'failed'

const workflowText = (key: string, fallback: string) => {
  const translated = i18n.global.t(key)
  return typeof translated === 'string' && translated !== key ? translated : fallback
}

export const isEditResultTool = (toolName?: string) =>
  typeof toolName === 'string' && EDIT_RESULT_TOOLS.has(toolName)

export const getToolStatusSummary = (
  toolName: string | undefined,
  state: ToolStatusSummaryState | undefined,
  fallback = ''
) => {
  if (!isEditResultTool(toolName) || !state) return fallback

  switch (state) {
    case 'pending':
      return workflowText('workflow.awaitingApproval', 'Awaiting approval')
    case 'running':
      return workflowText('workflow.executing', 'Executing...')
    case 'rejected':
      return workflowText('workflow.rejected', 'Rejected')
    case 'success':
      return toolName === 'edit_file'
        ? workflowText('workflow.edited', 'Edited')
        : workflowText('workflow.written', 'Written')
    case 'failed':
    default:
      return fallback
  }
}
