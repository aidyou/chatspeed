import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('decision settings are global and are not mapped into per-agent model configuration', async () => {
  const store = await readFile(new URL('../../stores/agent.js', import.meta.url), 'utf8')
  const component = await readFile(new URL('./Agent.vue', import.meta.url), 'utf8')
  const management = await readFile(new URL('./AgentManagement.vue', import.meta.url), 'utf8')
  const decision = await readFile(new URL('./Decision.vue', import.meta.url), 'utf8')
  assert.match(store, /Decision model selection is global/)
  assert.match(store, /decisionEnabled: false/)
  assert.match(store, /decision: null/)
  assert.doesNotMatch(component, /decisionModelRequired/)
  assert.match(management, /name="decision"/)
  assert.match(management, /<decision \/>/)
  assert.match(decision, /setSetting\('decisionConfig'/)
  assert.match(decision, /settings\.sandbox\.decisionTitle/)
})
