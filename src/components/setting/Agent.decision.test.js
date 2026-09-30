import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('decision configuration belongs to the decision settings, not the agent editor', async () => {
  const store = await readFile(new URL('../../stores/agent.js', import.meta.url), 'utf8')
  const component = await readFile(new URL('./Agent.vue', import.meta.url), 'utf8')
  const decisionSettings = await readFile(new URL('./Decision.vue', import.meta.url), 'utf8')

  // The decision model is configured once for all agents in the settings form.
  assert.match(decisionSettings, /setSetting\('decisionConfig'/)
  assert.match(decisionSettings, /settings\.sandbox\.decisionProvider/)

  // The agent editor must not own a per-agent decision model: an unset decision
  // draft used to be null there and broke the whole edit dialog render.
  assert.doesNotMatch(component, /decisionProviders/)
  assert.doesNotMatch(component, /decisionEnabled/)
  assert.doesNotMatch(component, /decisionModel/)
  assert.doesNotMatch(component, /key: 'decision'/)
  assert.doesNotMatch(store, /decisionEnabled/)
  assert.doesNotMatch(store, /decisionModel/)
  assert.doesNotMatch(store, /decision:/)
})
