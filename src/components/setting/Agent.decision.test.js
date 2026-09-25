import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('decision settings are stored in the per-agent model configuration', async () => {
  const store = await readFile(new URL('../../stores/agent.js', import.meta.url), 'utf8')
  const component = await readFile(new URL('./Agent.vue', import.meta.url), 'utf8')
  assert.match(store, /decisionEnabled: frontendAgent\.decisionEnabled === true/)
  assert.match(store, /decision: frontendAgent\.decisionEnabled === true/)
  assert.match(component, /decisionProviders/)
  assert.match(component, /role\.key === 'decision'/)
})
