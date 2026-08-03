import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const readAgentComponent = () => readFile(new URL('./Agent.vue', import.meta.url), 'utf8')

test('agent shell policy editor defers and bounds expensive control rendering', async () => {
  const source = await readAgentComponent()

  assert.match(source, /<el-dialog[\s\S]*?destroy-on-close[\s\S]*?>/)
  assert.match(
    source,
    /<el-tab-pane\s+:label="\$t\('settings\.agent\.security'\)"\s+name="security"\s+lazy>/
  )
  assert.match(source, /const SHELL_POLICY_PAGE_SIZE = 50/)
  assert.match(source, /v-for="entry in paginatedShellPolicies"/)
  assert.doesNotMatch(source, /v-for="\(rule, index\) in agentForm\.shellPolicy"/)
})
