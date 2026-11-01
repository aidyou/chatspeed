import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import test from 'node:test'
import * as sass from 'sass-embedded'
import { compileScript, compileStyleAsync, compileTemplate, parse } from 'vue/compiler-sfc'
import { DEFAULT_AGENT_RESOURCE_LIMITS, validateAgentResourceLimits } from '../../libs/agentResourceLimits.js'

const component = readFileSync(new URL('./AgentResources.vue', import.meta.url), 'utf8')
const management = readFileSync(new URL('./AgentManagement.vue', import.meta.url), 'utf8')
const agent = readFileSync(new URL('./Agent.vue', import.meta.url), 'utf8')
const require = createRequire(import.meta.url)

test('resource settings have a dedicated management tab outside agent settings', () => {
  assert.match(management, /import AgentResources from '\.\/AgentResources\.vue'/)
  assert.match(management, /<el-tab-pane :label="\$t\('settings\.agent\.resourceTab'\)" name="resources">\s*<agent-resources \/>\s*<\/el-tab-pane>/)
  assert.match(management, /const activeTab = ref\('agents'\)/)
  assert.doesNotMatch(agent, /agent-resource-settings|resourceLimits|saveResourceLimits/)
})

test('resource settings distinguish global and per-task budgets', () => {
  for (const field of ['totalMemoryMb', 'maxConcurrentTasks', 'taskMemoryMb', 'taskMaxProcesses']) {
    assert.match(component, new RegExp(`v-model="resourceLimits\\.${field}"`))
  }
  assert.match(component, /setSetting\('agentResourceLimits', \{ \.\.\.resourceLimits\.value \}\)/)
  assert.match(component, /validateAgentResourceLimits\(resourceLimits\.value\)/)
})

test('resource limits reject invalid integers and task budgets above the global budget', () => {
  assert.equal(validateAgentResourceLimits(DEFAULT_AGENT_RESOURCE_LIMITS), true)
  for (const invalid of [
    { taskMemoryMb: 8193 }, { taskMemoryMb: 0 }, { totalMemoryMb: undefined },
    { maxConcurrentTasks: 0 }, { taskMaxProcesses: 1.5 }, { enabled: 'true' }
  ]) {
    assert.equal(validateAgentResourceLimits({ ...DEFAULT_AGENT_RESOURCE_LIMITS, ...invalid }), false)
  }
  assert.equal(validateAgentResourceLimits(null), false)
})

test('resource settings and their parent compile as Vue components', async () => {
  for (const [filename, source] of [['AgentResources.vue', component], ['AgentManagement.vue', management], ['Agent.vue', agent]]) {
    const { descriptor, errors } = parse(source, { filename })
    assert.deepEqual(errors, [])
    const script = compileScript(descriptor, { id: 'agent-resources' })
    const template = compileTemplate({
      source: descriptor.template.content,
      filename,
      id: 'agent-resources',
      compilerOptions: { bindingMetadata: script.bindings }
    })
    assert.deepEqual(template.errors, [])
    for (const style of descriptor.styles) {
      const result = await compileStyleAsync({
        source: style.content,
        filename,
        id: 'agent-resources',
        preprocessLang: style.lang,
        preprocessCustomRequire: name => name === 'sass' ? sass : require(name)
      })
      assert.deepEqual(result.errors, [])
    }
  }
})

test('all resource setting translations exist and have consistent sorted keys', () => {
  let expected
  for (const locale of ['en', 'zh-Hans', 'zh-Hant']) {
    const messages = JSON.parse(readFileSync(new URL(`../../i18n/locales/${locale}.json`, import.meta.url), 'utf8'))
    const entries = messages.settings.agent
    const keys = Object.keys(entries).filter(key => key.startsWith('resource'))
    assert.deepEqual(keys, [...keys].sort())
    if (expected) assert.deepEqual(keys, expected)
    expected = keys
    for (const match of (component + management).matchAll(/settings\.agent\.(resource\w+)/g)) {
      assert.equal(typeof entries[match[1]], 'string', `${locale}: ${match[1]}`)
    }
    assert.ok(entries.resourceProtectionHint.includes('250'))
    assert.ok(entries.resourceProtectionHint.includes('MCP'))
  }
})
