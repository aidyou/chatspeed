import i18n from '@/i18n'

const PROJECT_MARKERS = [
  'app',
  'frontend',
  'src-tauri',
  'internal',
  'cmd',
  'pkg',
  'docs',
  'scripts',
  'work',
  'src',
  'test',
  'tests'
]

const normalizeSlash = (value: string) => value.replace(/\\/g, '/')

const stripTrailingSlash = (value: string) => value.replace(/\/+$/, '')

const normalizeRootCandidates = (roots?: string[]) =>
  (roots || [])
    .filter(root => typeof root === 'string' && root.trim())
    .map(root => stripTrailingSlash(normalizeSlash(root.trim())))
    .sort((a, b) => b.length - a.length)

export const formatDisplayPath = (path: string, roots?: string[]) => {
  if (!path || typeof path !== 'string') return path
  const normalized = normalizeSlash(path.trim())
  const rootCandidates = normalizeRootCandidates(roots)

  for (const root of rootCandidates) {
    if (normalized === root) return '.'
    if (normalized.startsWith(`${root}/`)) {
      return normalized.slice(root.length + 1) || '.'
    }
  }

  if (!normalized.startsWith('/')) return normalized

  for (const marker of PROJECT_MARKERS) {
    const token = `/${marker}/`
    const index = normalized.indexOf(token)
    if (index >= 0) {
      return normalized.slice(index + 1)
    }
  }

  const parts = normalized.split('/').filter(Boolean)
  return parts.slice(-3).join('/') || normalized
}

export const normalizeToolDisplayText = (text: string, roots?: string[]) => {
  if (!text || typeof text !== 'string') return text
  const pathPattern = /(?:[A-Za-z]:)?\/[^\s"'`<>),:;]+/g

  return text
    .replace(pathPattern, match => formatDisplayPath(match, roots))
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
      return workflowText('workflow.awaiting_approval', 'Awaiting approval')
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
