import assert from 'node:assert/strict'
import test from 'node:test'

import { createChatHubProvider } from './chatHubView.js'

/**
 * The ChatHub provider only maps a dock command to the carrier command payload. The
 * ordering and latest-intent scenarios that used to live in this file (a late hide never
 * overriding a newer show, a superseded command being dropped, a hidden page never being
 * resurrected by a resize) are still covered, now against the one shared coordinator in
 * `dockedViews.test.js`.
 */
const createRecorder = () => {
  const calls = []
  const provider = createChatHubProvider({
    invoke: (command, args) => {
      calls.push({ command, args })
      return Promise.resolve()
    }
  })
  return { calls, provider }
}

test('show forwards the tab, the measured bounds and the retained carrier hints', async () => {
  const { calls, provider } = createRecorder()
  const bounds = { x: 1240, y: 40, width: 600, height: 700 }

  await provider.show('chathub:7', {
    url: 'https://chatgpt.com/',
    width: 600,
    topInset: 40,
    cornerRadius: 12,
    bounds
  })

  assert.deepEqual(calls, [
    {
      command: 'show_chat_hub_page',
      args: {
        url: 'https://chatgpt.com/',
        width: 600,
        topInset: 40,
        cornerRadius: 12,
        tabId: 'chathub:7',
        bounds
      }
    }
  ])
})

test('show without a payload still forwards the tab so a re-place can be requested', async () => {
  const { calls, provider } = createRecorder()

  await provider.show('chathub:1')

  assert.equal(calls[0].command, 'show_chat_hub_page')
  assert.equal(calls[0].args.tabId, 'chathub:1')
})

test('hide hides every page without an id', async () => {
  const { calls, provider } = createRecorder()

  await provider.hide()

  assert.deepEqual(calls, [{ command: 'hide_chat_hub_page', args: undefined }])
})

test('destroy releases exactly one tab while destroyAll releases every tab', async () => {
  const { calls, provider } = createRecorder()

  await provider.destroy('chathub:3')
  await provider.destroyAll()

  assert.deepEqual(calls, [
    { command: 'destroy_chat_hub_page', args: { tabId: 'chathub:3' } },
    { command: 'destroy_chat_hub_page', args: undefined }
  ])
})

test('reload targets the current page of one tab', async () => {
  const { calls, provider } = createRecorder()

  await provider.reload('chathub:4')

  assert.deepEqual(calls, [{ command: 'reload_chat_hub_page', args: { tabId: 'chathub:4' } }])
})

test('a provider without an invoke adapter resolves instead of throwing', async () => {
  const provider = createChatHubProvider()

  await provider.hide()
  await provider.show('chathub:1', {})
  await provider.destroy('chathub:1')
  await provider.destroyAll()
  await provider.reload('chathub:1')
})