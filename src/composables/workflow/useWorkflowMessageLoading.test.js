import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import test from 'node:test'
import { nextTick, ref, watch } from 'vue'
import { compileScript, compileStyleAsync, compileTemplate, parse } from 'vue/compiler-sfc'
import * as sass from 'sass-embedded'
import { useWorkflowMessageLoading } from './useWorkflowMessageLoading.js'

const require = createRequire(import.meta.url)

const drainMicrotasks = async () => {
  for (let turn = 0; turn < 6; turn += 1) await nextTick()
}

const createFrames = context => {
  const previousRequest = globalThis.requestAnimationFrame
  const previousCancel = globalThis.cancelAnimationFrame
  let nextId = 0
  const callbacks = new Map()
  globalThis.requestAnimationFrame = callback => {
    const frameId = ++nextId
    callbacks.set(frameId, callback)
    return frameId
  }
  globalThis.cancelAnimationFrame = frameId => callbacks.delete(frameId)
  context.after(() => {
    if (previousRequest) globalThis.requestAnimationFrame = previousRequest
    else delete globalThis.requestAnimationFrame
    if (previousCancel) globalThis.cancelAnimationFrame = previousCancel
    else delete globalThis.cancelAnimationFrame
  })

  const advance = async () => {
    await drainMicrotasks()
    const frame = [...callbacks.values()]
    callbacks.clear()
    for (const callback of frame) callback()
    await drainMicrotasks()
  }

  return { callbacks, advance }
}

test('skeleton paints before mounting history and remains through Markdown and layout preparation', async context => {
  const frames = createFrames(context)
  const loading = ref(true)
  const sessionId = ref('long-session')
  const steps = []
  const state = useWorkflowMessageLoading({
    isLoading: loading,
    sessionId,
    onContentMounted: () => {
      steps.push('layout')
      assert.equal(state.showLoadingSkeleton.value, true)
    }
  })
  context.after(state.dispose)
  const stopRender = watch(state.renderMessages, async render => {
    if (!render) return
    for (let index = 0; index < 300; index += 1) steps.push('markdown')
    await nextTick()
    steps.push('highlight')
    assert.equal(state.showLoadingSkeleton.value, true)
  })
  context.after(stopRender)

  assert.equal(state.showLoadingSkeleton.value, true)
  assert.equal(state.renderMessages.value, false)
  loading.value = false
  await frames.advance()
  assert.equal(state.renderMessages.value, false, 'allow a skeleton paint before expensive mounts')
  assert.deepEqual(steps, [])
  await frames.advance()
  assert.equal(state.renderMessages.value, true)
  assert.equal(state.showLoadingSkeleton.value, true, 'data arrival must not remove the skeleton')
  assert.equal(steps.filter(step => step === 'markdown').length, 300)
  assert.ok(steps.includes('highlight'))
  assert.ok(steps.includes('layout'))
  await frames.advance()
  assert.equal(state.showLoadingSkeleton.value, false)
  assert.equal(frames.callbacks.size, 0)
})

test('a newer session cancels pending rendering and stale callbacks cannot dismiss its skeleton', async context => {
  const frames = createFrames(context)
  const loading = ref(false)
  const sessionId = ref('first')
  const mountedSessions = []
  const state = useWorkflowMessageLoading({
    isLoading: loading,
    sessionId,
    onContentMounted: () => mountedSessions.push(sessionId.value)
  })
  context.after(state.dispose)

  await frames.advance()
  const staleMount = [...frames.callbacks.values()][0]
  sessionId.value = 'second'
  loading.value = true
  await drainMicrotasks()
  staleMount()
  await drainMicrotasks()
  assert.equal(state.renderMessages.value, false)
  assert.equal(state.showLoadingSkeleton.value, true)
  assert.deepEqual(mountedSessions, [])

  loading.value = false
  await frames.advance()
  await frames.advance()
  const staleReveal = [...frames.callbacks.values()][0]
  loading.value = true
  await drainMicrotasks()
  staleReveal()
  assert.equal(state.showLoadingSkeleton.value, true)
  assert.equal(state.renderMessages.value, false)
  assert.deepEqual(mountedSessions, ['second'])

  loading.value = false
  await frames.advance()
  await frames.advance()
  await frames.advance()
  assert.equal(state.showLoadingSkeleton.value, false)
})

test('empty history finishes loading and stream-only updates do not restart the skeleton', async context => {
  const frames = createFrames(context)
  const loading = ref(false)
  const source = ref({ sessionId: 'empty', text: '' })
  let mounts = 0
  const state = useWorkflowMessageLoading({
    isLoading: loading,
    sessionId: () => source.value.sessionId,
    onContentMounted: () => { mounts += 1 }
  })
  context.after(state.dispose)
  await frames.advance()
  await frames.advance()
  await frames.advance()
  assert.equal(state.showLoadingSkeleton.value, false)
  assert.equal(state.renderMessages.value, true)

  for (let index = 0; index < 1000; index += 1) {
    source.value.text += 'chunk'
    await nextTick()
  }
  assert.equal(mounts, 1)
  assert.equal(frames.callbacks.size, 0)
  assert.equal(state.showLoadingSkeleton.value, false)
})

test('unmount cancels pending frames and ignores outstanding nextTick callbacks', async context => {
  const frames = createFrames(context)
  for (const advanceBeforeDispose of [false, true]) {
    let mounts = 0
    const state = useWorkflowMessageLoading({
      isLoading: ref(false),
      sessionId: ref('unmount'),
      onContentMounted: () => { mounts += 1 }
    })
    if (advanceBeforeDispose) await frames.advance()
    const stale = [...frames.callbacks.values()]
    state.dispose()
    for (const callback of stale) callback()
    await drainMicrotasks()
    assert.equal(mounts, 0)
    assert.equal(frames.callbacks.size, 0)
    assert.equal(state.renderMessages.value, false)
  }
})

test('message list overlay and content compile together without changing the scroll container', async () => {
  const filename = 'WorkflowMessageList.vue'
  const source = readFileSync(new URL('../../components/workflow/WorkflowMessageList.vue', import.meta.url), 'utf8')
  const { descriptor, errors } = parse(source, { filename })
  assert.deepEqual(errors, [])
  const script = compileScript(descriptor, { id: 'message-loading' })
  const template = compileTemplate({
    source: descriptor.template.content,
    filename,
    id: 'message-loading',
    compilerOptions: { bindingMetadata: script.bindings }
  })
  assert.deepEqual(template.errors, [])
  assert.match(source, /class="messages"\s+ref="messagesRef"/)
  assert.match(source, /onContentMounted:[\s\S]*?scrollController\.requestContentChange\(\)/)
  assert.match(source, /messageLoading\.dispose\(\)/)
  assert.match(source, /\.message-loading-overlay\s*\{\s*position: absolute;\s*inset: 0;/)
  for (const style of descriptor.styles) {
    const result = await compileStyleAsync({
      source: style.content,
      filename,
      id: 'message-loading',
      preprocessLang: style.lang,
      preprocessCustomRequire: name => name === 'sass' ? sass : require(name)
    })
    assert.deepEqual(result.errors, [])
  }
})
