import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const source = readFileSync(new URL('./General.vue', import.meta.url), 'utf8')

test('keeps complete backup handlers and gates configuration import behind risk confirmation', () => {
  assert.match(source, /invokeWrapper\('backup_setting'/)
  assert.match(source, /invokeWrapper\('restore_setting'/)
  assert.match(source, /configImportRiskConfirmed = ref\(false\)/)
  assert.match(source, /:disabled="!configImportRiskConfirmed \|\| configImportCategories\.length === 0"/)
  assert.match(source, /if \(!configImportRiskConfirmed\.value \|\| !configImportPath\.value/)
  assert.match(source, /invokeWrapper\('inspect_config_package'/)
  assert.match(source, /invokeWrapper\('import_config_package'/)
})

test('applies frontend category dependency closure and defaults export to every category', () => {
  assert.match(source, /const configCategories = \['aiModels', 'skills', 'mcp', 'proxy', 'agents', 'sandbox'\]/)
  assert.match(source, /configExportCategories\.value = \[\.\.\.configCategories\]/)
  assert.match(source, /if \(normalized\.has\('proxy'\)\) normalized\.add\('aiModels'\)/)
  assert.match(source, /if \(normalized\.has\('agents'\)\) \['aiModels', 'skills', 'mcp', 'sandbox'\]/)
  assert.match(source, /watch\(configExportCategories, categories => \{\n  const normalized = normalizeConfigCategories\(categories\)\n  if \(normalized\.length !== categories\.length\) configExportCategories\.value = normalized/)
  assert.match(source, /watch\(configImportCategories, categories =>/)
  assert.match(source, /configImportPreview\.counts\?\.\[category\] \|\| 0/)
  assert.match(source, /configImportResult\.value = result/)
  assert.match(source, /preservedAgents/)
  assert.match(source, /apiKeysLocked/)
  assert.match(source, /configExportBusy = ref\(false\)/)
  assert.match(source, /:loading="configExportBusy"/)
  assert.match(source, /try \{\n    const path = await save\(/)
  assert.match(source, /console\.error\('Failed to export configuration:', error\)/)
  assert.match(source, /showMessage\(error instanceof FrontendAppError \? error\.toFormattedString\(\) : error\.toString\(\), 'error'\)/)
})
