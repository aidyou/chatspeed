import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import test from 'node:test'
import { formatUserError } from '../../libs/error.js'

const read = file => readFileSync(new URL(`./${file}`, import.meta.url), 'utf8')

const messages = {
  'settings.general.updateSettingFailed': 'Failed to update settings: {error}',
  'settings.proxyGroup.deleteFailed': 'Failed to delete group: {error}',
  'common.errorFallback': 'Something went wrong. Please try again.'
}

const t = (key, params = {}) =>
  (messages[key] ?? key).replace(/\{(\w+)\}/g, (placeholder, name) =>
    params[name] === undefined ? placeholder : params[name]
  )

class FrontendAppError extends Error {
  constructor(message, originalError) {
    super(message)
    this.name = 'FrontendAppError'
    this.kind = 'Upstream'
    this.originalError = originalError
  }

  toFormattedString() {
    return `[Ai:Upstream] ${this.message}`
  }
}

const extractHandler = (source, name) => {
  const handler = source.match(new RegExp(`const ${name} = [^\\n]+\\{[\\s\\S]*?\\n\\}`))?.[0]
  assert.ok(handler, `${name} should remain an extractable handler`)
  return handler
}

const flush = () => new Promise(resolve => setImmediate(resolve))

const runUpdateSetting = async error => {
  const notifications = []
  const logs = []
  const context = {
    FrontendAppError,
    formatUserError,
    t,
    showMessage: (message, type) => notifications.push([message, type]),
    console: { error: (...args) => logs.push(args) },
    settingStore: { setSetting: () => Promise.reject(error) }
  }
  const handler = extractHandler(read('General.vue'), 'setSetting')
  const setSetting = runInNewContext(`${handler}\nsetSetting`, context)
  setSetting('theme', 'dark')
  await flush()
  return { notifications, logs }
}

test('the extracted update-setting handler shows one title for FrontendAppError and Error causes', async () => {
  const originalError = new Error('raw transport detail')
  const frontend = await runUpdateSetting(new FrontendAppError('provider declined', originalError))
  assert.equal(frontend.notifications.length, 1)
  assert.deepEqual(frontend.notifications[0], ['Failed to update settings: provider declined', 'error'])
  assert.equal(frontend.notifications[0][0].match(/Failed to update settings/g).length, 1)
  assert.doesNotMatch(frontend.notifications[0][0], /Ai:Upstream|raw transport detail/)
  assert.equal(frontend.logs[0][1], originalError)

  const plain = await runUpdateSetting(new Error('network down'))
  assert.deepEqual(plain.notifications[0], ['Failed to update settings: network down', 'error'])
  assert.equal(plain.notifications[0][0].match(/Failed to update settings/g).length, 1)

  const unknown = await runUpdateSetting({})
  assert.deepEqual(unknown.notifications[0], [
    'Failed to update settings: Something went wrong. Please try again.',
    'error'
  ])
  assert.doesNotMatch(unknown.notifications[0][0], /\[object Object\]/)
})

test('the extracted proxy-group delete handler stays silent when the confirmation is cancelled', async () => {
  const notifications = []
  let removeCalls = 0
  const context = {
    FrontendAppError,
    formatUserError,
    t,
    showMessage: (...args) => notifications.push(args),
    ElMessageBox: { confirm: () => Promise.reject(new Error('cancel')) },
    proxyGroupStore: {
      remove: async () => {
        removeCalls++
      }
    }
  }
  const handler = extractHandler(read('ProxyGroup.vue'), 'handleDeleteGroup')
  const handleDeleteGroup = runInNewContext(`${handler}\nhandleDeleteGroup`, context)
  handleDeleteGroup(1)
  await flush()
  assert.equal(removeCalls, 0)
  assert.deepEqual(notifications, [])
})

test('active proxy-stats delete keeps cancellation silent and displays one safe cause', async () => {
  const handler = extractHandler(read('ProxyStats.vue'), 'handleDeleteStats')
  const exercise = async (cancel, error) => {
    const notifications = []
    const loading = { value: false }
    let deletes = 0
    const run = runInNewContext(`${handler}\nhandleDeleteStats`, {
      t, formatUserError, loading,
      ElMessageBox: { confirm: () => cancel ? Promise.reject('cancel') : Promise.resolve() },
      invokeWrapper: async () => { deletes++; throw error },
      showMessage: (...args) => notifications.push(args),
      fetchDailyStats: () => assert.fail('failed delete must not refresh'),
      console: { error() {} }
    })
    await run(7)
    assert.equal(loading.value, false)
    return { notifications, deletes }
  }
  assert.deepEqual(await exercise(true), { notifications: [], deletes: 0 })
  const failure = { message: 'legacy CANARY', presentation: { schema_version: 1, message: 'provider busy' } }
  assert.deepEqual(await exercise(false, failure), { notifications: [['provider busy', 'error']], deletes: 1 })
  assert.match(read('../../views/ProxySwitcher.vue'), /import ProxyStats from/)
})

test('every assigned page imports the safe formatter instead of printing raw error forms', () => {
  for (const file of [
    'General.vue',
    'ProxyGroup.vue',
    'Proxy.vue',
    'ProxyStats.vue',
    'ScraperTest.vue',
    'Skill.vue',
    'Mcp.vue',
    'McpExecuteDialog.vue'
  ]) {
    const source = read(file)
    assert.match(source, /import \{ formatUserError \} from '@\/libs\/error'/, file)
    assert.doesNotMatch(source, /showMessage\(\s*(?:err|error|e)\.toFormattedString/, file)
    assert.doesNotMatch(source, /ElMessage\.error\(\s*(?:err|error|e)\.toFormattedString/, file)
    assert.doesNotMatch(source, /errorText\.value\s*=\s*e instanceof FrontendAppError/, file)
    assert.doesNotMatch(source, /error\.toString\(\),\s*'error'\)/, file)
  }
})