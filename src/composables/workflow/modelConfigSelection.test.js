import assert from 'node:assert/strict'
import test from 'node:test'

import { createLatestModelConfigSaver } from './modelConfigPersistence.js'
import {
  buildCurrentModelOptions,
  formatActiveModelName,
  getModelConfigForOption,
  resolveActiveModelConfig,
  resolveDisplayModelConfig,
  thinkingLevelFromBudget
} from './modelConfigSelection.js'

const providers = [
  {
    id: 1,
    name: 'Provider A',
    models: [
      { id: 'a-fast', name: 'A Fast' },
      { id: 'a-reasoning', name: 'A Reasoning', reasoning: true }
    ]
  },
  {
    id: 2,
    name: 'Provider B',
    models: [{ id: 'b-fast', name: 'B Fast' }]
  }
]
const modelStore = {
  getModelProviderById(id) {
    return providers.find(provider => provider.id === id) || null
  }
}
const settings = {
  chatCompletionProxy: {
    team: {
      alpha: [{ id: 1, model: 'a-reasoning' }],
      beta: [{ id: 2, model: 'b-fast' }]
    },
    other: {
      hidden: [{ id: 1, model: 'a-fast' }]
    }
  }
}

test('model dropdown limits options to the current provider or proxy group', () => {
  const providerOptions = buildCurrentModelOptions({
    currentConfig: { id: 1, model: 'a-fast', thinking: { type: 'disabled' } },
    modelStore,
    settings
  })
  assert.deepEqual(providerOptions.map(group => group.label), ['Provider A'])
  assert.deepEqual(providerOptions[0].models.map(model => model.model), ['a-fast', 'a-reasoning'])

  const proxyOptions = buildCurrentModelOptions({
    currentConfig: { id: 0, model: 'team@alpha', thinking: { type: 'enabled', budgetTokens: 4096 } },
    modelStore,
    settings
  })
  assert.deepEqual(proxyOptions.map(group => group.label), ['team'])
  assert.deepEqual(proxyOptions[0].models.map(model => model.name), ['alpha', 'beta'])
  assert.equal(proxyOptions[0].models.some(model => model.name === 'hidden'), false)
  assert.equal(proxyOptions[0].models[0].supportsThinking, true)
})

test('plan mode falls back to act while persisting the next choice in the plan slot', () => {
  const models = {
    plan: { id: '', model: '' },
    act: { id: 1, model: 'a-fast', thinking: { type: 'disabled' } }
  }
  const active = resolveActiveModelConfig(models, true)
  assert.equal(active.model, 'a-fast')
  assert.equal(resolveDisplayModelConfig(models, true).model, 'a-fast')
  assert.equal(
    formatActiveModelName({ models, planningMode: true, modelStore }),
    'Plan/A Fast'
  )

  const planConfig = getModelConfigForOption(
    { id: 1, model: 'a-reasoning', targetModel: providers[0].models[1] },
    active
  )
  const nextModels = { ...models, plan: planConfig }
  assert.equal(nextModels.plan.model, 'a-reasoning')
  assert.equal(nextModels.act.model, 'a-fast')
  assert.equal(planConfig.thinking.type, 'disabled')
  assert.equal(thinkingLevelFromBudget(8192), 'max')
})

test('latest model config save keeps a budget update after a model selection', async () => {
  const saves = []
  let resolveFirstSave
  const firstSave = new Promise(resolve => {
    resolveFirstSave = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async config => {
      saves.push(config)
      if (saves.length === 1) await firstSave
      return true
    }
  })

  const modelOnly = { act: { model: 'a-reasoning', thinking: { type: 'disabled' } } }
  const withBudget = {
    act: { model: 'a-reasoning', thinking: { type: 'enabled', budgetTokens: 8192 } }
  }

  const pending = saver.submit({ scope: 'workflow:A', configs: modelOnly })
  await Promise.resolve()
  await saver.submit({ scope: 'workflow:A', configs: withBudget })
  resolveFirstSave()
  await pending

  assert.deepEqual(saves.map(submission => submission.configs), [modelOnly, withBudget])
})

test('latest model config save keeps only the newest rapid selection', async () => {
  const saves = []
  let resolveFirstSave
  const firstSave = new Promise(resolve => {
    resolveFirstSave = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async config => {
      saves.push(config)
      if (saves.length === 1) await firstSave
      return true
    }
  })

  const modelOnly = { act: { model: 'a-reasoning', thinking: { type: 'disabled' } } }
  const withBudget = {
    act: { model: 'a-reasoning', thinking: { type: 'enabled', budgetTokens: 8192 } }
  }
  const finalSelection = { act: { model: 'a-fast', thinking: { type: 'disabled' } } }

  const pending = saver.submit({ scope: 'workflow:A', configs: modelOnly })
  await Promise.resolve()
  await saver.submit({ scope: 'workflow:A', configs: withBudget })
  await saver.submit({ scope: 'workflow:A', configs: finalSelection })
  resolveFirstSave()
  await pending

  assert.deepEqual(saves.map(submission => submission.configs), [modelOnly, finalSelection])
})

test('workflow scope change discards an old queued snapshot without affecting the new scope', async () => {
  const saves = []
  let resolveWorkflowA
  const workflowA = new Promise(resolve => {
    resolveWorkflowA = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async submission => {
      saves.push(submission)
      if (submission.scope === 'workflow:A') await workflowA
      return true
    }
  })

  const pending = saver.submit({
    scope: 'workflow:A',
    target: { type: 'workflow', sessionId: 'A' },
    configs: { act: { model: 'a-fast' } }
  })
  await Promise.resolve()
  await saver.submit({
    scope: 'workflow:A',
    target: { type: 'workflow', sessionId: 'A' },
    configs: { act: { model: 'a-reasoning' } }
  })
  saver.invalidateScope('workflow:B')
  await saver.submit({
    scope: 'workflow:B',
    target: { type: 'workflow', sessionId: 'B' },
    configs: { act: { model: 'b-fast' } }
  })
  resolveWorkflowA()
  await pending

  assert.deepEqual(
    saves.map(submission => [submission.target.sessionId, submission.configs.act.model]),
    [
      ['A', 'a-fast'],
      ['B', 'b-fast']
    ]
  )
})

test('agent scope change ignores an old failure and preserves the new update', async () => {
  const saves = []
  const failures = []
  let resolveAgentA
  const agentA = new Promise(resolve => {
    resolveAgentA = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async submission => {
      saves.push(submission)
      if (submission.scope === 'agent:A') {
        await agentA
        return false
      }
      return true
    },
    onFailure: scope => failures.push(scope)
  })

  const pending = saver.submit({
    scope: 'agent:A',
    target: { type: 'agent', agentId: 'A' },
    configs: { act: { model: 'a-fast' } }
  })
  await Promise.resolve()
  await saver.submit({
    scope: 'agent:A',
    target: { type: 'agent', agentId: 'A' },
    configs: { act: { model: 'a-reasoning' } }
  })
  saver.invalidateScope('agent:B')
  await saver.submit({
    scope: 'agent:B',
    target: { type: 'agent', agentId: 'B' },
    configs: { act: { model: 'b-fast' } }
  })
  resolveAgentA()
  await pending

  assert.deepEqual(
    saves.map(submission => [submission.target.agentId, submission.configs.act.model]),
    [
      ['A', 'a-fast'],
      ['B', 'b-fast']
    ]
  )
  assert.deepEqual(failures, [])
})

test('workflow scope re-entry preserves a later A update after the earlier A save fails', async () => {
  const saves = []
  const failures = []
  let resolveFirstWorkflowA
  const firstWorkflowA = new Promise(resolve => {
    resolveFirstWorkflowA = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async submission => {
      saves.push(submission)
      if (submission.configs.act.model === 'a-first') {
        await firstWorkflowA
        return false
      }
      return true
    },
    onFailure: submission => failures.push(submission)
  })

  const firstActivation = saver.invalidateScope('workflow:A')
  const pending = saver.submit({
    scope: 'workflow:A',
    activation: firstActivation,
    target: { type: 'workflow', sessionId: 'A' },
    configs: { act: { model: 'a-first' } }
  })
  await Promise.resolve()
  saver.invalidateScope('workflow:B')
  const returnedActivation = saver.invalidateScope('workflow:A')
  await saver.submit({
    scope: 'workflow:A',
    activation: returnedActivation,
    target: { type: 'workflow', sessionId: 'A' },
    configs: { act: { model: 'a-returned' } }
  })
  resolveFirstWorkflowA()
  await pending

  assert.deepEqual(
    saves.map(submission => [submission.target.sessionId, submission.configs.act.model]),
    [
      ['A', 'a-first'],
      ['A', 'a-returned']
    ]
  )
  assert.deepEqual(failures, [])
})

test('agent scope re-entry preserves a later A update after the earlier A save fails', async () => {
  const saves = []
  const failures = []
  let resolveFirstAgentA
  const firstAgentA = new Promise(resolve => {
    resolveFirstAgentA = resolve
  })
  const saver = createLatestModelConfigSaver({
    save: async submission => {
      saves.push(submission)
      if (submission.configs.act.model === 'a-first') {
        await firstAgentA
        return false
      }
      return true
    },
    onFailure: submission => failures.push(submission)
  })

  const firstActivation = saver.invalidateScope('agent:A')
  const pending = saver.submit({
    scope: 'agent:A',
    activation: firstActivation,
    target: { type: 'agent', agentId: 'A' },
    configs: { act: { model: 'a-first' } }
  })
  await Promise.resolve()
  saver.invalidateScope('agent:B')
  const returnedActivation = saver.invalidateScope('agent:A')
  await saver.submit({
    scope: 'agent:A',
    activation: returnedActivation,
    target: { type: 'agent', agentId: 'A' },
    configs: { act: { model: 'a-returned' } }
  })
  resolveFirstAgentA()
  await pending

  assert.deepEqual(
    saves.map(submission => [submission.target.agentId, submission.configs.act.model]),
    [
      ['A', 'a-first'],
      ['A', 'a-returned']
    ]
  )
  assert.deepEqual(failures, [])
})

for (const target of [
  { type: 'workflow', field: 'sessionId' },
  { type: 'agent', field: 'agentId' }
]) {
  test(`${target.type} save failure preserves a newer queued model update`, async () => {
    const saves = []
    const failures = []
    let resolveFirstSave
    const firstSave = new Promise(resolve => {
      resolveFirstSave = resolve
    })
    const saver = createLatestModelConfigSaver({
      save: async submission => {
        saves.push(submission)
        if (submission.configs.act.model === 'first') {
          await firstSave
          return false
        }
        return true
      },
      onFailure: submission => failures.push(submission)
    })

    const scope = `${target.type}:A`
    const activation = saver.invalidateScope(scope)
    const pending = saver.submit({
      scope,
      activation,
      target: { type: target.type, [target.field]: 'A' },
      configs: { act: { model: 'first' } }
    })
    await Promise.resolve()
    await saver.submit({
      scope,
      activation,
      target: { type: target.type, [target.field]: 'A' },
      configs: { act: { model: 'newest' } }
    })
    resolveFirstSave()
    await pending

    assert.deepEqual(
      saves.map(submission => [submission.target[target.field], submission.configs.act.model]),
      [
        ['A', 'first'],
        ['A', 'newest']
      ]
    )
    assert.deepEqual(failures, [])
  })
}

test('failed model config save discards pending updates and invokes rollback', async () => {
  const saved = []
  let rolledBack = 0
  const saver = createLatestModelConfigSaver({
    save: async config => {
      saved.push(config)
      return false
    },
    onFailure: () => {
      rolledBack += 1
    }
  })

  await saver.submit({ scope: 'workflow:A', configs: { act: { model: 'a-reasoning' } } })
  assert.equal(saved.length, 1)
  assert.equal(rolledBack, 1)
})
