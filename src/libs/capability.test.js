import assert from 'node:assert/strict'
import test from 'node:test'

import {
  defaultTargetSelection,
  mcpDisplayState,
  mcpViewIndex,
  mutationOutcomes,
  newIdempotencyKey,
  parseCapabilityError,
  verdictAllowsInstall
} from './capability.js'

test('a capability error envelope keeps its machine code', () => {
  const blocked = parseCapabilityError(
    JSON.stringify({ code: 'check_blocked', message: 'the skill was refused' })
  )
  assert.equal(blocked.code, 'check_blocked')
  assert.equal(blocked.message, 'the skill was refused')

  const conflict = parseCapabilityError({
    code: 'idempotency_key_conflict',
    message: 'conflict'
  })
  assert.equal(conflict.code, 'idempotency_key_conflict')
})

test('an unrecognized failure is still an error with an unknown code', () => {
  const plain = parseCapabilityError('backend exploded')
  assert.equal(plain.code, 'unknown')
  assert.match(plain.message, /backend exploded/)

  const notEnvelope = parseCapabilityError(JSON.stringify({ module: 'x' }))
  assert.equal(notEnvelope.code, 'unknown')
})

test('only an explicit pass verdict authorizes an install', () => {
  assert.equal(verdictAllowsInstall({ verdict: 'pass' }), true)
  // blocked and inconclusive both refuse, and no report never authorizes.
  assert.equal(verdictAllowsInstall({ verdict: 'blocked' }), false)
  assert.equal(verdictAllowsInstall({ verdict: 'inconclusive' }), false)
  assert.equal(verdictAllowsInstall(null), false)
  assert.equal(verdictAllowsInstall(undefined), false)
  assert.equal(verdictAllowsInstall({}), false)
})

test('the default selection is the default target only, never an external tool', () => {
  const targets = [
    { id: 'chatspeed', supported: true, default_selected: true },
    { id: 'agents', supported: true, default_selected: false },
    { id: 'claude-code', supported: true, default_selected: false },
    { id: 'codex', supported: false, default_selected: false }
  ]
  assert.deepEqual(defaultTargetSelection(targets), ['chatspeed'])

  // A registry that marks nothing as default selects nothing, rather than
  // falling back to everything.
  assert.deepEqual(
    defaultTargetSelection([{ id: 'agents', supported: true, default_selected: false }]),
    []
  )
  // An unsupported default can never be selected.
  assert.deepEqual(
    defaultTargetSelection([{ id: 'chatspeed', supported: false, default_selected: true }]),
    []
  )
  assert.deepEqual(defaultTargetSelection(undefined), [])
})

test('mutation outcomes are read from both recorded shapes', () => {
  const install = {
    result: {
      stage: 'applied',
      install: { outcomes: [{ target_id: 'chatspeed', status: 'installed' }] }
    }
  }
  assert.deepEqual(mutationOutcomes(install), [{ target_id: 'chatspeed', status: 'installed' }])

  const uninstall = {
    result: { outcomes: [{ target_id: 'chatspeed', status: 'refused' }] }
  }
  assert.deepEqual(mutationOutcomes(uninstall), [{ target_id: 'chatspeed', status: 'refused' }])

  // A replayed or failed operation may carry no projection at all.
  assert.deepEqual(mutationOutcomes(null), [])
  assert.deepEqual(mutationOutcomes({ result: null }), [])
  assert.deepEqual(mutationOutcomes({ result: { install: {} } }), [])
})

test('a generated idempotency key is unique per call and carries its prefix', () => {
  const first = newIdempotencyKey('skill-install', () => 1700000000000)
  const second = newIdempotencyKey('skill-install', () => 1700000000000)
  assert.match(first, /^skill-install-/)

  // A retry must be able to reuse a key, so the caller can pass a fixed one;
  // two fresh keys must never collide.
  assert.notEqual(first, second)
  assert.equal(newIdempotencyKey('x', () => 1700000000000).split('-')[0], 'x')
})

test('the MCP projection is indexed by id so a rename cannot mis-merge', () => {
  const index = mcpViewIndex([
    { id: 1, name: 'weather', desired: { enabled: true }, runtime: { observed: true, state: 'running' } },
    { id: 2, name: 'notes', desired: { enabled: false }, runtime: { observed: false, state: 'unknown' } },
    { name: 'malformed' }
  ])
  assert.deepEqual(Object.keys(index), ['1', '2'])
  assert.equal(index[2].runtime.observed, false)
})

/// A row must distinguish "the runtime said stopped" from "nobody looked", and
/// must never turn a missing projection into an invented state (INV-7).
test('an unobserved runtime is reported as unobserved, never as stopped', () => {
  const answered = mcpDisplayState({
    id: 1,
    runtime: { observed: true, state: 'stopped' },
    tools: { freshness: 'fresh', count: 4 },
    drift: null
  })
  assert.equal(answered.state, 'stopped')
  assert.equal(answered.toolsFreshness, 'fresh')
  assert.equal(answered.toolCount, 4)

  const unobserved = mcpDisplayState({
    id: 2,
    runtime: { observed: false, state: 'unknown' },
    tools: { freshness: 'unavailable' }
  })
  assert.equal(unobserved.state, 'unobserved')
  assert.equal(unobserved.observed, false)
  assert.equal(unobserved.toolCount, null)

  const missing = mcpDisplayState(undefined)
  assert.equal(missing.state, 'unknown')
  assert.equal(missing.drift, null)
})

/// The snake_case wire is converted once, at this boundary.
test('the observation time is converted from the canonical wire name', () => {
  const facts = mcpDisplayState({
    id: 1,
    runtime: { observed: true, state: 'running', observed_at_ms: 1700000000000 }
  })
  assert.equal(facts.observedAtMs, 1700000000000)
})
