import { getWorkflowToolFamily } from './toolClassification'

const DIRECT_ICON_NAMES = new Set([
  'list_dir',
  'edit_file',
  'read_file',
  'write_file',
  'submit_plan',
  'ask_user',
  'glob',
  'grep',
  'bash',
  'web_fetch',
  'web_search',
  'skill'
])

export function resolveWorkflowToolIcon(toolName: string, fallback = 'tool'): string {
  const normalized = String(toolName || '')
    .trim()
    .toLowerCase()
  if (!normalized) return fallback

  if (DIRECT_ICON_NAMES.has(normalized)) {
    return normalized
  }

  const family = getWorkflowToolFamily(normalized)
  if (family) return family

  return fallback
}
