import assert from 'node:assert/strict'
import test from 'node:test'

import { createChatHubViewController, restoreChatHubEntry } from './chatHubView.js'

/** Width the docked page uses in the tests, in logical pixels. */
const PAGE_WIDTH = 640

const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

/**
 * Test harness with commands that resolve after configurable delays, so the tests
 * can force the completion orders that used to leave the docked page in the wrong
 * state.
 */
const createHarness = ({ delays = {} } = {}) => {
  const log = []
  const errors = []
  const reported = []
  let inFlight = 0
  let maxInFlight = 0
  let nativeVisible = false
  let nativeUrl = ''
  let nativeWidth = 0
  let width = PAGE_WIDTH

  const delayFor = (kind, argument) => {
    const configured = delays[kind]
    if (typeof configured === 'function') {
      return configured(argument)
    }
    return configured ?? 0
  }

  const run = async (kind, argument, apply) => {
    inFlight += 1
    maxInFlight = Math.max(maxInFlight, inFlight)
    log.push(argument === undefined ? kind : `${kind}:${argument}`)
    await delay(delayFor(kind, argument))
    apply()
    inFlight -= 1
  }

  const controller = createChatHubViewController({
    show: (url, width) => {
      assert.ok(width, 'show must receive a usable page width')
      return run('show', url, () => {
        nativeVisible = true
        nativeUrl = url
        nativeWidth = width
      })
    },
    hide: () => run('hide', undefined, () => {
      nativeVisible = false
    }),
    destroy: () => run('destroy', undefined, () => {
      nativeVisible = false
      nativeUrl = ''
      nativeWidth = 0
    }),
    setWidth: applied => run('width', applied, () => {
      nativeWidth = applied
    }),
    getWidth: () => width,
    onVisibleChange: (visible, url) => reported.push({ visible, url }),
    onError: (error, action) => errors.push({ error, action })
  })

  return {
    controller,
    log,
    errors,
    reported,
    setWidth: value => {
      width = value
    },
    native: () => ({ visible: nativeVisible, url: nativeUrl, width: nativeWidth }),
    maxInFlight: () => maxInFlight
  }
}

test('a late hide never hides a page the user restored immediately after', async () => {
  // The hide is slow and the restore is fast: with unserialized fire-and-forget
  // commands the hide would resolve last and hide a page the UI reports as visible.
  const harness = createHarness({ delays: { hide: 30, show: 1 } })

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  harness.controller.hide()
  harness.controller.restore()
  await harness.controller.settled()

  assert.deepEqual(harness.native(), {
    visible: true,
    url: 'https://chatgpt.com/',
    width: PAGE_WIDTH
  })
  assert.equal(harness.controller.isVisible(), true)
  assert.deepEqual(harness.reported.at(-1), { visible: true, url: 'https://chatgpt.com/' })
  assert.equal(harness.maxInFlight(), 1, 'ChatHub commands must never overlap')
})

test('a slow release finishes before the next entry is shown', async () => {
  const harness = createHarness({ delays: { destroy: 30, show: 1 } })

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  harness.controller.close()
  harness.controller.select('https://claude.ai/')
  await harness.controller.settled()

  const destroyIndex = harness.log.indexOf('destroy')
  const showIndex = harness.log.lastIndexOf('show:https://claude.ai/')
  assert.ok(destroyIndex >= 0, 'the explicit close must still release the page')
  assert.ok(showIndex > destroyIndex, 'the show must be applied after the release completed')
  assert.equal(harness.native().visible, true)
  assert.equal(harness.native().url, 'https://claude.ai/')
  assert.equal(harness.maxInFlight(), 1)
})

test('a superseded command is dropped instead of flickering the page', async () => {
  const harness = createHarness({ delays: { hide: 30 } })

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  harness.controller.hide()
  harness.controller.restore()
  await harness.controller.settled()

  assert.equal(
    harness.log.includes('hide'),
    false,
    'a hide that a newer restore already superseded must not reach the command'
  )
  assert.equal(harness.native().visible, true)
  assert.equal(harness.native().url, 'https://chatgpt.com/')
})

test('the newest entry wins when the menu opens and another entry is picked at once', async () => {
  // Opening the menu restores the current entry, picking another one selects it:
  // the two shows must not race, and the picked entry has to stay on screen.
  const harness = createHarness({
    delays: { show: url => (url === 'https://chatgpt.com/' ? 30 : 1) }
  })

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  harness.controller.restore()
  harness.controller.select('https://deepseek.chat/')
  await harness.controller.settled()

  assert.equal(harness.native().url, 'https://deepseek.chat/')
  assert.equal(harness.controller.isVisible(), true)
  assert.deepEqual(harness.reported.at(-1), {
    visible: true,
    url: 'https://deepseek.chat/'
  })
  assert.equal(harness.maxInFlight(), 1)
})

test('clicking the terminal and then the chat entry brings the current page back', async () => {
  const harness = createHarness({ delays: { hide: 20, show: 1 } })

  await harness.controller.select('https://gemini.google.com/app')
  await harness.controller.settled()

  // Terminal click hides the page, chat entry click restores it right away.
  harness.controller.hide()
  harness.controller.restore()
  await harness.controller.settled()

  assert.equal(harness.native().visible, true)
  assert.equal(harness.native().url, 'https://gemini.google.com/app')
  assert.equal(harness.controller.isVisible(), true)
  assert.equal(harness.maxInFlight(), 1)
})

test('the chat entry restores the page right after a terminal hide', async () => {
  // Sidebar integration path. The applied visibility still reports the page as
  // visible while the queued hide runs, so gating the restore on it would drop the
  // newest intent and leave the page hidden.
  const harness = createHarness({ delays: { hide: 30, show: 1 } })
  const hub = { id: 1, url: 'https://chatgpt.com/' }

  await harness.controller.select(hub.url)
  await harness.controller.settled()

  harness.controller.hide()
  assert.equal(
    harness.controller.isVisible(),
    true,
    'the applied state lags behind the hide that was just requested'
  )

  restoreChatHubEntry(harness.controller, hub)
  await harness.controller.settled()

  assert.equal(harness.native().visible, true)
  assert.equal(harness.native().url, hub.url)
  assert.equal(harness.controller.isVisible(), true)
  assert.equal(harness.maxInFlight(), 1)
})

test('a guard on the applied visibility would lose the restore', async () => {
  // Documents why the sidebar entry submits a restore instead of checking the
  // applied flag: with that guard the page stays hidden and the command is never
  // sent.
  const harness = createHarness({ delays: { hide: 30, show: 1 } })
  const hub = { id: 1, url: 'https://chatgpt.com/' }
  const legacyOpen = visible => {
    if (!visible && hub) {
      harness.controller.restore(hub.url)
    }
  }

  await harness.controller.select(hub.url)
  await harness.controller.settled()

  harness.controller.hide()
  legacyOpen(harness.controller.isVisible())
  await harness.controller.settled()

  assert.equal(
    harness.native().visible,
    false,
    'the legacy guard drops the restore, which is the defect the adapter avoids'
  )
  assert.equal(harness.native().url, hub.url)
})

test('restoring the already desired entry does not issue another command', async () => {
  const harness = createHarness()
  const hub = { id: 1, url: 'https://chatgpt.com/' }

  await harness.controller.select(hub.url)
  await harness.controller.settled()
  const commandsBefore = harness.log.length

  restoreChatHubEntry(harness.controller, hub)
  await harness.controller.settled()

  assert.equal(harness.log.length, commandsBefore)
  assert.equal(harness.native().visible, true)
  assert.equal(harness.native().url, hub.url)
})

test('opening the chat entry without an active entry does nothing', async () => {
  const harness = createHarness()

  restoreChatHubEntry(harness.controller, null)
  restoreChatHubEntry(harness.controller, { id: 0, url: '' })
  await harness.controller.settled()

  assert.equal(harness.log.length, 0)
})

test('a page that fails to show reports the show action and stays hidden', async () => {
  const harness = createHarness()
  const failing = createChatHubViewController({
    show: () => Promise.reject(new Error('ipc failed')),
    hide: () => Promise.resolve(),
    destroy: () => Promise.resolve(),
    setWidth: () => Promise.resolve(),
    getWidth: () => PAGE_WIDTH,
    onVisibleChange: (visible, url) => harness.reported.push({ visible, url }),
    onError: (error, action) => harness.errors.push({ error, action })
  })

  await failing.select('https://chatgpt.com/')
  await failing.settled()

  assert.equal(failing.isVisible(), false)
  assert.equal(
    harness.reported.some(entry => entry.visible),
    false,
    'a failed show must never be reported as visible'
  )
  assert.equal(harness.errors.at(-1).action, 'show')
})

test('a page without a usable width stays hidden and does not reach the command', async () => {
  const harness = createHarness()
  const withoutWidth = createChatHubViewController({
    show: () => {
      throw new Error('show must not be called without a width')
    },
    hide: () => Promise.resolve(),
    destroy: () => Promise.resolve(),
    setWidth: () => Promise.resolve(),
    getWidth: () => null,
    onVisibleChange: (visible, url) => harness.reported.push({ visible, url }),
    onError: (error, action) => harness.errors.push({ error, action })
  })

  await withoutWidth.select('https://chatgpt.com/')
  await withoutWidth.settled()

  assert.equal(withoutWidth.isVisible(), false)
  assert.equal(
    harness.reported.some(entry => entry.visible),
    false,
    'a page without a width must stay hidden'
  )
  assert.equal(harness.errors.length, 0)
})

test('closing the page hides it and releases it in that order', async () => {
  const harness = createHarness()

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  await harness.controller.close()
  await harness.controller.settled()

  assert.deepEqual(harness.log, ['show:https://chatgpt.com/', 'hide', 'destroy'])
  assert.equal(harness.controller.isVisible(), false)
  assert.equal(harness.native().visible, false)
  assert.equal(harness.native().url, '')
  assert.equal(harness.maxInFlight(), 1)
})

test('a width change never re-shows a hidden page', async () => {
  const harness = createHarness()

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  harness.controller.hide()
  await harness.controller.settled()

  harness.controller.resize()
  await harness.controller.settled()

  assert.equal(harness.log.includes('width'), false, 'a hidden page must not be resized or shown')
  assert.equal(harness.native().visible, false)
  // Hiding keeps the loaded page, only visibility changes.
  assert.equal(harness.native().url, 'https://chatgpt.com/')
})

test('dragging the splitter applies the newest width and never overlaps commands', async () => {
  const harness = createHarness({ delays: { width: 10 } })

  await harness.controller.select('https://chatgpt.com/')
  await harness.controller.settled()

  // A drag reports a width per frame: the queue applies them in order, one at a
  // time, and the page ends on the newest width.
  harness.setWidth(700)
  harness.controller.resize()
  harness.setWidth(760)
  harness.controller.resize()
  harness.setWidth(820)
  await harness.controller.settled()

  assert.equal(harness.native().width, 820)
  assert.equal(harness.maxInFlight(), 1)
})

test('a width change follows the show that queued it', async () => {
  const harness = createHarness({ delays: { show: 10 } })

  harness.controller.select('https://chatgpt.com/')
  harness.setWidth(720)
  harness.controller.resize()
  await harness.controller.settled()

  assert.deepEqual(harness.log, ['show:https://chatgpt.com/', 'width:720'])
  assert.equal(harness.native().width, 720)
  assert.equal(harness.maxInFlight(), 1)
})
