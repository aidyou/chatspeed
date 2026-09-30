import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const readAgentComponent = () => readFile(new URL('./Agent.vue', import.meta.url), 'utf8')

test('shell execution is configured by the security switch for every agent role', async () => {
  const source = await readAgentComponent()

  // The switch is not gated by role, so child agents enable shell there as well.
  assert.match(
    source,
    /<div class="security-switch-row">\s*<span class="security-switch-label">/
  )
  assert.doesNotMatch(source, /AGENT_ROLE\.CHILD" class="security-switch-row"/)

  // The shell tool stays out of the selectable tool list instead of being a tool pick.
  assert.match(source, /const HIDDEN_AGENT_TOOL_IDS = \['bash'\]/)

  // Saving maps the switch onto the persisted shell tool for both roles.
  assert.equal(
    (source.match(/\[\.\.\.new Set\(\[\.\.\.normalized\.availableTools, 'bash'\]\)\]/g) || [])
      .length,
    2,
    'primary and child agents must both merge the shell tool when the switch is on'
  )

  // Turning the switch on or off must work for children too.
  const allowShellWatcher = source.slice(
    source.indexOf('() => agentForm.value.allowShell,')
  )
  assert.doesNotMatch(allowShellWatcher.slice(0, 300), /AGENT_ROLE\.CHILD/)
})

test('child agents own their shell rules instead of losing them on save', async () => {
  const source = await readAgentComponent()

  // The shell rule editor is gated by the switch alone, so children reach it as well.
  assert.match(
    source,
    /const canConfigureShellPolicy = computed\(\(\) => agentForm\.value\.allowShell\)/
  )
  assert.doesNotMatch(source, /normalized\.shellPolicy = \[\]/)

  // The backend must keep a child's shell rules for the workflow to merge them.
  const rustSource = await readFile(
    new URL('../../../src-tauri/src/commands/agent.rs', import.meta.url),
    'utf8'
  )
  assert.doesNotMatch(rustSource, /agent\.shell_policy = Some\("\[\]"\.to_string\(\)\);/)
})

test('child agents inherit authorized paths instead of configuring them', async () => {
  const source = await readAgentComponent()

  // Only a primary agent owns the authorized path list; a child inherits the parent session roots.
  assert.match(source, /const canConfigureAuthorizedPaths = computed\(\(\) => agentForm\.value\.role !== AGENT_ROLE\.CHILD\)/)
  assert.match(
    source,
    /<template v-if="canConfigureAuthorizedPaths">[\s\S]*?authorizedPathsAdd[\s\S]*?<\/template>/
  )
  // Children get an inheritance hint instead, and the shell switch stays outside the guard.
  assert.match(
    source,
    /<p v-else class="security-tip">\{\{ \$t\('settings\.agent\.authorizedPathsInherited'\) \}\}<\/p>/
  )
  assert.match(source, /<\/p>\s*<div class="security-switch-row">/)
  // Saving a child still clears the list, so it can never persist its own roots.
  assert.match(source, /normalized\.allowedPaths = \[\]/)
})
