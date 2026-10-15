import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DOCK_LAYOUT_VERSION,
  DOCK_MAX_PLUGIN_TABS,
  createDockedViewsCoordinator,
  hydrateDockLayout,
  serializeDockLayout
} from './dockedViews.js'

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
const agentSkillsTab = (tabId = 'agentSkills') => ({
  kind: 'agentSkills',
  tabId,
  pluginId: 'agent-skills'
})

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

test('a failed show is reported once and a new rectangle converges it', async () => {
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

  // Repeating the failing snapshot is settled by the coordinator, so the retry has to be a
  // genuinely new intent. A new rectangle is one, and the provider having recovered lets it
  // through.
  fail = false
  await coordinator.sync(
    snapshot({ tabs: [chatTab('a')], activeTabId: 'a', geometry: geometry(880, 620) })
  )
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

/**
 * Harness that adds the trusted Agent Skills surface as the third provider, so the tests can check
 * it enters the one serial queue with the other two and that its own commands never overlap.
 */
const createAgentSkillsHarness = ({ delays = {} } = {}) => {
  const log = []
  const errors = []
  let inFlight = 0
  let maxInFlight = 0
  const native = {
    chatHub: { visible: false, tabId: '' },
    agentSkills: { visible: false, bounds: null, cornerRadius: null }
  }

  const run = async (kind, argument, apply) => {
    inFlight += 1
    maxInFlight = Math.max(maxInFlight, inFlight)
    log.push(argument === undefined ? kind : `${kind}:${argument}`)
    await delay(typeof delays[kind] === 'function' ? delays[kind]() : (delays[kind] ?? 0))
    apply()
    inFlight -= 1
  }

  const chatHub = {
    show: (tabId, payload) =>
      run('ch.show', tabId, () => {
        native.chatHub.visible = true
        native.chatHub.tabId = tabId
        void payload
      }),
    hide: () =>
      run('ch.hide', undefined, () => {
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
    show: () => run('pl.show', undefined, () => {}),
    hide: () => run('pl.hide', undefined, () => {}),
    close: () => run('pl.close', undefined, () => {}),
    clear: () => run('pl.clear', undefined, () => {})
  }

  const agentSkills = {
    show: (tabId, payload) =>
      run('as.show', tabId, () => {
        native.agentSkills.visible = true
        native.agentSkills.bounds = payload.bounds
        native.agentSkills.cornerRadius = payload.cornerRadius
      }),
    hide: () =>
      run('as.hide', undefined, () => {
        native.agentSkills.visible = false
      }),
    close: () =>
      run('as.close', undefined, () => {
        native.agentSkills.visible = false
      })
  }

  const coordinator = createDockedViewsCoordinator({
    chatHub,
    plugin,
    agentSkills,
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

test('the trusted surface enters the one queue and the chat page is hidden before it is shown', async () => {
  const harness = createAgentSkillsHarness({ delays: { 'ch.hide': 20, 'as.show': 1 } })

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()
  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), agentSkillsTab()], activeTabId: 'agentSkills' })
  )
  await harness.coordinator.settled()

  assert.deepEqual(harness.log, ['ch.show:a', 'ch.hide', 'as.show:agentSkills'])
  assert.equal(harness.native().agentSkills.visible, true)
  assert.equal(harness.native().chatHub.visible, false, 'the chat page must be hidden first')
  assert.equal(harness.maxInFlight(), 1, 'provider commands must never overlap')
  // The surface only receives the measured rectangle and the corner radius, never a url.
  assert.deepEqual(harness.native().agentSkills.bounds, geometry().bounds)
  assert.equal(harness.native().agentSkills.cornerRadius, 8)
})

test('hiding the dock keeps the trusted surface session and re-showing reuses it', async () => {
  const harness = createAgentSkillsHarness()

  await harness.coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: 'agentSkills' }))
  await harness.coordinator.settled()
  await harness.coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: '' }))
  await harness.coordinator.settled()
  await harness.coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: 'agentSkills' }))
  await harness.coordinator.settled()

  assert.deepEqual(harness.log, ['as.show:agentSkills', 'as.hide', 'as.show:agentSkills'])
  assert.equal(
    harness.log.includes('as.close'),
    false,
    'hiding must never release the trusted surface'
  )
})

test('closing the trusted tab releases only its own provider', async () => {
  const harness = createAgentSkillsHarness()

  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), agentSkillsTab()], activeTabId: 'a' })
  )
  await harness.coordinator.settled()
  await harness.coordinator.sync(
    snapshot({ tabs: [chatTab('a'), agentSkillsTab()], activeTabId: 'agentSkills' })
  )
  await harness.coordinator.settled()
  const before = harness.log.length

  await harness.coordinator.sync(snapshot({ tabs: [chatTab('a')], activeTabId: 'a' }))
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(before), ['as.close', 'ch.show:a'])
  assert.equal(harness.native().agentSkills.visible, false)
  assert.equal(harness.native().chatHub.visible, true, 'the neighbouring chat tab stays')
})

test('reloading the trusted surface releases and rebuilds it through the queue', async () => {
  const harness = createAgentSkillsHarness()

  await harness.coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: 'agentSkills' }))
  await harness.coordinator.settled()
  const before = harness.log.length

  await harness.coordinator.reload('agentSkills')
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(before), ['as.close', 'as.show:agentSkills'])
  assert.equal(harness.native().agentSkills.visible, true)
  assert.equal(harness.maxInFlight(), 1)
})

test('dispose releases the trusted surface together with the other providers', async () => {
  const harness = createAgentSkillsHarness()

  await harness.coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: 'agentSkills' }))
  await harness.coordinator.settled()

  await harness.coordinator.dispose()
  await harness.coordinator.settled()

  assert.deepEqual(harness.log.slice(-1), ['as.close'])
  assert.equal(harness.native().agentSkills.visible, false)
  assert.equal(harness.coordinator.applied(), null)
})

test('a failed trusted show is reported once and never left visible', async () => {
  const errors = []
  let fail = true
  const coordinator = createDockedViewsCoordinator({
    chatHub: {
      show: async () => {},
      hide: async () => {},
      destroy: async () => {},
      destroyAll: async () => {},
      reload: async () => {}
    },
    plugin: { show: async () => {}, hide: async () => {}, close: async () => {}, clear: async () => {} },
    agentSkills: {
      show: () => (fail ? Promise.reject(new Error('boom')) : Promise.resolve()),
      hide: async () => {},
      close: async () => {}
    },
    onError: (error, action) => errors.push({ error, action })
  })

  await coordinator.sync(snapshot({ tabs: [agentSkillsTab()], activeTabId: 'agentSkills' }))
  await coordinator.settled()
  assert.equal(errors.at(-1).action, 'agentSkills.show')
  assert.equal(coordinator.applied(), null, 'a failed show must never be reported as visible')

  // The record is per tab and rectangle, so converging takes a new intent rather than the
  // same failing snapshot again.
  fail = false
  await coordinator.sync(
    snapshot({
      tabs: [agentSkillsTab()],
      activeTabId: 'agentSkills',
      geometry: geometry(880, 620)
    })
  )
  await coordinator.settled()
  assert.deepEqual(coordinator.applied(), { kind: 'agentSkills', tabId: 'agentSkills' })
})

/**
 * Providers whose `show` fails until the test lets it succeed, with one harness per provider kind.
 *
 * The same failing command used to be re-issued by every layout pass of one snapshot, so one click
 * could report the same open failure several times. The behaviour has to be the same whichever
 * provider was clicked, so one harness covers the ChatHub page, an ordinary plugin panel and the
 * trusted Agent Skills surface.
 */
const createShowFailureHarness = kind => {
  const errors = []
  const shows = []
  let failing = true

  const show = (tabId, payload) => {
    shows.push({ tabId, payload })
    return failing ? Promise.reject(new Error('boom')) : Promise.resolve()
  }
  const idle = async () => {}

  const providers = {
    chatHub: {
      show,
      hide: idle,
      destroy: idle,
      destroyAll: idle,
      reload: async () => {}
    },
    plugin: { show, hide: idle, close: idle, clear: idle },
    agentSkills: { show, hide: idle, close: idle }
  }

  const coordinator = createDockedViewsCoordinator({
    ...providers,
    onError: (error, action) => errors.push({ error, action })
  })

  return {
    coordinator,
    errors,
    showCalls: () => shows.length,
    recover: () => {
      failing = false
    }
  }
}

/** The single tab of a failing-show harness, per provider kind. */
const failingTab = kind =>
  kind === 'chatHub' ? chatTab('a') : kind === 'plugin' ? pluginTab('p') : agentSkillsTab()

const failingTabId = kind => failingTab(kind).tabId

const kinds = ['chatHub', 'plugin', 'agentSkills']

test('the layout passes of one failing snapshot do not re-issue the show or its error', async () => {
  for (const kind of kinds) {
    const harness = createShowFailureHarness(kind)
    const tabs = [failingTab(kind)]
    const activeTabId = failingTabId(kind)

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()
    assert.equal(harness.errors.length, 1, `${kind}: the failure must be reported once`)

    // Opening the dock makes room in the window, and both the window resize and the surface
    // observer re-measure the same rectangle. The visible tab and its rectangle did not change,
    // so the failing command must not run again and the user must not see the message twice.
    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()

    assert.equal(harness.showCalls(), 1, `${kind}: the failing show must not be re-issued`)
    assert.equal(harness.errors.length, 1, `${kind}: the same error must not be reported again`)
    assert.equal(harness.coordinator.applied(), null, `${kind}: a failed show is never visible`)
  }
})

test('a background tab changing does not retry a failed show', async () => {
  for (const kind of kinds) {
    const harness = createShowFailureHarness(kind)
    const activeTabId = failingTabId(kind)
    const tabs = [failingTab(kind)]

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()

    // The record is keyed by the visible tab and its rectangle, so a second tab opening in the
    // background must not bring the failing show back.
    await harness.coordinator.sync(snapshot({ tabs: [...tabs, chatTab('bg')], activeTabId }))
    await harness.coordinator.settled()

    assert.equal(harness.showCalls(), 1, `${kind}: a background tab must not retry the show`)
    assert.equal(harness.errors.length, 1, `${kind}: the error must stay reported once`)
  }
})

test('a new rectangle retries the show and a recovered provider clears the record', async () => {
  for (const kind of kinds) {
    const harness = createShowFailureHarness(kind)
    const tabs = [failingTab(kind)]
    const activeTabId = failingTabId(kind)

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 1)

    // A window resize changes the rectangle, which is a new intent and retries the command.
    await harness.coordinator.sync(
      snapshot({ tabs, activeTabId, geometry: geometry(880, 620) })
    )
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 2, `${kind}: a new rectangle must retry the show`)
    assert.equal(harness.errors.length, 2)

    // The same new rectangle repeating is settled again.
    await harness.coordinator.sync(
      snapshot({ tabs, activeTabId, geometry: geometry(880, 620) })
    )
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 2, `${kind}: the repeated rectangle must not re-show`)
    assert.equal(harness.errors.length, 2)

    // A successful show clears the record, so the tab is visible and unchanged snapshots are
    // still deduped by the applied signature rather than by the failure.
    harness.recover()
    await harness.coordinator.sync(
      snapshot({ tabs, activeTabId, geometry: geometry(860, 640) })
    )
    await harness.coordinator.settled()
    assert.deepEqual(harness.coordinator.applied(), {
      kind: failingTab(kind).kind,
      tabId: activeTabId
    })
    assert.equal(harness.showCalls(), 3)
    assert.equal(harness.errors.length, 2, `${kind}: a successful show reports no error`)

    await harness.coordinator.sync(
      snapshot({ tabs, activeTabId, geometry: geometry(860, 640) })
    )
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 3, `${kind}: an unchanged snapshot stays deduped`)
  }
})

test('a reload clears the recorded failure and retries the show', async () => {
  for (const kind of kinds) {
    const harness = createShowFailureHarness(kind)
    const tabs = [failingTab(kind)]
    const activeTabId = failingTabId(kind)

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()

    // The reload retries while the provider still fails, so the failure is reported once more and
    // recorded again: the following layout passes must not turn that into a message per pass.
    await harness.coordinator.reload(activeTabId)
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 2, `${kind}: a reload must retry the failing show`)
    assert.equal(harness.errors.length, 2)

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()
    assert.equal(harness.showCalls(), 2, `${kind}: the recorded failure must stand again`)
    assert.equal(harness.errors.length, 2)
  }

  for (const kind of kinds) {
    const harness = createShowFailureHarness(kind)
    const tabs = [failingTab(kind)]
    const activeTabId = failingTabId(kind)

    await harness.coordinator.sync(snapshot({ tabs, activeTabId }))
    await harness.coordinator.settled()

    // Once the provider recovers, the explicit reload opens the tab again.
    harness.recover()
    await harness.coordinator.reload(activeTabId)
    await harness.coordinator.settled()

    assert.equal(harness.showCalls(), 2)
    assert.deepEqual(harness.coordinator.applied(), {
      kind: failingTab(kind).kind,
      tabId: activeTabId
    })
  }
})

// ---------------------------------------------------------------------------
// Dock layout persistence
// ---------------------------------------------------------------------------

/** A chat hub entry as the store exposes it. */
const hub = id => ({ id, name: `hub-${id}`, url: `https://hub-${id}.example/` })
/** A plugin UI the runtime verified and left enabled. */
const plugin = (id, entry = `${id}/index.html`) => ({ id, ui: { entry, verified: true } })
/** The built-in trusted plugin whose surface the dock restores. */
const trustedPlugin = () => plugin('agent-skills', 'agent-skills/agent-skills.js')
const storedLayout = (tabs, activeIndex = 0, visible = true) =>
  JSON.stringify({ version: DOCK_LAYOUT_VERSION, visible, activeIndex, tabs })
const empty = { tabs: [], activeTabId: '', visible: false }

test('a remembered layout keeps only the identity of each restorable tab', () => {
  const raw = serializeDockLayout({
    tabs: [
      { id: 'chathub:1', kind: 'chatHub', hubId: 1 },
      { id: 'plugin:demo', kind: 'plugin', pluginId: 'demo', entry: 'demo/index.html' },
      { id: 'agentSkills', kind: 'agentSkills', pluginId: 'agent-skills' },
      { id: 'trusted:1', kind: 'trusted' }
    ],
    activeTabId: 'plugin:demo',
    visible: true
  })

  assert.deepEqual(JSON.parse(raw), {
    version: DOCK_LAYOUT_VERSION,
    visible: true,
    activeIndex: 1,
    tabs: [
      { kind: 'chatHub', hubId: 1 },
      { kind: 'plugin', pluginId: 'demo' },
      { kind: 'agentSkills' }
    ]
  })
  // A url, a plugin entry and a capability token are runtime-verified, so none is remembered.
  assert.doesNotMatch(raw, /https?:|\.html|\.js|url|entry|token/)
})

test('a hidden dock and an unknown active tab are remembered without an active index', () => {
  const raw = serializeDockLayout({
    tabs: [{ id: 'chathub:7', kind: 'chatHub', hubId: 7 }],
    activeTabId: '',
    visible: false
  })

  assert.deepEqual(JSON.parse(raw), {
    version: DOCK_LAYOUT_VERSION,
    visible: false,
    activeIndex: -1,
    tabs: [{ kind: 'chatHub', hubId: 7 }]
  })
})

test('a remembered layout is restored from the inventories that confirm it again', () => {
  const hubs = [hub(1), hub(2), hub(3)]
  // The verified entry has moved on since the layout was remembered; the restore uses this one
  // instead of anything the stored layout could have carried.
  const plugins = [plugin('demo', 'demo/v9/index.html'), trustedPlugin()]
  const raw = serializeDockLayout({
    tabs: [
      { id: 'chathub:2', kind: 'chatHub', hubId: 2 },
      { id: 'plugin:demo', kind: 'plugin', pluginId: 'demo', entry: 'demo/index.html' },
      { id: 'agentSkills', kind: 'agentSkills', pluginId: 'agent-skills' }
    ],
    activeTabId: 'plugin:demo',
    visible: true
  })

  assert.deepEqual(hydrateDockLayout(raw, { hubs, plugins }), {
    tabs: [
      { id: 'chathub:2', kind: 'chatHub', hubId: 2 },
      { id: 'plugin:demo', kind: 'plugin', pluginId: 'demo', entry: 'demo/v9/index.html' },
      { id: 'agentSkills', kind: 'agentSkills', pluginId: 'agent-skills' }
    ],
    activeTabId: 'plugin:demo',
    visible: true
  })
})

test('a restored dock keeps the visibility it had', () => {
  const hubs = [hub(1)]
  const tabs = [{ id: 'chathub:1', kind: 'chatHub', hubId: 1 }]

  const shown = hydrateDockLayout(
    serializeDockLayout({ tabs, activeTabId: 'chathub:1', visible: true }),
    { hubs }
  )
  assert.deepEqual(shown, {
    tabs: [{ id: 'chathub:1', kind: 'chatHub', hubId: 1 }],
    activeTabId: 'chathub:1',
    visible: true
  })

  // A dock that was hidden stays hidden, while its active tab is kept for the next toggle.
  const hidden = hydrateDockLayout(
    serializeDockLayout({ tabs, activeTabId: 'chathub:1', visible: false }),
    { hubs }
  )
  assert.deepEqual(hidden, {
    tabs: [{ id: 'chathub:1', kind: 'chatHub', hubId: 1 }],
    activeTabId: 'chathub:1',
    visible: false
  })
})

test('a hub that no longer exists is dropped and a neighbour becomes active', () => {
  const restored = hydrateDockLayout(
    storedLayout(
      [
        { kind: 'chatHub', hubId: 1 },
        { kind: 'chatHub', hubId: 2 },
        { kind: 'chatHub', hubId: 3 }
      ],
      1
    ),
    { hubs: [hub(1), hub(3)] }
  )

  assert.deepEqual(restored.tabs.map(tab => tab.hubId), [1, 3])
  // The removed middle tab held the active index, so the first tab after it takes over.
  assert.equal(restored.activeTabId, 'chathub:3')
})

test('a dropped trailing active tab falls back to the tab before it', () => {
  const restored = hydrateDockLayout(
    storedLayout(
      [
        { kind: 'chatHub', hubId: 1 },
        { kind: 'chatHub', hubId: 2 },
        { kind: 'plugin', pluginId: 'gone' }
      ],
      2
    ),
    { hubs: [hub(1), hub(2)], plugins: [] }
  )

  assert.deepEqual(restored.tabs.map(tab => tab.id), ['chathub:1', 'chathub:2'])
  assert.equal(restored.activeTabId, 'chathub:2')
})

test('a plugin the runtime no longer confirms is dropped and its entry is re-read', () => {
  const restored = hydrateDockLayout(
    storedLayout([
      { kind: 'plugin', pluginId: 'demo' },
      // Disabled or unverified plugins never reach the dock, so they are not offered here.
      { kind: 'plugin', pluginId: 'disabled' },
      { kind: 'plugin', pluginId: 'unverified' }
    ]),
    { hubs: [], plugins: [plugin('demo', 'demo/v3/index.html')] }
  )

  assert.deepEqual(restored.tabs, [
    { id: 'plugin:demo', kind: 'plugin', pluginId: 'demo', entry: 'demo/v3/index.html' }
  ])
})

test('the trusted surface is never restored from a stored kind or plugin id alone', () => {
  const trusted = trustedPlugin()

  // A trusted kind naming another plugin is refused instead of trusted.
  assert.deepEqual(
    hydrateDockLayout(storedLayout([{ kind: 'agentSkills', pluginId: 'evil' }]), {
      plugins: [trusted]
    }),
    empty
  )

  // The ordinary plugin kind cannot borrow the trusted plugin id either.
  assert.deepEqual(
    hydrateDockLayout(storedLayout([{ kind: 'plugin', pluginId: 'agent-skills' }]), {
      plugins: [trusted]
    }),
    empty
  )

  // The runtime not confirming the plugin keeps the surface out, whatever was remembered.
  assert.deepEqual(
    hydrateDockLayout(storedLayout([{ kind: 'agentSkills' }]), { plugins: [plugin('demo')] }),
    empty
  )

  // The runtime confirming it restores exactly one trusted tab, even from two stored copies.
  assert.deepEqual(
    hydrateDockLayout(storedLayout([{ kind: 'agentSkills' }, { kind: 'agentSkills' }]), {
      plugins: [trusted]
    }),
    {
      tabs: [{ id: 'agentSkills', kind: 'agentSkills', pluginId: 'agent-skills' }],
      activeTabId: 'agentSkills',
      visible: true
    }
  )
})

test('duplicate tabs collapse and the plugin panels are capped at the dock limit', () => {
  const plugins = Array.from({ length: DOCK_MAX_PLUGIN_TABS + 2 }, (_, index) => plugin(`p${index}`))
  const restored = hydrateDockLayout(
    storedLayout([
      { kind: 'chatHub', hubId: 1 },
      { kind: 'chatHub', hubId: 1 },
      ...plugins.map(item => ({ kind: 'plugin', pluginId: item.id }))
    ]),
    { hubs: [hub(1)], plugins }
  )

  assert.equal(DOCK_MAX_PLUGIN_TABS, 8)
  assert.deepEqual(restored.tabs.map(tab => tab.id), [
    'chathub:1',
    ...plugins.slice(0, DOCK_MAX_PLUGIN_TABS).map(item => `plugin:${item.id}`)
  ])
})

test('malformed storage restores an empty dock instead of throwing', () => {
  const hubs = [hub(1)]
  const plugins = [plugin('demo'), trustedPlugin()]
  const malformed = [
    null,
    undefined,
    '',
    'not json',
    '[]',
    '42',
    storedLayout([{ kind: 'chatHub', hubId: 1 }]).replace('"version":1', '"version":2'),
    JSON.stringify({ visible: true, activeIndex: 0, tabs: [{ kind: 'chatHub', hubId: 1 }] }),
    JSON.stringify({ version: DOCK_LAYOUT_VERSION, visible: true, activeIndex: 0, tabs: 'nope' }),
    storedLayout([null, 'x', 7, {}, { kind: 'unknown' }, { kind: 'chatHub' }, { kind: 'plugin' }])
  ]

  for (const raw of malformed) {
    assert.deepEqual(hydrateDockLayout(raw, { hubs, plugins }), empty, `raw: ${String(raw)}`)
  }

  // A layout whose tabs all disappeared yields the same empty dock.
  assert.deepEqual(
    hydrateDockLayout(storedLayout([{ kind: 'chatHub', hubId: 99 }]), { hubs, plugins }),
    empty
  )
})

test('an already parsed layout is accepted like a stored string', () => {
  const restored = hydrateDockLayout(
    { version: DOCK_LAYOUT_VERSION, visible: false, activeIndex: 0, tabs: [{ kind: 'chatHub', hubId: 1 }] },
    { hubs: [hub(1)] }
  )

  assert.deepEqual(restored, {
    tabs: [{ id: 'chathub:1', kind: 'chatHub', hubId: 1 }],
    activeTabId: 'chathub:1',
    visible: false
  })
})

test('a stored hub id is matched against the inventory that owns it', () => {
  const restored = hydrateDockLayout(storedLayout([{ kind: 'chatHub', hubId: '1' }]), {
    hubs: [hub(1)]
  })

  // The canonical id comes from the inventory, so the tab answers to the id a live click gives it.
  assert.deepEqual(restored.tabs, [{ id: 'chathub:1', kind: 'chatHub', hubId: 1 }])
})