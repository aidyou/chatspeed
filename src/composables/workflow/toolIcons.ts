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

  if (normalized === 'create_file') {
    return 'write_file'
  }

  if (normalized.startsWith('todo')) {
    return 'todo'
  }

  if (normalized.startsWith('task') || normalized.startsWith('sub_agent')) {
    return 'task'
  }

  return fallback
}
