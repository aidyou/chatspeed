import i18n from '@/i18n'

const normalizeSlash = (value: string) => value.replace(/\\/g, '/')

const stripTrailingSlash = (value: string) => {
  const stripped = value.replace(/\/+$/, '')
  return stripped || '/'
}

const getMatchingRoot = (path: string, roots?: string[]) => {
  return (roots || [])
    .filter(candidate => typeof candidate === 'string' && candidate.trim())
    .map(candidate => stripTrailingSlash(normalizeSlash(candidate.trim())))
    .filter(root => root === '/' || path === root || path.startsWith(`${root}/`))
    .sort((left, right) => right.length - left.length)[0]
}

export const formatDisplayPath = (path: string, roots?: string[]) => {
  if (!path || typeof path !== 'string') return path
  const normalized = normalizeSlash(path.trim())
  const matchingRoot = getMatchingRoot(normalized, roots)

  if (matchingRoot) {
    if (normalized === matchingRoot) return '.'
    return matchingRoot === '/'
      ? normalized.slice(1) || '.'
      : normalized.slice(matchingRoot.length + 1) || '.'
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

const resolveWorkflowSummaryText = (text: string) => {
  if (!text || typeof text !== 'string') return text
  if (!text.startsWith('workflow.')) return text
  return workflowText(text, text)
}

export const isEditResultTool = (toolName?: string) =>
  typeof toolName === 'string' && EDIT_RESULT_TOOLS.has(toolName)

export const getToolStatusSummary = (
  toolName: string | undefined,
  state: ToolStatusSummaryState | undefined,
  fallback = ''
) => {
  const resolvedFallback = resolveWorkflowSummaryText(fallback)
  if (!state) return resolvedFallback

  switch (state) {
    case 'pending':
      return workflowText('workflow.awaitingApproval', 'Awaiting approval')
    case 'running':
      return workflowText('workflow.executing', 'Executing...')
    case 'rejected':
      return workflowText('workflow.rejected', 'Rejected')
    case 'success':
      return isEditResultTool(toolName)
        ? toolName === 'edit_file'
          ? workflowText('workflow.edited', 'Edited')
          : workflowText('workflow.written', 'Written')
        : resolvedFallback
    case 'failed':
    default:
      return resolvedFallback
  }
}
