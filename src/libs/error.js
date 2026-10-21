import { normalizeErrorReason } from '../constants/errorReasons.js'

// The renderer supplies a locale-aware provider; this module has no UI/runtime imports.
let fallbackProvider = () => ''
export function setUserErrorFallback(provider) {
  fallbackProvider = typeof provider === 'function' ? provider : () => ''
}

function get(value, key) {
  try {
    return value !== null && typeof value === 'object' ? value[key] : undefined
  } catch {
    return undefined
  }
}

function record(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function text(value) {
  return typeof value === 'string' && value.trim() !== '' ? value : undefined
}

function status(value) {
  return Number.isInteger(value) && value >= 100 && value <= 599 ? value : undefined
}

// Scan only named credential containers, including JSON embedded in quoted JSON.
// A fixed input/depth budget fails closed rather than exposing a partial container.
function redactContainers(value) {
  const marker = /(?:\\*"|')?\b(?:headers|env|environment)(?:\\*"|')?\s*[:=]\s*[[{]/gi
  let output = ''
  let cursor = 0
  for (let match; (match = marker.exec(value));) {
    const start = marker.lastIndex - 1
    const escapedQuotes = (match[0].match(/^(\\*)"/)?.[1] || '').length
    const stack = []
    let quoted = false
    let quote = ''
    let end = start
    let complete = false
    for (; end < value.length && end - start < 131072; end++) {
      const char = value[end]
      if (char === '"' || char === "'") {
        let slashes = 0
        for (let i = end - 1; i >= start && value[i] === '\\'; i--) slashes++
        if (slashes % (2 * (escapedQuotes + 1)) === escapedQuotes) {
          if (!quoted) { quoted = true; quote = char }
          else if (quote === char) quoted = false
        }
      }
      if (quoted) continue
      if (char === '{' || char === '[') {
        stack.push(char)
        if (stack.length > 256) break
      } else if (char === '}' || char === ']') {
        if (stack.pop() !== (char === '}' ? '{' : '[')) break
        if (stack.length === 0) { end++; complete = true; break }
      }
    }
    output += value.slice(cursor, start) + '[REDACTED]'
    // Unterminated or over-budget containers may still hold secrets.
    if (!complete) return output
    cursor = end
    marker.lastIndex = end
  }
  return output + value.slice(cursor)
}

function redact(value) {
  return redactContainers(value)
    .replace(/(\b(?:authorization|proxy-authorization|cookie|set-cookie)["']?\s*[:=]\s*)[^\r\n]+/gi, '$1[REDACTED]')
    .replace(/(\\"[\w.-]*(?:api[_-]?key|token|secret|password|credential)[\w.-]*\\"\s*:\s*\\")(?:\\.|[^"\\])*?\\"/gi, '$1[REDACTED]\\"')
    .replace(/\bBearer\s+[^\s"'<>;,}]+/gi, 'Bearer [REDACTED]')
    .replace(/(\b[a-z][a-z\d+.-]*:\/\/)[^\s/@]+@/gi, '$1[REDACTED]@')
    .replace(/([?&](?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|secret|password|signature|credential|key)=)[^&#\s"'<>]*/gi, '$1[REDACTED]')
    .replace(/(["']?(?:[\w.-]*(?:api[_-]?key|token|secret|password|credential)[\w.-]*|x-[\w-]+)["']?\s*[:=]\s*)(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;}\r\n]+)/gi, '$1[REDACTED]')
    .replace(/(\b[A-Z][A-Z\d_]*\s*=\s*)(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;}\r\n]+)/g, '$1[REDACTED]')
}

function display(value, fallback) {
  const candidate = text(value)
  if (!candidate || /<\s*(?:!doctype\s+html|html|head|body|script|title|h[1-6]|div|p|pre|span|table)\b/i.test(candidate)
    || /^(?:\[object Object\]|Error|TypeError|RangeError|SyntaxError|ReferenceError|URIError|EvalError|AggregateError)$/.test(candidate.trim())) {
    return fallback
  }
  const points = Array.from(redact(candidate))
  return points.length > 4096 ? `${points.slice(0, 4096).join('')}…` : points.join('')
}

function unwrap(value) {
  if (typeof value !== 'string' || !value.trimStart().startsWith('{')) return value
  try {
    const parsed = JSON.parse(value)
    // Unknown JSON is not a license to search arbitrary payloads for messages.
    if (record(parsed) && ['message', 'module', 'details', 'code', 'presentation', 'status', 'error', 'errors']
      .some(key => get(parsed, key) !== undefined)) return parsed
    return undefined
  } catch {
    return value
  }
}

/**
 * Normalize only documented envelope paths, never arbitrary causes/payloads.
 * Metadata is read from structured fields only, never inferred from prose.
 * @param {unknown} error
 * @param {string} [fallback] Localized generic error, supplied by the renderer.
 * @returns {{message: string, reason: string, code?: string, upstream_status?: number, module?: string, kind?: string}}
 */
export function normalizeUserError(error, fallback) {
  const safeFallback = display(typeof fallback === 'string' ? fallback : fallbackProvider(), '')
  const root = unwrap(error)
  const details = get(root, 'details')
  const innerDetails = get(details, 'details')
  const message = unwrap(get(root, 'message'))
  const detailMessage = unwrap(get(details, 'message'))
  const innerMessage = unwrap(get(message, 'message'))
  const httpError = unwrap(get(root, 'error'))
  const protocolErrors = get(root, 'errors')
  // AiError uses content="message"; its struct variants carry a details string.
  const aiKinds = ['apiRequestFailed', 'rawApiRequestFailed', 'responseParseFailed',
    'streamProcessingFailed', 'toolCallSerializationFailed']
  const aiMessage = get(root, 'module') === 'Ai' && aiKinds.includes(get(details, 'kind'))
    ? detailMessage : undefined
  const aiDetails = unwrap(get(aiMessage, 'details'))
  const aiError = get(aiDetails, 'error')
  const aiErrors = get(aiDetails, 'errors')
  const nodes = [root, httpError, protocolErrors, details, innerDetails, message, detailMessage,
    innerMessage, aiDetails, aiError, aiErrors].filter(record)
  const result = { message: safeFallback, reason: 'unknown' }
  for (const key of ['code', 'module', 'kind']) {
    const value = nodes.map(node => text(get(node, key))).find(Boolean)
    if (value) result[key] = value
  }
  if (!result.kind && text(get(root, 'module')) && typeof details === 'string') {
    result.kind = get(root, 'module')
  }
  const legacyStatus = nodes.map(node => status(get(node, 'upstream_status')) ?? status(get(node, 'status')))
    .find(value => value !== undefined) ?? status(get(aiMessage, 'status_code'))
  if (legacyStatus !== undefined) result.upstream_status = legacyStatus
  result.reason = normalizeErrorReason(nodes.map(node => get(node, 'reason')).find(value => value !== undefined))

  for (const node of nodes) {
    const presentation = get(node, 'presentation')
    if (get(presentation, 'schema_version') !== 1 || !text(get(presentation, 'message'))
      || (get(presentation, 'reason') !== undefined && typeof get(presentation, 'reason') !== 'string')
      || (get(presentation, 'upstream_status') !== undefined && status(get(presentation, 'upstream_status')) === undefined)) continue
    result.message = display(get(presentation, 'message'), safeFallback)
    result.reason = normalizeErrorReason(get(presentation, 'reason'))
    if (status(get(presentation, 'upstream_status')) !== undefined) {
      result.upstream_status = get(presentation, 'upstream_status')
    }
    return result
  }

  const candidates = [get(httpError, 'message'), httpError, get(protocolErrors, 'message'),
    get(aiError, 'message'), aiError, get(aiErrors, 'message'), get(aiDetails, 'message'), aiDetails,
    get(innerDetails, 'message'), innerDetails, get(detailMessage, 'message'), detailMessage, details,
    get(innerMessage, 'message'), innerMessage, message, typeof root === 'string' ? root : undefined]
  result.message = display(candidates.find(value => text(value)), safeFallback)
  return result
}

export function formatUserError(error, fallback) {
  return normalizeUserError(error, fallback).message
}
