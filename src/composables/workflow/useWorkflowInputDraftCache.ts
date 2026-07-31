import { csGetStorage, csRemoveStorage, csSetStorage } from '@/libs/util'

const DRAFT_KEY_PREFIX = 'workflow-input-draft:'
const MAX_DATA_URL_LENGTH = 512 * 1024
const MAX_SERIALIZED_DRAFT_LENGTH = 2 * 1024 * 1024

export function workflowInputDraftKey(sessionId) {
  return `${DRAFT_KEY_PREFIX}${sessionId}`
}

export function normalizeWorkflowInputDraft(sessionId, draft = {}) {
  const inputMessage = typeof draft.inputMessage === 'string' ? draft.inputMessage : ''
  const attachments = Array.isArray(draft.attachments)
    ? draft.attachments.map(normalizeDraftAttachment).filter(Boolean)
    : []

  return fitDraftToBudget({
    version: 1,
    sessionId,
    updatedAt: Date.now(),
    inputMessage,
    attachments
  })
}

export function loadWorkflowInputDraft(sessionId) {
  if (!sessionId) return null
  const draft = csGetStorage(workflowInputDraftKey(sessionId), null)
  if (!draft || typeof draft !== 'object') return null
  if (draft.sessionId && draft.sessionId !== sessionId) return null

  return {
    version: 1,
    sessionId,
    updatedAt: Number(draft.updatedAt) || 0,
    inputMessage: typeof draft.inputMessage === 'string' ? draft.inputMessage : '',
    attachments: Array.isArray(draft.attachments)
      ? draft.attachments.map(normalizeRestoredAttachment).filter(Boolean)
      : []
  }
}

export function saveWorkflowInputDraft(sessionId, draft = {}) {
  if (!sessionId) return false
  const normalized = normalizeWorkflowInputDraft(sessionId, draft)

  if (!normalized.inputMessage.trim() && normalized.attachments.length === 0) {
    removeWorkflowInputDraft(sessionId)
    return true
  }

  try {
    csSetStorage(workflowInputDraftKey(sessionId), normalized)
    return true
  } catch (error) {
    const textOnlyDraft = fitDraftToBudget({ ...normalized, attachments: [] })
    try {
      if (textOnlyDraft.inputMessage.trim()) {
        csSetStorage(workflowInputDraftKey(sessionId), textOnlyDraft)
        return true
      }
      removeWorkflowInputDraft(sessionId)
      return true
    } catch (textOnlyError) {
      console.warn('[Workflow] Failed to save input draft:', textOnlyError || error)
      return false
    }
  }
}

export function removeWorkflowInputDraft(sessionId) {
  if (!sessionId) return
  try {
    csRemoveStorage(workflowInputDraftKey(sessionId))
  } catch (error) {
    console.warn('[Workflow] Failed to remove input draft:', error)
  }
}

function normalizeDraftAttachment(attachment) {
  if (!attachment || typeof attachment !== 'object') return null
  const url = typeof attachment.url === 'string' ? attachment.url : ''
  const sourceUrl = typeof attachment.sourceUrl === 'string' ? attachment.sourceUrl : ''
  const path = typeof attachment.path === 'string' ? attachment.path : ''
  const persistentUrl = pickPersistableUrl(url) || pickPersistableUrl(sourceUrl)
  const isDataUrl = (url || sourceUrl).startsWith('data:')
  const base = {
    id: String(attachment.id || `draft_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`),
    type: attachment.type || 'image',
    name: String(attachment.name || 'image'),
    size: Number(attachment.size) || 0,
    path,
    sourceKind: path ? 'path' : isDataUrl ? 'dataUrl' : 'url'
  }

  if (persistentUrl) {
    return {
      ...base,
      url: persistentUrl,
      sourceUrl: pickPersistableUrl(sourceUrl) || persistentUrl,
      uploading: false
    }
  }

  if (path) {
    return {
      ...base,
      url: '',
      sourceUrl: '',
      uploading: false
    }
  }

  return {
    ...base,
    url: '',
    sourceUrl: '',
    uploading: false,
    unrestorable: true
  }
}

function normalizeRestoredAttachment(attachment) {
  if (!attachment || attachment.unrestorable) return null
  if (!attachment.url && !attachment.sourceUrl && !attachment.path) return null
  return {
    id: String(attachment.id || `draft_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`),
    type: attachment.type || 'image',
    name: String(attachment.name || 'image'),
    size: Number(attachment.size) || 0,
    url: typeof attachment.url === 'string' ? attachment.url : '',
    sourceUrl: typeof attachment.sourceUrl === 'string' ? attachment.sourceUrl : '',
    path: typeof attachment.path === 'string' ? attachment.path : '',
    uploading: false,
    sourceKind: attachment.sourceKind || (attachment.path ? 'path' : 'url')
  }
}

function fitDraftToBudget(draft) {
  if (serializedLength(draft) <= MAX_SERIALIZED_DRAFT_LENGTH) return draft

  const fittedDraft = { ...draft, attachments: [] }
  for (const attachment of draft.attachments) {
    if (attachment.unrestorable) continue
    const candidate = {
      ...fittedDraft,
      attachments: [...fittedDraft.attachments, attachment]
    }
    if (serializedLength(candidate) <= MAX_SERIALIZED_DRAFT_LENGTH) {
      fittedDraft.attachments = candidate.attachments
    }
  }

  if (serializedLength(fittedDraft) <= MAX_SERIALIZED_DRAFT_LENGTH) return fittedDraft
  return {
    ...fittedDraft,
    inputMessage: fitStringToSerializedBudget(fittedDraft, 'inputMessage')
  }
}

function fitStringToSerializedBudget(draft, fieldName) {
  const value = String(draft[fieldName] || '')
  if (!value) return ''

  let low = 0
  let high = value.length
  let best = ''
  while (low <= high) {
    const mid = Math.floor((low + high) / 2)
    const candidate = value.slice(0, mid)
    if (serializedLength({ ...draft, [fieldName]: candidate }) <= MAX_SERIALIZED_DRAFT_LENGTH) {
      best = candidate
      low = mid + 1
    } else {
      high = mid - 1
    }
  }
  return best
}

function serializedLength(value) {
  return JSON.stringify(value).length
}

function pickPersistableUrl(value) {
  if (typeof value !== 'string' || !value) return ''
  if (value.startsWith('data:') && value.length > MAX_DATA_URL_LENGTH) return ''
  return value
}
