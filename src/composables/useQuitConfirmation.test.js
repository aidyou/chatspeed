import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { computed, ref } from 'vue'

const source = readFileSync(new URL('./useQuitConfirmation.js', import.meta.url), 'utf8')

// The module imports Vue and the Tauri event API at its top level. Stripping the
// imports (and the `export` keywords) lets the state machine run against an
// injected command and listener registry, with no Tauri or live runtime needed.
const moduleBody = source.replace(/^import .*\n/gm, '').replace(/^export /gm, '')

function loadComposable(invoke, subscribe) {
  const createQuitConfirmation = runInNewContext(
    `${moduleBody}\ncreateQuitConfirmation`,
    { computed, ref, console }
  )
  return createQuitConfirmation({ invoke, subscribe })
}

const noopSubscribe = async () => () => {}

// The command arguments are built inside the isolated module context, so compare
// their plain data rather than their prototype identity.
const plain = value => JSON.parse(JSON.stringify(value))

test('the checkbox starts unchecked and only a new prompt resets it', () => {
  const controller = loadComposable(async () => ({ phase: 'confirming' }), noopSubscribe)

  controller.open({ nonce: 'n1', host: 'workflow' })
  assert.equal(controller.active.value, true)
  assert.equal(controller.phase.value, 'confirming')
  assert.equal(controller.stopRuntime.value, false)

  // A repeated tray click re-publishes the same prompt: the user's current choice
  // must survive it.
  controller.stopRuntime.value = true
  controller.open({ nonce: 'n1', host: 'workflow' })
  assert.equal(controller.stopRuntime.value, true)

  // A genuinely new prompt always starts unchecked.
  controller.open({ nonce: 'n2', host: 'workflow' })
  assert.equal(controller.stopRuntime.value, false)
})

test('a pending prompt is opened from the mount-time query', async () => {
  const controller = loadComposable(
    async () => ({ phase: 'confirming', prompt: { nonce: 'pending-1', host: 'workflow' } }),
    noopSubscribe
  )

  await controller.requestPending()
  assert.equal(controller.active.value, true)
  assert.equal(controller.nonce.value, 'pending-1')
  assert.equal(controller.host.value, 'workflow')
})

test('cancel invokes quit_cancel with the bound nonce and host, then returns to idle', async () => {
  const calls = []
  const controller = loadComposable(async (command, args) => {
    calls.push([command, args])
    return { phase: 'idle' }
  }, noopSubscribe)

  controller.open({ nonce: 'n1', host: 'workflow' })
  controller.stopRuntime.value = true
  await controller.cancel()

  assert.deepEqual(plain(calls), [['quit_cancel', { nonce: 'n1', host: 'workflow' }]])
  assert.equal(controller.active.value, false)
  assert.equal(controller.phase.value, 'idle')
  assert.equal(controller.stopRuntime.value, false)
})

test('submit sends the current choice and applies the returned status', async () => {
  const calls = []
  const controller = loadComposable(async (command, args) => {
    calls.push([command, args])
    return { phase: 'stopping_runtime', prompt: { nonce: 'n1', host: 'workflow' } }
  }, noopSubscribe)

  controller.open({ nonce: 'n1', host: 'workflow' })
  controller.stopRuntime.value = true
  await controller.submit()

  assert.deepEqual(plain(calls), [
    ['quit_confirm', { nonce: 'n1', host: 'workflow', stopRuntime: true }]
  ])
  assert.equal(controller.phase.value, 'stopping_runtime')
  assert.equal(controller.busy.value, true)
  assert.equal(controller.failed.value, false)
})

test('a failed stop exposes its code and stays retryable', async () => {
  const calls = []
  let response = {
    phase: 'stopping_runtime',
    error: 'runtime_stop_timeout',
    prompt: { nonce: 'n1', host: 'workflow' }
  }
  const controller = loadComposable(async (command, args) => {
    calls.push([command, args])
    return response
  }, noopSubscribe)

  controller.open({ nonce: 'n1', host: 'workflow' })
  controller.stopRuntime.value = true
  await controller.submit()

  assert.equal(controller.failed.value, true)
  assert.equal(controller.error.value, 'runtime_stop_timeout')
  assert.equal(controller.active.value, true)

  // While the stop is failed, the dialog offers an explicit retry.
  response = { phase: 'committing_exit' }
  controller.stopRuntime.value = true
  await controller.submit()

  assert.equal(calls.length, 2)
  assert.equal(controller.active.value, false)
  assert.equal(controller.phase.value, 'committing_exit')
})

test('a duplicate submit is ignored while one is in flight', async () => {
  let resolveSubmit
  let calls = 0
  const controller = loadComposable(() => {
    calls += 1
    return new Promise(resolve => {
      resolveSubmit = resolve
    })
  }, noopSubscribe)

  controller.open({ nonce: 'n1', host: 'workflow' })
  const first = controller.submit()
  // The second call must not reach the backend.
  await controller.submit()
  assert.equal(calls, 1)

  resolveSubmit({ phase: 'confirming' })
  await first
  assert.equal(calls, 1)
})

test('init registers both events and disposes them, and still reads a pending prompt', async () => {
  const handlers = {}
  const subscribe = async (event, handler) => {
    handlers[event] = handler
    return () => {
      delete handlers[event]
    }
  }
  const controller = loadComposable(async () => ({ phase: 'idle' }), subscribe)

  const dispose = await controller.init()
  assert.ok(handlers['quit://request'])
  assert.ok(handlers['quit://status'])

  handlers['quit://request']({ payload: { nonce: 'n9', host: 'workflow' } })
  assert.equal(controller.active.value, true)
  assert.equal(controller.nonce.value, 'n9')

  // The status event closes the dialog when the coordinator reports idle.
  handlers['quit://status']({ payload: { phase: 'idle' } })
  assert.equal(controller.active.value, false)

  dispose()
  assert.deepEqual(Object.keys(handlers), [])
})