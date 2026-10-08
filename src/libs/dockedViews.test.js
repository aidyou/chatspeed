import assert from 'node:assert/strict'
import test from 'node:test'

import { createDockedViewsCoordinator } from './dockedViews.js'

/** Geometry the dock measures for one snapshot. */
const geometry = (x = 900, width = 600, height = 700) => ({
  width,
  topInset: 40,
  cornerRadius: 8,
  bounds: { x, y: 40, width, height }
})

const chatTab = (tabId, url = `https://${tabId}.example/`) => ({ kind: 'chatHub', tabId, url })
const pluginTab = (tabId, entry = `${tabId}/index.html`) => ({
  kind: 'plugin',
  tabId,
  pluginId: `demo-${tabId}`,
  entry
})
const trustedTab = tabId => ({ kind: 'trusted', tabId, pluginId: 'agent-skills' })

const snapshot = ({ tabs = [], activeTabId = '', geometry: geo = geometry() } = {}) => ({
  tabs,
  activeTabId,
  geometry: geo
})

const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

/**
 * Harness whose provider commands resolve after configurable delays, so the tests can force
 * the completion orders that used to leave two native views stacked or a hidden view on
 * screen. Both providers record into one log and share one in-flight counter, which is how
 * "one serial coordinator" is checked.
 */
const createHarness = ({ delays = {} } = {}) => {
  const log = []
  const errors = []
  let inFlight = 0
  let maxInFlight = 0
  const native = {
    chatHub: { visible: false, tabId: '', url: '', width: 0, bounds: null },
    plugin: { visible: false, tabId: '', entry: '' }
  }

  const delayFor = kind => {
    const configured = delays[kind]
    return typeof configured === 'function' ? configured() : (configured ?? 0)
  }

  const run = async (kind, argument, apply) => {
    inFlight += 1
    maxInFlight = Math.max(maxInFlight, inFlight)
    log.push(argument === undefined ? kind : `${kind}:${argument}`)
    await delay(delayFor(kind))
    apply()
    inFlight -= 1
  }

  const chatHub = {
    show: (tabId, payload) =>
      run('ch.show', tabId, () => {
        native.chatHub.visible = true
        native.chatHub.tabId = tabId
        native.chatHub.url = payload.url
        native.chatHub.width = payload.width
        native.chatHub.bounds = payload.bounds
      }),
    hide: () => run('ch.hide', undefined, () => {
      native.chatHub.visible = false
    }),
    destroy: tabId =>
      run('ch.destroy', tabId, () => {
        if (native.chatHub.tabId === tabId) {
          native.chatHub.visible = false
          native.chatHub.tabId = ''
        }
      }),
    destroyAll: () =>
      run('ch.destroyAll', undefined, () => {
        native.chatHub.visible = false
        native.chatHub.tabId = ''
      }),
    reload: tabId => run('ch.reload', tabId, () => {})
  }

  const plugin = {
    show: (tabId, payload) =>
      run('pl.show', tabId, () => {
        native.plugin.visible = true
        native.plugin.tabId = tabId
        native.plugin.entry = payload.entry
      }),
    hide: () => run('pl.hide', undefined, () => {
      native.plugin.visible = false
    }),
    close: tabId =>
      run('pl.close', tabId, () => {
        if (native.plugin.tabId === tabId) {
          native.plugin.visible = false
          native.plugin.tabId = ''
        }
      }),
    clear: () => run('pl.clear', undefined, () => {
      native.plugin.visible = false
      native.plugin.tabId = ''
    })
  }

  const coordinator = createDockedViewsCoordinator({
    chatHub,
    plugin,
    onError: (error, action) => errors.push({ error, action })
  })

  return {
    coordinator,
    log,
    errors,
    native: () => JSON.parse(JSON.stringify(native)),
    maxInFlight: () => maxInFlight
  }
}

test('a slow hide of the inactive provider is awaited before the other provider is shown', async () => {
  const harness = createHarness({ delays: { 'ch.hide': 30, 'pl.show': 1 } })

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), pluginTab('p')], activeTabId: 'p' })
  )
  await harness.coordinator.settled()

  assert.deepEqual(harness.log, ['ch.show:a', 'ch.hide', 'pl.show:p'])
  assert.equal(harness.native().plugin.visible, true, 'the plugin panel must be shown')
  assert.equal(harness.native().chatHub.visible, false, 'the chat page must be hidden first')
  assert.equal(harness.maxInFlight(), 1, 'provider commands must never overlap')
})

test('a superseded snapshot issues no command instead of flickering', async () => {
  // Two intents land back to back: only the newest one may reach a provider.
  const harness = createHarness({ delays: { 'ch.show': 30 } })

  harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.sync(
    snapshot({ tabs: [pluginTab('p')], activeTabId: 'p', geometry: geometry() })
  )
  await harness.coordinator.settled()

  assert.deepEqual(harness.log, ['pl.show:p'])
  assert.equal(harness.native().plugin.visible, true)
  assert.equal(harness.native().chatHub.visible, false)
  assert.equal(harness.maxInFlight(), 1)
})

test('a slow release finishes before the neighbour tab is shown', async () => {
  const harness = createHarness({ delays: { 'ch.destroy': 30, 'ch.show': 1 } })

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), chatTab('b')], activeTabId: 'a' })
  )
  await harness.coordinator.settled()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('b')], activeTabId: 'b' }))
  await harness.coordinator.settled()

  const destroyIndex = harness.log.indexOf('ch.destroy:a')
  const showIndex = harness.log.lastIndexOf('ch.show:b')
  assert.ok(destroyIndex >= 0, 'the closed tab must still be released')
  assert.ok(showIndex > destroyIndex, 'the neighbour must be shown after the release')
  assert.equal(harness.native().chatHub.tabId, 'b')
  assert.equal(harness.maxInFlight(), 1)
})

test('closing a background tab releases only that tab', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), chatTab('b')], activeTabId: 'a' })
  )
  await harness.coordinator.settled()
  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), chatTab('b')], activeTabId: 'b' })
  )
  await harness.coordinator.settled()

  const before = harness.log.length
  await harness.coordinator.sync(snapshot({ tabs: [chatTab('b')], activeTabId: 'b' }))
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(before), ['ch.destroy:a'])
  assert.equal(harness.native().chatHub.tabId, 'b', 'the visible tab must stay')
  assert.equal(harness.native().chatHub.visible, true)
})

test('hiding the dock preserves the tab and re-showing reuses the same session', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()
  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: '' }))
  await harness.coordinator.settled()
  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()

  assert.deepEqual(harness.log, ['ch.show:a', 'ch.hide', 'ch.show:a'])
  assert.equal(
    harness.log.includes('ch.destroy:a'),
    false,
    'hiding must never release the tab session'
  )
})

test('a resize re-places the visible tab and never resurrects a hidden one', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a')], activeTabId: 'a', geometry: geometry(900, 600) })
  )
  await harness.coordinator.settled()
  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a')], activeTabId: 'a', geometry: geometry(880, 620) })
  )
  await harness.coordinator.settled()

  assert.equal(harness.native().chatHub.bounds.width, 620, 'the newest bounds must win')

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a')], activeTabId: '', geometry: geometry(880, 620) })
  )
  await harness.coordinator.settled()
  const hiddenAt = harness.log.length

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a')], activeTabId: '', geometry: geometry(860, 640) })
  )
  await harness.coordinator.settled()

  assert.equal(
    harness.log.length,
    hiddenAt,
    'a hidden dock must not be shown again by a layout change'
  )
  assert.equal(harness.native().chatHub.visible, false)
})

test('an unchanged snapshot issues no command', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()
  const count = harness.log.length

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()

  assert.equal(harness.log.length, count)
})

test('the trusted tab keeps both native providers hidden', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), trustedTab('s')], activeTabId: 's' })
  )
  await harness.coordinator.settled()

  assert.equal(harness.native().chatHub.visible, false)
  assert.equal(harness.native().plugin.visible, false)
  assert.equal(
    harness.log.filter(entry => entry.startsWith('ch.show:') || entry.startsWith('pl.show:')).length,
    1,
    'the trusted tab must not open a native view'
  )
})

test('a failed show is reported once and the queue keeps converging', async () => {
  let fail = true
  const harness = createHarness()
  const coordinator = createDockedViewsCoordinator({
    chatHub: {
      show: () => (fail ? Promise.reject(new Error('boom')) : Promise.resolve()),
      hide: () => Promise.resolve(),
      destroy: () => Promise.resolve(),
      destroyAll: () => Promise.resolve(),
      reload: () => Promise.resolve()
    },
    plugin: {
      show: () => Promise.resolve(),
      hide: () => Promise.resolve(),
      close: () => Promise.resolve(),
      clear: () => Promise.resolve()
    },
    onError: (error, action) => harness.errors.push({ error, action })
  })

  await coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await coordinator.settled()
  assert.equal(harness.errors.at(-1).action, 'chatHub.show')
  assert.equal(coordinator.applied(), null, 'a failed show must never be reported as visible')

  fail = false
  await coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await coordinator.settled()

  assert.deepEqual(coordinator.applied(), { kind: 'chatHub', tabId: 'a' })
})

test('reload keeps a chat page in place and renews a plugin panel through the queue', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()
  await harness.coordinator.reload('a')
  await harness.coordinator.settled()
  assert.deepEqual(harness.log.slice(-1), ['ch.reload:a'])
  assert.equal(harness.native().chatHub.visible, true, 'a chat reload must not hide the page')

  await harness.coordinator.sync(snapshot({ tabs: [pluginTab('p')], activeTabId: 'p' }))
  await harness.coordinator.settled()
  const before = harness.log.length
  await harness.coordinator.reload('p')
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(before), ['pl.close:p', 'pl.show:p'])
  assert.equal(harness.native().plugin.visible, true)
  assert.equal(harness.native().plugin.tabId, 'p')
  assert.equal(harness.maxInFlight(), 1)
})

test('a rapid show, hide, close, reopen and resize burst converges on the last intent', async () => {
  const harness = createHarness({ delays: { 'ch.show': 5, 'ch.hide': 5, 'pl.show': 5 } })

  harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: '' }))
  harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), pluginTab('p')], activeTabId: 'p' })
  )
  harness.coordinator.sync(
    snapshot({
      tabs: [chatTab('a'), pluginTab('p')],
      activeTabId: 'p',
      geometry: geometry(820, 640)
    })
  )
  await harness.coordinator.settled()

  assert.deepEqual(harness.coordinator.applied(), { kind: 'plugin', tabId: 'p' })
  assert.equal(harness.native().plugin.visible, true)
  assert.equal(harness.native().chatHub.visible, false)
  assert.equal(harness.maxInFlight(), 1, 'no two provider commands may ever overlap')
})

test('dispose releases every native view', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), pluginTab('p')], activeTabId: 'p' })
  )
  await harness.coordinator.settled()

  await harness.coordinator.dispose()
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(-2), ['ch.destroyAll', 'pl.clear'])
  assert.equal(harness.native().chatHub.visible, false)
  assert.equal(harness.native().plugin.visible, false)
  assert.equal(harness.coordinator.applied(), null)
  assert.deepEqual(harness.coordinator.openTabs(), [])
})

test('a reload of a tab that is no longer in the dock is a no-op', async () => {
  const harness = createHarness()

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()
  const count = harness.log.length

  await harness.coordinator.reload('missing')
  await harness.coordinator.settled()

  assert.equal(harness.log.length, count)
})

test('radius-only changes reach both carriers even when the rectangle stays unchanged', async () => {
  for (const kind of ['chatHub', 'plugin']) {
    const calls = []
    const provider = {
      show: async (_tabId, payload) => calls.push(payload),
      hide: async () => {},
      close: async () => {},
      destroy: async () => {}
    }
    const coordinator = createDockedViewsCoordinator({ chatHub: provider, plugin: provider })
    const tab = kind === 'chatHub' ? chatTab('a') : pluginTab('p')
    const tabs = [tab]
    const first = geometry()
    await coordinator.sync(snapshot({ tabs, activeTabId: tab.tabId, geometry: first }))
    const square = { ...first, cornerRadius: 0 }
    await coordinator.sync(snapshot({ tabs, activeTabId: tab.tabId, geometry: square }))
    await coordinator.sync(snapshot({ tabs, activeTabId: tab.tabId, geometry: square }))
    await coordinator.settled()
    assert.equal(calls.length, 2, `${kind} must apply a radius change and dedupe the repeat`)
    assert.equal(calls[0].cornerRadius, 8)
    assert.equal(calls[1].cornerRadius, 0)
    assert.deepEqual(calls[1].bounds, first.bounds)
  }
})

test('settled resolves on an idle coordinator without running any operation', async () => {
  const harness = createHarness()

  await harness.coordinator.settled()

  assert.deepEqual(harness.log, [])
})