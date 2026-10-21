import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'

import { resolveWorkflowUserError } from './messageProjectionRules.js'

// The composable cannot be imported directly in Node because it depends on Vue,
// Pinia and Tauri aliases. Extract only the pure adapters so their behavior is still
// covered against real snapshot payload shapes.
const source = readFileSync(new URL('./useSubAgentSummaries.ts', import.meta.url), 'utf8')

const extractConst = name => {
  const startMarker = `const ${name} =`
  const start = source.indexOf(startMarker)
  assert.notEqual(start, -1, `missing ${name}`)
  const end = source.indexOf('\n}\n', start)
  assert.notEqual(end, -1, `unterminated ${name}`)
  return source.slice(start, end + 2)
}

const resolveSubAgentSnapshotUserError = runInNewContext(
  `${extractConst('resolveSubAgentSnapshotUserError')}\nresolveSubAgentSnapshotUserError`,
  { resolveWorkflowUserError }
)

const resolveSubAgentSummaryDisplay = runInNewContext(
  `${extractConst('resolveSnapshotUserErrorValue')}\n${extractConst(
    'resolveSubAgentSummaryDisplay'
  )}\nresolveSubAgentSummaryDisplay`,
  { resolveWorkflowUserError }
)

const typedUserError = {
  schema_version: 1,
  message: 'provider overloaded',
  reason: 'unknown',
  upstream_status: 503
}

test('legacy snapshot without typed terminal metadata has no user error', () => {
  assert.equal(
    resolveSubAgentSnapshotUserError({
      role: 'assistant',
      message: 'Critical Error: raw provider failure',
      metadata: {}
    }),
    null
  )
})

test('legacy terminal text is not classified as a typed user error', () => {
  assert.equal(
    resolveSubAgentSnapshotUserError({
      role: 'assistant',
      message: 'Critical Error: overloaded, reason: unknown',
      metadata: { observation_type: 'terminal_error' }
    }),
    null
  )
})

test('typed terminal metadata projects the safe root user error', () => {
  const latest = {
    role: 'assistant',
    message: 'Critical Error: raw provider failure CANARY',
    metadata: {
      observation_type: 'terminal_error',
      user_error: typedUserError
    }
  }

  const projection = resolveSubAgentSnapshotUserError(latest)

  assert.equal(projection.message, 'provider overloaded')
  assert.equal(projection.display, 'provider overloaded')
  assert.equal(projection.reason, 'unknown')
  assert.equal(projection.upstream_status, 503)
  assert.deepEqual(latest.metadata.user_error, typedUserError)
  assert.equal(latest.message, 'Critical Error: raw provider failure CANARY')
})

test('non-terminal observations never project a user error from metadata', () => {
  assert.equal(
    resolveSubAgentSnapshotUserError({
      role: 'tool',
      message: 'overloaded',
      metadata: { observation_type: 'tool_result', user_error: typedUserError }
    }),
    null
  )
})

test('safe projection supersedes legacy summary overrides from live metadata', () => {
  const last = {
    role: 'assistant',
    message: 'Critical Error: raw provider failure CANARY',
    metadata: { user_error: typedUserError }
  }
  const progress = { subAgentId: 'child', summary: 'Critical Error: snapshot CANARY' }

  const display = resolveSubAgentSummaryDisplay(
    progress,
    last,
    'Critical Error: legacy summary CANARY'
  )

  assert.equal(display, 'provider overloaded')
  assert.doesNotMatch(display, /\[object Object\]|CANARY/)
})

test('snapshot-normalized userError supersedes progress summary text', () => {
  const progress = {
    subAgentId: 'child',
    summary: '[object Object]',
    userError: { display: 'safe snapshot copy', message: 'safe snapshot copy', reason: 'unknown' }
  }

  const display = resolveSubAgentSummaryDisplay(
    progress,
    { role: 'assistant', message: 'raw snapshot failure' },
    '[object Object]'
  )

  assert.equal(display, 'safe snapshot copy')
  assert.doesNotMatch(display, /\[object Object\]/)
})

test('live progress result error metadata resolves through the central path', () => {
  const progress = {
    subAgentId: 'child',
    summary: 'raw progress summary',
    result: { error_metadata: { user_error: typedUserError } }
  }

  assert.equal(
    resolveSubAgentSummaryDisplay(
      progress,
      { role: 'assistant', message: 'raw live failure' },
      'raw progress summary'
    ),
    'provider overloaded'
  )
})

test('typed snapshot presentation object is projected without object stringification', () => {
  const progress = { userError: typedUserError }

  const display = resolveSubAgentSummaryDisplay(
    progress,
    { role: 'assistant', message: 'raw live failure' },
    'raw legacy summary'
  )

  assert.equal(display, 'provider overloaded')
  assert.doesNotMatch(display, /\[object Object\]/)
})

test('normalized snapshot errors are sanitized again at the summary boundary', () => {
  const value = resolveSubAgentSummaryDisplay({
    userError: { display: 'provider token=CANARY', reason: 'unknown' }
  }, null, 'legacy')
  assert.match(value, /^provider/)
  assert.doesNotMatch(value, /CANARY/)
  assert.equal(resolveSubAgentSummaryDisplay({ userError: 'not a typed projection' }, null, 'legacy'), 'legacy')
})

test('legacy summary survives when no safe projection exists', () => {
  assert.equal(
    resolveSubAgentSummaryDisplay(
      { summary: 'Running' },
      { role: 'assistant', message: 'Working' },
      'Working'
    ),
    'Working'
  )
})

test('summary assembly applies the safe projection after legacy overrides', () => {
  const progressSummary = source.indexOf('summary = truncateSummary(progress.summary) || summary')
  const safeProjection = source.indexOf(
    'summary = truncateSummary(resolveSubAgentSummaryDisplay(progress, last, summary))'
  )

  assert.ok(progressSummary >= 0, 'progress summary override should remain on the legacy path')
  assert.ok(
    safeProjection > progressSummary,
    'safe projection must run after legacy summary assembly'
  )
})