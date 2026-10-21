import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import { compileScript, compileTemplate, parse as parseSfc } from 'vue/compiler-sfc'

import { USER_ERROR_REASONS } from '../constants/errorReasons.js'
import { formatUserError, normalizeUserError } from '../libs/error.js'

const read = file => readFileSync(new URL(`./${file}`, import.meta.url), 'utf8')

const messages = {
  'common.errorFallback': 'Something went wrong. Please try again.',
  'common.deleteSuccess': 'Deleted successfully',
  'common.warning': 'Warning',
  'chat.messageCopied': 'Message copied',
  'chat.errorOnCopyMessage': 'Failed to copy message: {error}',
  'chat.errorOnSearchNotes': 'Failed to search notes: {error}',
  'settings.sandbox.deleteConfirm': 'Delete {name}?',
  'settings.chatHub.deleteConfirm': 'Delete chat entry {name}?',
  'settings.chatHub.deleteConfirmTitle': 'Delete Chat Entry',
  'settings.chatHub.deleteSuccess': 'Deleted chat entry {name}',
  'settings.chatHub.deleteFailed': 'Failed to delete the chat entry',
  'settings.agent.disableFailed': 'Failed to disable agent "{name}": {error}',
  'settings.agent.disableSuccess': 'Agent "{name}" has been disabled.',
  'settings.agent.enableFailed': 'Failed to enable agent "{name}": {error}',
  'settings.agent.enableSuccess': 'Agent "{name}" has been enabled.',
  'settings.agentSkills.reconcileDone': 'Reconcile complete',
  'settings.agentSkills.operationFailed': 'Skill operation failed',
  'settings.model.decisionImportFailed': 'Could not import decision models: {error}.'
}

const t = (key, params = {}) =>
  (messages[key] ?? key).replace(/\{(\w+)\}/g, (placeholder, name) =>
    params[name] === undefined ? placeholder : String(params[name])
  )

/**
 * Mirrors the renderer error object without the browser/Tauri imports, so
 * handlers can keep their `instanceof FrontendAppError` log branches.
 */
class FrontendAppError extends Error {
  constructor(message, originalError) {
    super(message)
    this.name = 'FrontendAppError'
    this.module = 'Ai'
    this.kind = 'Upstream'
    this.originalError = originalError
  }

  toFormattedString() {
    return `[Ai:Upstream] ${this.message}`
  }
}

const extractConst = (source, name) => {
  const match = source.match(new RegExp(`const ${name} = [^\\n]*\\{[\\s\\S]*?\\n\\}`))
  assert.ok(match, `${name} should remain an extractable handler`)
  return match[0]
}

const extractFunction = (source, name) => {
  const match = source.match(new RegExp(`(?:async )?function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`))
  assert.ok(match, `${name} should remain an extractable handler`)
  return match[0]
}

const loadHandler = (source, name, context, kind = 'const') => {
  const declaration = kind === 'const' ? extractConst(source, name) : extractFunction(source, name)
  return runInNewContext(`${declaration}\n${name}`, context)
}

const clipboard = () => {
  const written = []
  return {
    written,
    navigator: { clipboard: { writeText: text => written.push(text) } }
  }
}

const indexCopy = async ({ writeText, messages: chatMessages = [] }) => {
  const state = clipboard()
  const notifications = []
  const handler = loadHandler(read('Index.vue'), 'onCopyMessage', {
    chatStore: { messages: chatMessages },
    navigator: { clipboard: { writeText: writeText || state.navigator.clipboard.writeText } },
    formatUserError,
    t,
    showMessage: (...args) => notifications.push(args)
  })
  handler(1)
  return { notifications, written: state.written }
}

test('Index copy handler keeps success silent and shows the formatted cause once on failure', async () => {
  const success = await indexCopy({ messages: [{ id: 1, content: 'hello' }] })
  assert.deepEqual(success.notifications, [['Message copied', 'success', 1000]])

  const failure = await indexCopy({
    writeText: () => {
      throw new FrontendAppError('clipboard denied')
    },
    messages: [{ id: 1, content: 'hello' }]
  })
  assert.equal(failure.notifications.length, 1)
  assert.deepEqual(failure.notifications[0], [
    'Failed to copy message: clipboard denied',
    'error',
    3000
  ])
  assert.doesNotMatch(failure.notifications[0][0], /Ai:Upstream/)
})

const searchNotes = async error => {
  const notifications = []
  const searchResult = { value: [] }
  const handler = loadHandler(read('Note.vue'), 'onSearchNote', {
    kw: { value: 'alpha' },
    searchResult,
    noteStore: {
      searchNotes: error ? async () => Promise.reject(error) : async () => [{ id: 9 }]
    },
    FrontendAppError,
    formatUserError,
    t,
    showMessage: (...args) => notifications.push(args)
  })
  await handler()
  return { notifications, searchResult }
}

test('Note search refreshes results on success and localizes unknown failures', async () => {
  const success = await searchNotes(null)
  assert.deepEqual(success.searchResult.value, [{ id: 9 }])
  assert.deepEqual(success.notifications, [])

  const plain = await searchNotes(new FrontendAppError('index missing'))
  assert.deepEqual(plain.notifications, [
    ['Failed to search notes: index missing', 'error', 3000]
  ])
  assert.doesNotMatch(plain.notifications[0][0], /Ai:Upstream/)

  const unknown = await searchNotes({})
  assert.deepEqual(unknown.notifications, [
    ['Failed to search notes: Something went wrong. Please try again.', 'error', 3000]
  ])
})

const removeScheme = async (confirm, invoke) => {
  const notifications = []
  const deleted = []
  const fetched = []
  const handler = loadHandler(read('../components/setting/Sandbox.vue'), 'remove', {
    t,
    formatUserError,
    ElMessageBox: { confirm },
    invoke: async (command, payload) => {
      deleted.push([command, payload])
      return invoke(command, payload)
    },
    schemeStore: { fetchSchemes: async () => fetched.push('fetched') },
    showMessage: (...args) => notifications.push(args)
  })
  await handler({ id: 7, name: 'strict' })
  return { notifications, deleted, fetched }
}

test('Sandbox delete keeps cancellation silent and reports a failed delete once', async () => {
  const cancelled = await removeScheme(() => Promise.reject('cancel'), async () => {})
  assert.deepEqual(cancelled.notifications, [])
  assert.deepEqual(cancelled.deleted, [])

  const success = await removeScheme(() => Promise.resolve(), async () => {})
  assert.equal(success.notifications.length, 1)
  assert.equal(success.notifications[0][0], 'Deleted successfully')
  assert.equal(success.notifications[0][1], 'success')
  assert.equal(success.deleted.length, 1)
  assert.equal(success.deleted[0][0], 'delete_sandbox_scheme')
  assert.equal(success.deleted[0][1].id, 7)
  assert.deepEqual(success.fetched, ['fetched'])

  const failed = await removeScheme(
    () => Promise.resolve(),
    async () => Promise.reject(new FrontendAppError('scheme locked'))
  )
  assert.deepEqual(failed.notifications, [['scheme locked', 'error']])
  assert.doesNotMatch(failed.notifications[0][0], /Ai:Upstream/)
})

const deleteChatHub = async (confirm, remove) => {
  const notifications = []
  const removed = []
  const context = {
    t,
    formatUserError,
    FrontendAppError,
    console: { error: () => {} },
    ElMessageBox: { confirm },
    chatHubStore: {
      remove: async id => {
        removed.push(id)
        return remove()
      }
    },
    chatHubBrokenLogos: { value: { delete: () => {} } },
    showMessage: (...args) => notifications.push(args)
  }
  const source = read('../components/setting/ChatHub.vue')
  const handler = runInNewContext(
    `${extractConst(source, 'reportChatHubError')}\n${extractConst(source, 'deleteChatHub')}\ndeleteChatHub`,
    context
  )
  await handler({ id: 3, name: 'Docs' })
  return { notifications, removed }
}

test('ChatHub delete treats the confirm dismissal as cancel and formats a failed remove', async () => {
  const cancelled = await deleteChatHub(() => Promise.reject(new Error('cancel')), async () => {})
  assert.deepEqual(cancelled.notifications, [])
  assert.deepEqual(cancelled.removed, [])

  const success = await deleteChatHub(() => Promise.resolve(), async () => {})
  assert.deepEqual(success.notifications, [['Deleted chat entry Docs', 'success']])
  assert.deepEqual(success.removed, [3])

  const failed = await deleteChatHub(
    () => Promise.resolve(),
    async () => Promise.reject(new FrontendAppError('entry in use'))
  )
  assert.deepEqual(failed.notifications, [['entry in use', 'error']])
  assert.doesNotMatch(failed.notifications[0][0], /Ai:Upstream/)
})

const toggleAgent = async error => {
  const notifications = []
  const agent = { disabled: false, name: 'Writer' }
  const handler = loadHandler(read('../components/setting/Agent.vue'), 'toggleAgentStatus', {
    t,
    formatUserError,
    agentStore: {
      saveAgent: error ? async () => Promise.reject(error) : async () => {}
    },
    showMessage: (...args) => notifications.push(args)
  })
  await handler(agent)
  return { agent, notifications }
}

test('Agent status toggle reports the localized failure and restores the previous state', async () => {
  const success = await toggleAgent(null)
  assert.equal(success.agent.disabled, true)
  assert.deepEqual(success.notifications, [['Agent "Writer" has been disabled.', 'success']])

  const failed = await toggleAgent(new FrontendAppError('agent busy'))
  assert.equal(failed.agent.disabled, false)
  assert.deepEqual(failed.notifications, [
    ['Failed to disable agent "Writer": agent busy', 'error']
  ])
  assert.doesNotMatch(failed.notifications[0][0], /Ai:Upstream/)
})

const reconcile = async error => {
  const operationError = { value: '' }
  const operationErrorVisible = { value: false }
  const success = []
  const context = {
    t,
    formatUserError,
    operationError,
    operationErrorVisible,
    ElMessage: { success: message => success.push(message) },
    store: { reconcile: error ? async () => Promise.reject(error) : async () => {} }
  }
  const source = read('../components/setting/AgentSkills.vue')
  const handler = runInNewContext(
    `${extractFunction(source, 'showOperationError')}\n${extractFunction(source, 'runReconcile')}\nrunReconcile`,
    context
  )
  await handler()
  return { operationError, operationErrorVisible, success }
}

test('Agent Skills reconcile opens one dialog with the cause body and keeps the dialog title', async () => {
  const success = await reconcile(null)
  assert.deepEqual(success.success, ['Reconcile complete'])
  assert.equal(success.operationErrorVisible.value, false)

  const failed = await reconcile({ message: 'journal locked' })
  assert.equal(failed.operationError.value, 'journal locked')
  assert.equal(failed.operationErrorVisible.value, true)
  assert.doesNotMatch(failed.operationError.value, /Operation failed|\[object Object\]/)
})

const listProviderModels = async ({ protocol = 'openai', listModels }) => {
  const notifications = []
  const warnings = []
  const errors = []
  let catalogLoads = 0
  const handler = loadHandler(
    read('../components/setting/Model.vue'),
    'fetchedProviderModelsFromServer',
    {
      isLoadingProviderModels: { value: false },
      fetchedProviderModels: { value: [] },
      modelStore: { listModels },
      loadCatalogProviderModels: async () => {
        catalogLoads++
      },
      normalizeUserError,
      USER_ERROR_REASONS,
      formatUserError,
      FrontendAppError,
      t,
      showMessage: (...args) => notifications.push(args),
      console: {
        warn: (...args) => warnings.push(args),
        error: (...args) => errors.push(args)
      }
    }
  )
  await handler(protocol, 'https://api.example/v1', 'key', {})
  return { notifications, warnings, errors, catalogLoads }
}

test('Model catalog list-models keeps the decision, unsupported and fallback order', async () => {
  const success = await listProviderModels({
    listModels: async () => [{ id: 'gpt-x' }]
  })
  assert.deepEqual(success.notifications, [])
  assert.equal(success.catalogLoads, 0)

  const decision = await listProviderModels({
    protocol: 'decision',
    listModels: async () => {
      throw Object.assign(new Error('boom'), { upstream_status: 405 })
    }
  })
  // The decision branch runs before the 405 skip branch.
  assert.deepEqual(decision.notifications, [
    ['Could not import decision models: boom.', 'error']
  ])
  assert.equal(decision.catalogLoads, 0)

  const unsupported = await listProviderModels({
    listModels: async () => {
      throw { reason: USER_ERROR_REASONS.UNSUPPORTED_LIST_MODELS, message: 'not available' }
    }
  })
  assert.deepEqual(unsupported.notifications, [])
  assert.equal(unsupported.catalogLoads, 1)
  assert.equal(unsupported.warnings.length, 1)

  const status405 = await listProviderModels({
    listModels: async () => {
      throw { upstream_status: 405, message: 'Method Not Allowed' }
    }
  })
  assert.deepEqual(status405.notifications, [])
  assert.equal(status405.catalogLoads, 1)

  const generic = await listProviderModels({
    listModels: async () => {
      throw new Error('network down')
    }
  })
  assert.deepEqual(generic.notifications, [])
  assert.equal(generic.catalogLoads, 1)
  assert.equal(generic.errors.length, 1)
  assert.doesNotMatch(generic.errors[0][0], /Ai:Upstream/)
})

const streamPayload = JSON.stringify({
  presentation: { schema_version: 1, message: 'Provider declined the request', reason: 'auth' },
  message: '[Ai:Upstream] Provider declined the request'
})

const streamError = async (file, extraContext = {}, projection = undefined) => {
  const chatErrorMessage = { value: '' }
  const handler = loadHandler(read(file), 'handleChatMessage', {
    chatState: { value: {} },
    chatErrorMessage,
    isChatting: { value: true },
    currentModel: { value: { defaultModel: 'model-x' } },
    currentModelProvider: { value: { defaultModel: 'model-x' } },
    payloadMetadata: { value: {} },
    handleChatMessageCommon: (_payload, _state, refs) => {
      // Mirrors parseErrorMsg keeping the raw prefix until the UI normalizes it.
      refs.chatErrorMessage.value = JSON.parse(streamPayload).message
    },
    formatUserError,
    t,
    nextTick: fn => fn(),
    scrollToBottomIfNeeded: () => {},
    userHasScrolled: { value: false },
    isScrolledToBottom: { value: true },
    ...extraContext
  })
  const payload = { type: 'error', chunk: streamPayload, metadata: {
    model: 'model-x', ...(projection && { user_error: projection })
  } }
  const before = JSON.stringify(payload)
  await handler(payload)
  assert.equal(JSON.stringify(payload), before, 'UI projection must not mutate the stream payload')
  return chatErrorMessage.value
}

test('Index and Assistant normalize only the streamed error copy shown in the UI', async () => {
  for (const file of ['Index.vue', 'Assistant.vue']) {
    const message = await streamError(file)
    assert.equal(message, 'Provider declined the request')
    assert.doesNotMatch(message, /Ai:Upstream|schema_version/)
    assert.equal(await streamError(file, {}, {
      schema_version: 1, message: 'typed metadata cause: https://provider.test\nretry', upstream_status: 503
    }), 'typed metadata cause: https://provider.test\nretry')
  }
})

const changedErrorSurfaces = [
  'Index.vue',
  'Assistant.vue',
  'Note.vue',
  'ProxySwitcher.vue',
  '../components/setting/Agent.vue',
  '../components/setting/ChatHub.vue',
  '../components/setting/Plugin.vue',
  '../components/setting/AgentSkills.vue',
  '../components/setting/Sandbox.vue',
  '../components/setting/Decision.vue',
  '../components/setting/Model.vue',
  '../components/setting/ProxyStats.vue',
  '../components/setting/ScraperTest.vue'
]

test('assigned error surfaces import the safe formatter and never print raw error forms', () => {
  for (const file of changedErrorSurfaces) {
    const source = read(file)
    assert.match(source, /import \{[^}]*formatUserError[^}]*\} from '@\/libs\/error'/, file)
    assert.doesNotMatch(source, /(?:showMessage|ElMessage\.(?:error|warning))\([^\n]*toFormattedString/, file)
    assert.doesNotMatch(source, /showMessage\(\s*(?:err|error|e)\.message/, file)
    assert.doesNotMatch(source, /error:\s*(?:error|err|e)\.message/, file)
    assert.doesNotMatch(source, /String\((?:error|err|e)\)/, file)
  }
  // The old ProxySwitcher helper was the last local formatter that printed raw causes.
  assert.doesNotMatch(read('ProxySwitcher.vue'), /const formatError =/)
})

const assignedFiles = [
  'Index.vue',
  'Assistant.vue',
  'Note.vue',
  'ProxySwitcher.vue',
  'AgentSkills.vue',
  'Settings.vue',
  '../components/workflow/StatusPanel.vue',
  '../components/setting/Agent.vue',
  '../components/setting/ChatHub.vue',
  '../components/setting/Plugin.vue',
  '../components/setting/AgentSkills.vue',
  '../components/setting/Sandbox.vue',
  '../components/setting/Decision.vue',
  '../components/setting/Model.vue',
  '../components/setting/ProxyStats.vue',
  '../components/setting/ScraperTest.vue'
]

test('every assigned page still parses and compiles as a Vue SFC', () => {
  for (const file of assignedFiles) {
    const source = read(file)
    const { descriptor, errors } = parseSfc(source, { filename: file })
    assert.deepEqual(errors, [], `${file} must parse`)
    assert.ok(descriptor.script || descriptor.scriptSetup, `${file} must have a script`)

    compileScript(descriptor, { id: `data-v-${file.replace(/\W/g, '-')}` })

    if (descriptor.template) {
      const template = compileTemplate({
        source: descriptor.template.content,
        filename: file,
        id: `data-v-${file.replace(/\W/g, '-')}`
      })
      assert.deepEqual(template.errors, [], `${file} template must compile`)
    }
  }
})