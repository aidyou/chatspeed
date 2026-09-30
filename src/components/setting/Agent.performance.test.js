import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const readAgentComponent = () => readFile(new URL('./Agent.vue', import.meta.url), 'utf8')
const readSandboxComponent = () => readFile(new URL('./Sandbox.vue', import.meta.url), 'utf8')

test('agent model settings expose and preserve the maximum reasoning level', async () => {
  const source = await readAgentComponent()

  assert.match(source, /max: 8192/)
  assert.match(source, /if \(normalized > 4096\) return 'max'/)
  assert.match(
    source,
    /\{ value: 'max', label: 'settings\.model\.reasoningMax' \}/
  )
})

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

test('sandbox profiles use a bounded compact list and a dedicated editor', async () => {
  // Profile management moved from the agent dialog into the shared sandbox settings.
  const agentSource = await readAgentComponent()
  assert.doesNotMatch(agentSource, /SANDBOX_PROFILE_PAGE_SIZE|sandboxProfileEditorVisible/)

  const sandboxSource = await readSandboxComponent()
  // Profile and host-rule rows render inside bounded tables, not an unbounded card list.
  assert.match(sandboxSource, /<el-table[\s\S]*?max-height="320"/)
  // Editing one profile owns a dedicated dialog with explicit open/save handlers.
  assert.match(sandboxSource, /<el-dialog v-model="profileDialogVisible"/)
  assert.match(sandboxSource, /const openProfileEditor = async profile =>/)
  assert.match(sandboxSource, /const saveProfile = \(\) =>/)
  assert.doesNotMatch(sandboxSource, /<el-card v-for="profile in sandboxProfiles/)
})
