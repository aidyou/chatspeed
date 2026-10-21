import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import { resolveToolResultState, resolveToolResultDisplay } from './toolResultState.js'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')

test('only typed execution results determine success and failure', () => {
  for (const result of ['', 0, false, 'Error: normal output', '失败', { error: 'ordinary payload' }]) {
    assert.equal(resolveToolResultState({ result, is_error: false }), 'success')
    assert.equal(resolveToolResultState({ result, is_error: true }), 'error')
    assert.equal(resolveToolResultState({ result }), 'unknown')
  }
  for (const result of ['Error: request failed', '{"error":"denied"}', 'completed successfully']) {
    assert.equal(resolveToolResultState({ result }), 'unknown')
  }
  assert.equal(resolveToolResultState({ result: 'ok', is_error: 'false' }), 'unknown')
  assert.equal(resolveToolResultState({ result: 'ok', status: 'failed' }), 'error')
  assert.equal(resolveToolResultState({ result: 'Error: example', status: 'completed' }), 'success')
  assert.equal(resolveToolResultState({ result: 'ok', status: 'failure with extra words' }), 'unknown')
})

test('pending detection depends on result presence, not result truthiness or wording', () => {
  for (const value of [null, undefined, {}, { result: null }, { result: undefined }]) {
    assert.equal(resolveToolResultState(value), 'calling')
  }
  for (const result of ['', 0, false]) {
    assert.equal(resolveToolResultState({ result }), 'unknown')
  }
})

test('failed tools display the safe sidecar without mutating the persisted result', () => {
  const call = {
    result: 'Tool execution failed: legacy CANARY',
    is_error: true,
    user_error: { schema_version: 1, message: 'overloaded: https://example.test/api?token=SECRET\nretry' }
  }
  const before = JSON.stringify(call)
  assert.equal(resolveToolResultDisplay(call), 'overloaded: https://example.test/api?token=[REDACTED]\nretry')
  assert.equal(JSON.stringify(call), before)
  assert.equal(resolveToolResultDisplay({ ...call, is_error: false }), call.result)
  assert.equal(resolveToolResultDisplay({ result: 'Error: old history' }), 'Error: old history')
  assert.equal(resolveToolResultState({ result: 'ordinary output', user_error: call.user_error }), 'unknown')
})

test('all three ordinary-chat renderers use the same structured state adapter', async () => {
  for (const path of ['./chat.js', '../components/chat/ToolCall.vue', '../components/chat/Chatting.vue']) {
    const source = await read(path)
    assert.match(source, /resolveToolResultState\(call\)/)
    assert.match(source, /resolveToolResultDisplay\(call\)/)
    assert.doesNotMatch(source, /startsWith\(['"]Error: /)
  }
})

test('streaming tool renderer retains the i18n binding used by its template builder', async () => {
  const source = await read('../components/chat/Chatting.vue')
  assert.match(source, /import i18n from ['"]@\/i18n\/index\.js['"]/)
  const helper = source.match(/const createSingleToolCallHtml = call => \{[\s\S]*?\n\}/)?.[0]
  assert.ok(helper)
  const render = runInNewContext(`${helper}; createSingleToolCallHtml`, {
    resolveToolResultState,
    resolveToolResultDisplay,
    toolName: name => name,
    htmlspecialchars: value => value,
    i18n: { global: { t: key => key } }
  })
  assert.match(render({ function: { name: 'tool', arguments: '{}' }, result: 'ok', is_error: false }), /tool-name success/)
})
