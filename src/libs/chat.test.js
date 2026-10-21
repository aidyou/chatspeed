import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import test from 'node:test'

import { formatUserError } from './error.js'

// chat.js imports Vue, Pinia, i18n and Tauri at module scope. Extract only the
// shared stream handler so the error display contract can run in Node.
const source = readFileSync(new URL('./chat.js', import.meta.url), 'utf8')
const start = source.indexOf('export const handleChatMessage =')
assert.notEqual(start, -1, 'missing handleChatMessage')
const end = source.indexOf('\n}\n', start)
assert.notEqual(end, -1, 'unterminated handleChatMessage')
const declaration = source.slice(start, end + 2).replace('export const ', 'const ')

const fallback = 'Something went wrong. Please try again.'
const i18n = {
  global: {
    t: key => (key === 'common.errorFallback' ? fallback : key)
  }
}

const loadHandler = formatter => {
  const calls = []
  const handler = runInNewContext(`${declaration}\nhandleChatMessage`, {
    console,
    i18n,
    formatUserError: (...args) => {
      calls.push(args)
      return formatter(...args)
    }
  })
  return { handler, calls }
}

const createRefs = () => ({
  chatErrorMessage: { value: '' },
  isChatting: { value: true }
})

test('typed metadata root is displayed before the legacy chunk', () => {
  const { handler, calls } = loadHandler(formatUserError)
  const payload = {
    type: 'error',
    chunk: JSON.stringify({
      status: 401,
      message: '[Ai:Upstream] raw legacy cause CANARY'
    }),
    metadata: {
      model: 'model-x',
      user_error: {
        schema_version: 1,
        message: 'typed provider copy',
        reason: 'auth',
        upstream_status: 401
      }
    }
  }
  const before = JSON.stringify(payload)
  const refs = createRefs()
  let completions = 0

  const done = handler(payload, { value: {} }, refs, () => {
    completions += 1
  })

  assert.equal(done, true)
  assert.equal(refs.chatErrorMessage.value, 'typed provider copy')
  assert.equal(refs.isChatting.value, false)
  assert.equal(completions, 1, 'terminal error must invoke the completion callback exactly once')
  assert.equal(calls.length, 1, 'the display copy must be formatted exactly once')
  assert.equal(
    JSON.stringify(calls[0][0]),
    JSON.stringify({ presentation: payload.metadata.user_error, message: payload.chunk })
  )
  assert.equal(JSON.stringify(payload), before, 'stream payload must remain unchanged')
})

test('object chunks never render object stringification', () => {
  const { handler } = loadHandler(formatUserError)
  const payload = {
    type: 'error',
    chunk: { message: 'safe object message', code: 'busy' },
    metadata: {}
  }
  const before = JSON.stringify(payload)
  const refs = createRefs()
  let completions = 0

  handler(payload, { value: {} }, refs, () => {
    completions += 1
  })

  assert.equal(refs.chatErrorMessage.value, 'safe object message')
  assert.doesNotMatch(refs.chatErrorMessage.value, /\[object Object\]/)
  assert.equal(completions, 1)
  assert.equal(JSON.stringify(payload), before)
})

test('legacy plain chunk keeps its display copy when metadata is absent', () => {
  const { handler } = loadHandler(formatUserError)
  const refs = createRefs()

  handler(
    { type: 'error', chunk: 'Critical Error: legacy raw cause' },
    { value: {} },
    refs
  )

  assert.equal(refs.chatErrorMessage.value, 'Critical Error: legacy raw cause')
  assert.equal(refs.isChatting.value, false)
})