import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('agent model configuration orders providers, models, and proxy options alphabetically', async () => {
  const component = await readFile(new URL('./Agent.vue', import.meta.url), 'utf8')

  // Provider and proxy group options render the sorted computeds, never the raw store arrays.
  assert.match(component, /v-for="provider in sortedModelProviders"/)
  assert.match(component, /v-for="group in sortedProxyGroups"/)
  assert.doesNotMatch(component, /v-for="provider in modelStore\.getAvailableProviders"/)
  assert.doesNotMatch(component, /v-for="group in proxyGroupStore\.list"/)

  // Both computed lists sort copies by name, so the stores keep their own order.
  assert.match(
    component,
    /const sortedModelProviders = computed\(\(\) =>\s*\[\.\.\.modelStore\.getAvailableProviders\]\.sort/
  )
  assert.match(
    component,
    /const sortedProxyGroups = computed\(\(\) =>\s*\[\.\.\.proxyGroupStore\.list\]\.sort/
  )

  // The provider's model list and the proxy alias list are sorted alphabetically too.
  assert.match(component, /return \[\.\.\.\(provider\?\.models \|\| \[\]\)\]\.sort/)
  assert.match(component, /Object\.keys\(groupData\)\.sort\(compareOptionLabels\)/)
})