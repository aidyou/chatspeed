import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const projectRoot = new URL('../../../', import.meta.url)
const sourceUrl = new URL('src/composables/workflow/useWorkflowInputDraftCache.ts', projectRoot)
const source = readFileSync(sourceUrl, 'utf8')
const storage = new Map()
let failSetStorage = false

globalThis.__workflowDraftTestStorage = {
  get(key, fallback) {
    return storage.has(key) ? storage.get(key) : fallback
  },
  set(key, value) {
    if (failSetStorage) throw new Error('quota exceeded')
    storage.set(key, value)
  },
  remove(key) {
    storage.delete(key)
  }
}

const moduleSource = source.replace(
  "import { csGetStorage, csRemoveStorage, csSetStorage } from '@/libs/util'",
  `const { get: csGetStorage, set: csSetStorage, remove: csRemoveStorage } = globalThis.__workflowDraftTestStorage`
)
const draftCache = await import(`data:text/javascript;charset=utf-8,${encodeURIComponent(moduleSource)}`)

function resetStorage() {
  storage.clear()
  failSetStorage = false
}

function dataUrl(size) {
  return `data:image/png;base64,${'a'.repeat(size)}`
}

resetStorage()
assert.equal(draftCache.workflowInputDraftKey('wf-a'), 'workflow-input-draft:wf-a')
assert.equal(
  draftCache.saveWorkflowInputDraft('wf-a', {
    inputMessage: 'hello',
    attachments: [{ id: 'img-1', type: 'image', name: 'small', url: dataUrl(32), sourceUrl: dataUrl(32) }]
  }),
  true
)
assert.equal(draftCache.loadWorkflowInputDraft('wf-a').inputMessage, 'hello')
assert.equal(draftCache.loadWorkflowInputDraft('wf-a').attachments.length, 1)
assert.equal(draftCache.loadWorkflowInputDraft('wf-b'), null)

draftCache.removeWorkflowInputDraft('wf-a')
assert.equal(draftCache.loadWorkflowInputDraft('wf-a'), null)

resetStorage()
assert.equal(
  draftCache.saveWorkflowInputDraft('wf-large', {
    inputMessage: 'keep text',
    attachments: [
      { id: 'big-1', type: 'image', name: 'big-1', url: dataUrl(500 * 1024), sourceUrl: dataUrl(500 * 1024) },
      { id: 'big-2', type: 'image', name: 'big-2', url: dataUrl(500 * 1024), sourceUrl: dataUrl(500 * 1024) },
      { id: 'big-3', type: 'image', name: 'big-3', url: dataUrl(500 * 1024), sourceUrl: dataUrl(500 * 1024) }
    ]
  }),
  true
)
const largeDraft = draftCache.loadWorkflowInputDraft('wf-large')
assert.equal(largeDraft.inputMessage, 'keep text')
assert.ok(JSON.stringify(storage.get(draftCache.workflowInputDraftKey('wf-large'))).length <= 2 * 1024 * 1024)
assert.ok(largeDraft.attachments.length < 3)

resetStorage()
assert.equal(
  draftCache.saveWorkflowInputDraft('wf-huge', {
    inputMessage: 'text survives huge image',
    attachments: [{ id: 'huge', type: 'image', name: 'huge', url: dataUrl(600 * 1024), sourceUrl: dataUrl(600 * 1024) }]
  }),
  true
)
const hugeDraft = draftCache.loadWorkflowInputDraft('wf-huge')
assert.equal(hugeDraft.inputMessage, 'text survives huge image')
assert.equal(hugeDraft.attachments.length, 0)

resetStorage()
assert.equal(
  draftCache.saveWorkflowInputDraft('wf-oversized-text', {
    inputMessage: 'x'.repeat(3 * 1024 * 1024),
    attachments: []
  }),
  true
)
const oversizedTextDraft = draftCache.loadWorkflowInputDraft('wf-oversized-text')
assert.ok(oversizedTextDraft.inputMessage.length > 0)
assert.ok(oversizedTextDraft.inputMessage.length < 3 * 1024 * 1024)
assert.ok(
  JSON.stringify(storage.get(draftCache.workflowInputDraftKey('wf-oversized-text'))).length <=
    2 * 1024 * 1024
)

resetStorage()
failSetStorage = true
assert.equal(
  draftCache.saveWorkflowInputDraft('wf-quota', {
    inputMessage: 'quota text',
    attachments: [{ id: 'img', type: 'image', name: 'img', url: dataUrl(32), sourceUrl: dataUrl(32) }]
  }),
  false
)

console.log('workflow input draft cache tests passed')
