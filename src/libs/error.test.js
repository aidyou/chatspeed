import assert from 'node:assert/strict'
import test from 'node:test'
import { readFile } from 'node:fs/promises'
import { runInNewContext } from 'node:vm'
import { formatUserError, normalizeUserError, setUserErrorFallback } from './error.js'
import { USER_ERROR_REASONS } from '../constants/errorReasons.js'

const fallback = '请重试'

test('plain messages retain colons, URLs and multiline causes without prefix guessing', () => {
  const message = 'Workflow: request https://example.com:8443/x failed\ncause: connection refused'
  assert.equal(formatUserError(message, fallback), message)
  assert.equal(formatUserError(new TypeError(message), fallback), message)
  assert.equal(formatUserError('{not json: cause}', fallback), '{not json: cause}')
})

test('only bounded known envelope paths are read, including Sensitive details', () => {
  const samples = [
    [{ module: 'Ai', details: { kind: 'Upstream', message: 'provider declined' } }, 'provider declined'],
    [{ module: 'General', details: 'general failure' }, 'general failure'],
    [{ module: 'Sensitive', details: { kind: 'InvalidRule', details: { message: 'invalid rule' } } }, 'invalid rule'],
    [{ message: { code: 'c', message: 'nested' } }, 'nested'],
    [{ message: { message: { message: 'bounded' } } }, 'bounded'],
    [{ details: { message: { message: 'nested details message' } } }, 'nested details message'],
    [{ status: 503, message: 'unavailable' }, 'unavailable']
  ]
  for (const [value, expected] of samples) {
    assert.equal(formatUserError(value, fallback), expected)
    assert.equal(formatUserError(JSON.stringify(value), fallback), expected)
  }
  assert.equal(formatUserError({ cause: { message: 'hidden' }, payload: { message: 'hidden' } }, fallback), fallback)
  assert.equal(formatUserError('{"arbitrary":{"message":"hidden"}}', fallback), fallback)
  assert.equal(formatUserError({ message: JSON.stringify({ code: 'x', message: 'known json' }) }, fallback), 'known json')
})

test('HTTP shared envelopes and Tauri detail normalize identical presentation data', () => {
  for (const presentation of [undefined,
    { schema_version: 1, message: 'preferred' },
    { schema_version: 1, message: 'preferred', reason: 'session_stopping', upstream_status: 503 },
    { schema_version: 2, message: 'malformed' },
    { schema_version: 1, message: 'malformed', reason: null },
    { schema_version: 1, message: 'malformed', upstream_status: null }]) {
    const detail = { code: 'invalid_state', message: 'legacy', ...(presentation && { presentation }) }
    const expected = normalizeUserError(detail, fallback)
    assert.deepEqual(normalizeUserError({ error: detail }, fallback), expected)
    assert.deepEqual(normalizeUserError(JSON.stringify({ error: detail }), fallback), expected)
    assert.equal(expected.code, 'invalid_state')
    assert.equal(expected.reason, presentation?.reason === 'session_stopping' ? 'session_stopping' : 'unknown')
    assert.equal(expected.upstream_status, presentation?.upstream_status === 503 ? 503 : undefined)
    assert.equal(expected.message, presentation?.schema_version === 1 && presentation.reason !== null
      && presentation.upstream_status !== null ? 'preferred' : 'legacy')
  }
})

test('desktop public rejection JSON retains typed reasons independently of legacy wording', () => {
  for (const [reason, code, message] of [
    ['session_stopping', 'invalid_state', '会话正在停止，请稍后再试'],
    ['session_already_exists', 'conflict', '该会话已存在'],
    ['no_active_session', 'invalid_state', '当前没有活动会话']
  ]) {
    const wire = JSON.stringify({
      error: { code, message: 'technical detail', presentation: { schema_version: 1, message, reason } },
      status: 409
    })
    assert.deepEqual(normalizeUserError(wire, fallback), {
      code, message, reason, upstream_status: 409
    })
  }
  const keywords = 'Session already exists; Session is stopping; No active session'
  for (const presentation of [undefined, { schema_version: 1, message: keywords, reason: 'unknown' }]) {
    const wire = JSON.stringify({
      error: { code: 'invalid_state', message: keywords, ...(presentation && { presentation }) },
      status: 409
    })
    assert.deepEqual(normalizeUserError(wire, fallback), {
      code: 'invalid_state', message: keywords, reason: 'unknown', upstream_status: 409
    })
  }
  const legacy = { error: { code: 'not_found', message: 'workflow is gone' }, status: 404 }
  assert.equal(normalizeUserError(JSON.stringify(legacy), fallback).message, 'workflow is gone')
  assert.equal(normalizeUserError(JSON.stringify(legacy), fallback).reason, 'unknown')
  assert.equal(normalizeUserError(JSON.stringify(legacy), fallback).upstream_status, 404)
})

test('file-selection failure exposes one safe cause and cancellation remains silent', async () => {
  const source = await readFile(new URL('../components/common/FileSelector.vue', import.meta.url), 'utf8')
  const handler = source.match(/const selectFile = async \(\) => \{[\s\S]*?\n\}/)?.[0]
  assert.ok(handler)
  assert.match(source, /import \{ ElMessage \} from 'element-plus'/)
  assert.match(source, /const \{ t \} = useI18n\(\)/)
  const notices = []
  let selected = null
  const context = {
    props: { multiple: false, type: 'image', defaultPath: '' },
    allowedTypes: { image: {} }, pictureDir: async () => '/pictures', documentDir: async () => '/docs',
    open: async () => { if (selected instanceof Error) throw selected; return selected },
    selectPath: { value: [] }, FileUrls: { value: [] }, loading: { value: false },
    emit() {}, t: () => fallback, formatUserError,
    ElMessage: { error: value => notices.push(value) }
  }
  const selectFile = runInNewContext(`${handler}; selectFile`, context)
  await selectFile()
  assert.equal(notices.length, 0)
  selected = new Error('failed: token=CANARY')
  await selectFile()
  assert.equal(notices.length, 1)
  assert.ok(notices[0].startsWith('failed:'))
  assert.ok(!notices[0].includes('CANARY'))
})

test('key unlock renders only the safe cause and preserves failure-state cleanup', async () => {
  const source = await readFile(new URL('../components/common/ApiKeyUnlockDialog.vue', import.meta.url), 'utf8')
  const handler = source.match(/const selectKeyFile = async \(\) => \{[\s\S]*?\n\}/)?.[0]
  assert.ok(handler)
  const notices = []
  const selecting = { value: false }
  const visible = { value: true }
  const selectKeyFile = runInNewContext(`${handler}; selectKeyFile`, {
    open: async () => '/safe/key.csk', t: () => fallback, selecting, visible,
    status: { value: null }, console: { error() {} },
    invokeWrapper: async () => { throw { presentation: { schema_version: 1, message: 'invalid key token=CANARY' } } },
    showMessage: (...args) => notices.push(args), formatUserError
  })
  await selectKeyFile()
  assert.equal(selecting.value, false)
  assert.equal(visible.value, true)
  assert.equal(notices.length, 1)
  assert.equal(notices[0][1], 'error')
  assert.ok(notices[0][0].startsWith('invalid key'))
  assert.ok(!notices[0][0].includes('CANARY'))
})

test('raw protocol errors and exact Ai serde struct details retain useful data', () => {
  const samples = [
    [{ error: { message: 'upstream refused', code: 'denied' } }, 'upstream refused'],
    [{ error: 'ollama failure' }, 'ollama failure'],
    [{ errors: { message: 'protocol failure' } }, 'protocol failure'],
    [{ module: 'Sensitive', details: { kind: 'IoError', details: { message: 'read failed' } } }, 'read failed'],
    [{ module: 'Sensitive', details: { kind: 'RegexCompilationFailed', details: { pattern: 'x', message: 'invalid regex' } } }, 'invalid regex']
  ]
  for (const [value, expected] of samples) {
    assert.equal(formatUserError(value, fallback), expected)
    assert.equal(formatUserError(JSON.stringify(value), fallback), expected)
  }
  for (const kind of ['apiRequestFailed', 'rawApiRequestFailed']) {
    for (const details of ['overloaded', JSON.stringify({ error: { code: 'busy', message: 'overloaded' } }),
      JSON.stringify({ errors: { message: 'overloaded' } }), JSON.stringify({ error: 'overloaded' })]) {
      const value = { module: 'Ai', details: { kind, message: { status_code: 503, provider: 'x', details } } }
      for (const wire of [value, JSON.stringify(value)]) {
        const normalized = normalizeUserError(wire, fallback)
        assert.equal(normalized.message, 'overloaded')
        assert.equal(normalized.upstream_status, 503)
        assert.equal(normalized.module, 'Ai')
        assert.equal(normalized.kind, kind)
      }
    }
  }
  assert.equal(formatUserError({ module: 'Unknown', details: { message: { details: 'hidden' } } }, fallback), fallback)
  assert.equal(normalizeUserError({ status_code: 503, message: 'x' }).upstream_status, undefined)
})

test('presentation v1 takes priority and malformed presentations fall back to legacy', () => {
  const value = { code: 'legacy_code', message: 'old', presentation: {
    schema_version: 1, message: 'new', reason: 'session_stopping', upstream_status: 409
  } }
  assert.deepEqual(normalizeUserError(value, fallback), {
    code: 'legacy_code', message: 'new', reason: 'session_stopping', upstream_status: 409
  })
  for (const presentation of [null, {}, { schema_version: 2, message: 'bad' },
    { schema_version: 1, message: {} }, { schema_version: 1, message: '' },
    { schema_version: 1, message: 'bad', upstream_status: '503' },
    { schema_version: 1, message: 'bad', reason: {} }]) {
    assert.equal(formatUserError({ ...value, presentation }, fallback), 'old')
  }
  assert.equal(formatUserError({ details: { presentation: value.presentation } }, fallback), 'new')
})

test('reason constants are exact; message keywords cannot infer reason, code or status', () => {
  for (const reason of Object.values(USER_ERROR_REASONS)) {
    assert.equal(normalizeUserError({ reason, message: 'x' }).reason, reason)
  }
  for (const reason of ['SessionStopping', 'SESSION_STOPPING', 'session_stopping ', 'future_reason', null, {}]) {
    assert.equal(normalizeUserError({ presentation: { schema_version: 1, reason, message: 'x' } }).reason, 'unknown')
  }
  assert.deepEqual(normalizeUserError('session_already_exists file_too_large HTTP 401', fallback), {
    message: 'session_already_exists file_too_large HTTP 401', reason: 'unknown'
  })
  assert.equal(normalizeUserError({ message: 'x', status: 503 }).upstream_status, 503)
  assert.equal(normalizeUserError({ message: 'x', status: '503' }).upstream_status, undefined)
})

test('null, cycles, deep unknown messages, throwing getters and HTML use safe fallback', () => {
  const cycle = {}; cycle.message = cycle; cycle.details = cycle
  let deep = { message: 'not reached' }
  for (let i = 0; i < 1000; i++) deep = { message: deep }
  const hostile = { get message() { throw new Error('secret') } }
  for (const value of [null, undefined, {}, [], 123, cycle, deep, hostile, new Error(), 'TypeError', '[object Object]',
    '<!DOCTYPE html><html><body>502 upstream</body></html>', '<div>bad gateway</div>', { message: '<html>bad gateway</html>' }]) {
    assert.equal(formatUserError(value, fallback), fallback)
  }
})

test('credentials are redacted before display in all supported message sources', () => {
  const canary = 'UNIQUE_CREDENTIAL_CANARY_9283'
  const messages = [
    `failed: {"api_key":"${canary}"}`,
    `failed: {"API_KEY":"${canary}"}`,
    `Authorization: Bearer ${canary}`,
    `failed: ${JSON.stringify({ Authorization: `Basic ${canary}` })}`,
    `Cookie: session=${canary}`,
    `escaped: ${JSON.stringify(JSON.stringify({ api_key: canary }))}`,
    `Bearer ${canary}`,
    `CUSTOM_ENV=${canary}`,
    `X-Custom-Header: ${canary}`,
    `headers: {"arbitrary":"${canary}"}`,
    `env: {"anything":"${canary}"}`,
    `env: {"nested":{"first":"ok"},"arbitrary":"${canary}"}`,
    `https://user:${canary}@example.com/path?api_key=${canary}&token=${canary}&page=2`,
    `https://example.com?access_token=${canary}`
  ]
  for (const message of messages) {
    for (const value of [message, new Error(message), { message }, { presentation: { schema_version: 1, message } },
      JSON.stringify({ module: 'Ai', details: { message } })]) {
      const rendered = formatUserError(value, fallback)
      assert.ok(!rendered.includes(canary), rendered)
      assert.ok(rendered.includes('[REDACTED]'), rendered)
    }
  }
})

test('credential containers redact multiline, escaped and nested arbitrary values without eating suffixes', () => {
  const canary = 'CONTAINER_CANARY_71629'
  for (const name of ['env', 'environment', 'headers']) {
    const container = { [name]: { arbitrary: canary, nested: [{ odd: `brace } quote " ${canary}` }],
      list: [canary, 42, null], unusual: { deeper: canary } } }
    const json = JSON.stringify(container, null, 2)
    const variants = [json, JSON.stringify(json), JSON.stringify(JSON.stringify(json)),
      `${name}: {\n arbitrary: '${canary}',\n nested: {other: '${canary}'}\n}`,
      `${name}: [\n "${canary}", {"odd":"${canary}"}\n]`]
    for (const variant of variants) {
      const message = `before: ${variant}\nafter: useful cause`
      for (const value of [message, { error: { message } },
        { presentation: { schema_version: 1, message } }, JSON.stringify({ message })]) {
        const rendered = formatUserError(value, fallback)
        assert.ok(!rendered.includes(canary), rendered)
        assert.ok(rendered.startsWith('before: '), rendered)
        assert.ok(rendered.endsWith('after: useful cause'), rendered)
        assert.ok(rendered.includes('[REDACTED]'), rendered)
      }
    }
  }
  for (const suffix of [`{broken: '${canary}'`, `[wrong} ${canary}`, '{'.repeat(257) + canary,
    `{"long":"${'x'.repeat(131072)}${canary}"}`]) {
    assert.equal(formatUserError(`before env: ${suffix}`, fallback), 'before env: [REDACTED]')
  }
})

test('Unicode length limit does not split surrogate pairs and fallback provider stays runtime independent', () => {
  assert.equal(formatUserError('\u{20000}'.repeat(4097), fallback), '\u{20000}'.repeat(4096) + '…')
  assert.equal(formatUserError('\u{20000}'.repeat(4096), fallback), '\u{20000}'.repeat(4096))
  setUserErrorFallback(() => fallback)
  assert.equal(formatUserError(null), fallback)
  setUserErrorFallback(null)
  assert.equal(formatUserError(null, fallback), fallback)
})

test('Tauri wrapper keeps FrontendAppError contract and normalizes invoke rejection', async () => {
  // Substitute runtime imports only, exercising the actual wrapper without Tauri or Vue.
  const source = await readFile(new URL('./tauri.js', import.meta.url), 'utf8')
  const errorModule = new URL('./error.js', import.meta.url).href
  const isolated = source
    .replace("import { invoke } from '@tauri-apps/api/core'", 'const invoke = async () => { throw globalThis.__userErrorFixture }')
    .replace("import i18n from '../i18n'", `const i18n = { global: { t: () => '${fallback}' } }`)
    .replace("'./error.js'", JSON.stringify(errorModule))
  const { FrontendAppError, invokeWrapper } = await import(`data:text/javascript;base64,${Buffer.from(isolated).toString('base64')}`)
  const original = { module: 'Ai', details: { kind: 'Upstream', message: { code: 'blocked', message: 'safe',
    presentation: { schema_version: 1, message: 'preferred', reason: 'runtime_stopping', upstream_status: 503 } } } }
  globalThis.__userErrorFixture = original
  try {
    await assert.rejects(invokeWrapper('test'), error => {
      assert.ok(error instanceof FrontendAppError)
      assert.ok(error instanceof Error)
      assert.equal(error.name, 'FrontendAppError')
      assert.equal(error.module, 'Ai')
      assert.equal(error.kind, 'Upstream')
      assert.equal(error.message, 'preferred')
      assert.equal(error.originalError, original)
      assert.equal(error.code, 'blocked')
      assert.equal(error.reason, 'runtime_stopping')
      assert.equal(error.upstream_status, 503)
      assert.equal(error.toFormattedString(), '[Ai:Upstream] preferred')
      return true
    })
    const detail = { code: 'busy', message: 'legacy', presentation: {
      schema_version: 1, message: 'overloaded', reason: 'runtime_stopping', upstream_status: 503
    } }
    const expected = normalizeUserError({ error: detail }, fallback)
    for (const wire of [detail, JSON.stringify(detail), { error: detail }, JSON.stringify({ error: detail })]) {
      globalThis.__userErrorFixture = wire
      await assert.rejects(invokeWrapper('test'), error => {
        assert.equal(error.message, expected.message)
        assert.equal(error.code, expected.code)
        assert.equal(error.reason, expected.reason)
        assert.equal(error.upstream_status, expected.upstream_status)
        return true
      })
    }
    assert.equal(new FrontendAppError('M', 'K', { message: 'nested safe' }, original).message, 'nested safe')
  } finally {
    delete globalThis.__userErrorFixture
    setUserErrorFallback(null)
  }
})
