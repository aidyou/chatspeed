import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { runInNewContext } from 'node:vm'
import test from 'node:test'

const source = () => readFile(new URL('./Model.vue', import.meta.url), 'utf8')

test('decision editor preserves import, hides chat fields and retains prices', async () => {
  const component = await source()
  assert.match(component, /Decision: 'decision'/)
  assert.match(component, /v-if="modelForm\.apiProtocol !== 'decision'" :label="\$t\('settings\.model\.supportsResponsesApi'\)"/)
  assert.equal((component.match(/<el-tab-pane v-if="modelForm\.apiProtocol !== 'decision'" :label="\$t\('settings\.model\.additionalInfo'\)"/g) || []).length, 2)
  for (const key of ['reasoning', 'functionCall', 'imageInput', 'maxTokens', 'temperature']) {
    assert.match(component, new RegExp(`v-if="modelForm.apiProtocol !== 'decision'" :label="\\$t\\('settings.model.${key}'\\)"`))
  }
  // The evaluation endpoint's usable input is far below the chat default, so the decision editor
  // keeps the context size visible instead of hiding it and imports the endpoint's own value.
  assert.doesNotMatch(component, /v-if="modelForm\.apiProtocol !== 'decision'" :label="\$t\('settings\.model\.contextSize'\)"/)
  assert.match(component, /\{\{ \$t\('settings\.model\.decisionContextSizeHint'\) \}\}/)
  assert.match(component, /const DECISION_CONTEXT_SIZE = 8192/)
  assert.match(component, /contextSize: DECISION_CONTEXT_SIZE/)
  for (const field of ['inputPerMillion', 'outputPerMillion', 'multiplier']) {
    assert.match(component, new RegExp(`v-model="modelConfigForm.pricing.${field}"`))
  }
  assert.match(component, /modelForm\.apiProtocol !== 'decision' && Object\.keys\(providerModelToShow\)/)
  assert.match(component, /\.\.\.\(editId\.value \? modelStore\.getModelProviderById\(editId\.value\)\?\.metadata \|\| \{\} : \{\}\)/)
  assert.match(component, /const updatedDecisionModel = \{\s*\.\.\.currentModel,/)
  assert.match(component, /if \(modelForm\.value\.apiProtocol === 'decision'\) return\s*const modelId = modelConfigForm/)
  assert.match(component, /if \(protocol === 'decision'\) \{\s*showMessage\(t\('settings\.model\.decisionImportFailed'/)
})

test('decision editor keeps the context size it shows', async () => {
  const component = await source()
  // The decision save path rebuilds the model entry from a fixed set of fields, so a context size
  // left out of it would silently disappear and the editor would show the chat default on reopen.
  assert.match(component, /contextSize: decisionContextSize\(modelConfigForm\.value\.contextSize\)/)
  // A newly added decision model starts from the chat default, which the endpoint refuses.
  assert.match(component, /if \(modelForm\.value\.apiProtocol === 'decision'\) \{\s*modelConfigForm\.value\.contextSize = DECISION_CONTEXT_SIZE/)
  const declaration = component.match(/const decisionContextSize = [\s\S]*?\n\}/)?.[0]
  assert.ok(declaration)
  const resolve = runInNewContext(`${declaration}; decisionContextSize`, { DECISION_CONTEXT_SIZE: 8192 })
  assert.equal(resolve(4096), 4096)
  assert.equal(resolve(128000), 128000)
  for (const missing of [undefined, null, 0, -1, '', 'nope', Number.NaN]) {
    assert.equal(resolve(missing), 8192)
  }
})

test('ordinary model provider selection excludes decision providers', async () => {
  const store = await readFile(new URL('../../stores/model.js', import.meta.url), 'utf8')
  assert.match(store, /getAvailableProviders = computed\(\(\) => providers\.value\.filter\(m => !m\.disabled && m\.apiProtocol !== 'decision'\)\)/)
  assert.match(store, /currentDefaultModel && !currentDefaultModel\.disabled && currentDefaultModel\.apiProtocol !== 'decision'/)
})

test('Assistant restores and refreshes only usable chat providers, preserving valid selections', async () => {
  const assistant = await readFile(new URL('../../views/Assistant.vue', import.meta.url), 'utf8')
  const selector = assistant.match(/const selectAssistantChatProvider = ([\s\S]*?)\n\nconst currentModelProvider = ref\(/)?.[1]
  assert.ok(selector)
  const select = runInNewContext(`(${selector})`, {})
  const decision = { id: 1, apiProtocol: 'decision', models: [{ id: 'Kev-4b' }], defaultModel: 'Kev-4b' }
  const disabled = { id: 2, disabled: true, apiProtocol: 'openai', models: [{ id: 'old' }], defaultModel: 'old' }
  const ordinary = { id: 3, apiProtocol: 'openai', models: [{ id: 'chat-a' }, { id: 'chat-b' }], defaultModel: 'chat-a' }
  const alternate = { id: 4, apiProtocol: 'gemini', models: [{ id: 'gemini-a' }], defaultModel: 'gemini-a' }
  const providers = [decision, disabled, ordinary, alternate]
  assert.equal(select(providers, decision.id, decision).id, ordinary.id)
  assert.equal(select(providers, disabled.id, decision).id, ordinary.id)
  assert.equal(select(providers, ordinary.id, decision).defaultModel, 'chat-a')
  assert.equal(select(providers, alternate.id, ordinary).id, alternate.id)
  assert.equal(select(providers, ordinary.id, ordinary, 'chat-b').defaultModel, 'chat-b')
  assert.equal(select(providers, ordinary.id, ordinary, 'missing').defaultModel, 'chat-a')
  assert.equal(select([decision, disabled], decision.id, decision).id, undefined)
  assert.match(assistant, /restoreAssistantProvider\(csGetStorage\(csStorageKey\.defaultModelIdAtDialog\)\)/)
  assert.match(assistant, /\(\) => modelStore\.providers,[\s\S]*?restoreAssistantProvider\(currentId\)/)
  assert.match(assistant, /const canSendMessage = computed\([\s\S]*?modelStore\.getAvailableProviders\.some\(provider =>[\s\S]*?provider\.id === currentModelProvider\.value\?\.id/)
  assert.match(assistant, /providerId: currentModelProvider\.value\.id,\s*model: currentModelProvider\.value\.defaultModel/)
})

test('ordinary settings and title/vision execution reject decision models', async () => {
  const general = await readFile(new URL('./General.vue', import.meta.url), 'utf8')
  assert.equal((general.match(/v-for="model in modelStore\.getAvailableProviders"/g) || []).length, 3)
  assert.match(general, /const chatProviderModels = providerId => modelStore\.getAvailableProviders\.find/)
  assert.doesNotMatch(general, /getModelProviderById\(settingStore\.settings\.(?:visionModel|conversationTitleGenModel)\.id\)/)

  const index = await readFile(new URL('../../views/Index.vue', import.meta.url), 'utf8')
  assert.match(index, /const titleProvider = modelStore\.getAvailableProviders\.find/)
  assert.match(index, /if \(titleProvider\?\.models\?\.some/)
  assert.match(index, /modelStore\.getAvailableProviders\.find\(provider => provider\.id === visionModel\.id\)/)
  assert.match(index, /modelStore\.getAvailableProviders\.some\(provider =>\s*provider\.id === visionModel\.id/)
  const assistant = await readFile(new URL('../../views/Assistant.vue', import.meta.url), 'utf8')
  assert.match(assistant, /modelStore\.getAvailableProviders\.some\(provider =>\s*provider\.id === visionModel\.id/)
  const workflow = await readFile(new URL('../../views/Workflow.vue', import.meta.url), 'utf8')
  assert.match(workflow, /function normalizeVisionModel\(model\) \{[\s\S]*?modelStore\.getAvailableProviders\.some/)

  const decision = await readFile(new URL('../../../src-tauri/src/workflow/react/decision.rs', import.meta.url), 'utf8')
  assert.match(decision, /provider\.api_protocol != "decision"/)
  const chat = await readFile(new URL('../../../src-tauri/src/ai/interaction/chat_completion.rs', import.meta.url), 'utf8')
  assert.match(chat, /validate_chat_provider\(chat_state_arc\.main_store\.as_ref\(\), provider_id\)\?/)
})
