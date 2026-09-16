import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

const workflowView = read('../../views/Workflow.vue')
const sidebar = read('./WorkflowSidebar.vue')
const entry = read('./ChatHubEntry.vue')
const splitter = read('./ChatHubSplitter.vue')
const chatHubViewController = read('../../libs/chatHubView.js')
const chatHubStore = read('../../stores/chatHub.js')
const carrier = read('../../../src-tauri/src/chat_hub/mod.rs')

/** Text between two markers, so assertions stay inside one block of a file. */
const section = (text, start, end) => {
  const from = text.indexOf(start)
  assert.ok(from >= 0, `missing section start: ${start}`)
  const to = text.indexOf(end, from)
  assert.ok(to > from, `missing section end: ${end}`)
  return text.slice(from, to)
}

const chatHubBlock = () =>
  section(workflowView, '// ChatHub (web chat entries)', 'const openWorkflowSidebarTab = tab => {')

/** Body of one top level handler in the workflow view. */
const handlerBody = name => {
  const start = workflowView.indexOf(`const ${name} = `)
  assert.ok(start >= 0, `missing handler: ${name}`)
  const end = workflowView.indexOf('\nconst ', start + 1)
  assert.ok(end > start, `missing end of handler: ${name}`)
  return workflowView.slice(start, end)
}

const occurrences = (text, needle) => text.split(needle).length - 1

test('workflow sidebar keeps the chat entry directly above the terminal entry', () => {
  assert.match(
    workflowView,
    /<div class="workflow-side-rail__bottom">\s*<ChatHubEntry[\s\S]*?class="workflow-side-rail__item workflow-side-rail__terminal"/
  )
  assert.match(
    sidebar,
    /<div class="compact-bottom-entries">\s*<ChatHubEntry[\s\S]*?class="workflow-terminal-entry compact-terminal-entry"/
  )
  assert.match(sidebar, /:hubs="chatHubs"[\s\S]*?:active-hub-id="activeChatHubId"[\s\S]*?@select="\$emit\('select-chat-hub', \$event\)"/)
  assert.match(workflowView, /@select-chat-hub="onSelectChatHubEntry"/)
})

test('chat entry renders logos with the shared avatar fallback and current entry', () => {
  assert.match(entry, /<cs name="talk" size="var\(--cs-font-size-lg\)" \/>/)
  assert.match(entry, /v-for="hub in hubs"[\s\S]*?:command="hub\.id"/)
  assert.match(entry, /<img[\s\S]*?v-if="logoOf\(hub\)"[\s\S]*?@error="markLogoBroken\(hub\)"/)
  assert.match(entry, /<avatar v-else :text="hub\.name" :size="16" \/>/)
  assert.match(entry, /const logoOf = hub => \(hub\.logo && !brokenLogoIds\.value\.has\(hub\.id\) \? hub\.logo : ''\)/)
  assert.match(entry, /v-if="activeHub"[\s\S]*?class="chat-hub-entry__current"/)
})

test('the page is docked under the app chrome, never measured from this side', () => {
  // The page is a second webview inside this very window, so the view layer never
  // measures a rectangle, follows the window or hides the workflow chat pane.
  assert.match(workflowView, /<div class="workflow-chat-pane">/)
  assert.doesNotMatch(workflowView, /ChatHubPane/)
  assert.doesNotMatch(workflowView, /getBoundingClientRect/)
  assert.doesNotMatch(workflowView, /new ResizeObserver/)
  assert.doesNotMatch(workflowView, /bounds-change/)
  assert.doesNotMatch(workflowView, /chat-hub-visible/)
  assert.doesNotMatch(workflowView, /show_chat_hub_webview|update_chat_hub_webview_bounds/)
})

test('showing a chat entry only drives the ordered view commands', () => {
  // Every view command goes through one ordered boundary, so a late IPC reply can
  // never override a newer user action (see src/libs/chatHubView.test.js).
  assert.match(
    workflowView,
    /import \{ createChatHubViewController, restoreChatHubEntry \} from '@\/libs\/chatHubView'/
  )
  assert.match(
    workflowView,
    /const chatHubView = createChatHubViewController\(\{[\s\S]*?invokeWrapper\('show_chat_hub_page', \{ url, width, topInset: chatHubTopInset\(\) \}\)[\s\S]*?hide: \(\) => invokeWrapper\('hide_chat_hub_page'\)[\s\S]*?destroy: \(\) => invokeWrapper\('destroy_chat_hub_page'\)[\s\S]*?setWidth: width => invokeWrapper\('set_chat_hub_page_width', \{ width \}\)/
  )
  assert.match(workflowView, /getWidth: \(\) => chatHubStore\.pageWidth/)
  // The visible flag follows the applied native state instead of a local guess.
  assert.match(workflowView, /onVisibleChange: visible => \{\s*chatHubVisible\.value = visible\s*\}/)
  assert.match(workflowView, /const hideChatHub = \(\) => \{\s*chatHubView\.hide\(\)\s*\}/)
  assert.match(workflowView, /const onChatHubEntryOpened = \(\) => \{\s*restoreChatHubEntry\(chatHubView, activeChatHub\.value\)\s*\}/)
  // Failures only report a message and keep the original workflow UI usable.
  assert.match(workflowView, /onError: \(error, action\) => \{[\s\S]*?workflow\.chatHub\.\$\{key\}/)

  const block = chatHubBlock()
  // The commands are issued from the ordered boundary only: no second
  // fire-and-forget call site may bypass it.
  assert.equal(occurrences(block, "invokeWrapper('show_chat_hub_page'"), 1)
  assert.equal(occurrences(block, "invokeWrapper('hide_chat_hub_page'"), 1)
  assert.equal(occurrences(block, "invokeWrapper('destroy_chat_hub_page'"), 1)
  assert.equal(occurrences(block, "invokeWrapper('set_chat_hub_page_width'"), 1)
})

test('the splitter owns the width and clamps it with the limits the backend enforces', () => {
  assert.match(
    workflowView,
    /<ChatHubSplitter\s+v-if="chatHubVisible"[\s\S]*?:right="chatHubReservedWidth"[\s\S]*?:width="chatHubStore\.pageWidth"[\s\S]*?:min-width="chatHubStore\.pageMinWidth"[\s\S]*?:min-host-width="chatHubStore\.pageMinHostWidth"[\s\S]*?@resize="onChatHubPageResize"/
  )
  assert.match(
    workflowView,
    /const onChatHubPageResize = width => \{\s*chatHubStore\.setPageWidth\(width\)\s*chatHubView\.resize\(\)\s*\}/
  )
  // The drag mirrors the backend clamp, so this side never asks for a width the page
  // cannot have.
  assert.match(chatHubStore, /invokeWrapper\('get_chat_hub_view_mode'\)/)
  assert.match(chatHubStore, /invokeWrapper\('get_chat_hub_page_limits'\)/)
  assert.match(splitter, /maxWidth = Math\.max\(props\.minWidth, \(await windowInnerWidth\(\)\) - props\.minHostWidth\)/)
  assert.match(splitter, /Math\.min\(maxWidth, Math\.max\(props\.minWidth, startWidth - \(clientX - startX\)\)\)/)
  assert.match(splitter, /cursor: col-resize/)
  assert.match(splitter, /requestAnimationFrame/)
})

test('a width change never re-shows a hidden page', () => {
  assert.match(
    chatHubViewController,
    /const resize = \(\) =>\s*enqueue\(async \(\) => \{\s*if \(!appliedVisible \|\| !appliedUrl\) \{\s*return\s*\}/
  )
})

test('stacked carriers keep the page inside the reserved space, splitting carriers do not', () => {
  assert.match(
    workflowView,
    /const chatHubReservedWidth = computed\(\(\) =>\s*chatHubStore\.viewMode === 'reserve' && chatHubVisible\.value \? chatHubStore\.pageWidth : 0\s*\)/
  )
  assert.match(workflowView, /const chatHubLayoutStyle = computed\(\(\) =>\s*chatHubReservedWidth\.value \? \{ paddingRight: `\$\{chatHubReservedWidth\.value\}px` \} : undefined\s*\)/)
  assert.match(workflowView, /<div class="workflow-layout" :style="chatHubLayoutStyle">/)
  // A stacked page must not cover the app chrome, so the view reports how much room the
  // titlebar with the window controls needs.
  assert.match(workflowView, /getPropertyValue\('--cs-titlebar-height'\)/)
  assert.match(workflowView, /topInset: chatHubTopInset\(\)/)
})

test('the entry of the shown site toggles the page and the entry list releases it', () => {
  // The docked page has no window chrome of its own, so its entry icon toggles it: a page
  // that is on screen is hidden and a hidden one comes back. The icon carries no cross
  // because the entry list below already offers the close action.
  assert.match(entry, /class="chat-hub-entry__current-surface" @click="emit\('toggle'\)"/)
  assert.doesNotMatch(entry, /chat-hub-entry__current-close/)
  assert.doesNotMatch(entry, /@click\.stop/)
  assert.match(entry, /const emit = defineEmits\(\['select', 'open-current', 'close', 'toggle'\]\)/)
  assert.match(workflowView, /@toggle="onChatHubEntryToggled"/)
  assert.match(sidebar, /@toggle="\$emit\('toggle-chat-hub'\)"/)
  assert.match(workflowView, /@toggle-chat-hub="onChatHubEntryToggled"/)
  // Toggling hides a page that is on screen and brings a hidden one back, so the same
  // entry stays useful in both states.
  assert.match(
    workflowView,
    /const onChatHubEntryToggled = \(\) => \{\s*if \(chatHubVisible\.value\) \{\s*hideChatHub\(\)\s*return\s*\}\s*restoreChatHubEntry\(chatHubView, activeChatHub\.value\)\s*\}/
  )
  // The cross must not come back as a second close action on the icon; the icon keeps a
  // hover highlight so it still reads as clickable.
  assert.doesNotMatch(entry, /current-close/)
  assert.match(
    entry,
    /\.chat-hub-entry__current-surface \{[\s\S]*?&:hover \{\s*background-color: var\(--cs-hover-bg-color\);\s*\}/
  )

  // Releasing the page stays in the entry list, which is the only close action.
  assert.match(entry, /const CLOSE_COMMAND = 'close'/)
  assert.match(entry, /:command="CLOSE_COMMAND"/)
  assert.match(entry, /if \(command === CLOSE_COMMAND\) \{\s*emit\('close'\)\s*return\s*\}/)
  assert.match(entry, /\$t\('workflow\.chatHub\.close'\)/)
  assert.match(workflowView, /@close="onCloseChatHub"/)
  assert.match(sidebar, /@close="\$emit\('close-chat-hub'\)"/)
  assert.match(workflowView, /@close-chat-hub="onCloseChatHub"/)
  // Closing selects no entry and releases the page through the ordered boundary, which
  // hides first and destroys afterwards.
  assert.match(
    workflowView,
    /const onCloseChatHub = \(\) => \{\s*chatHubStore\.setActiveHub\(0\)\s*chatHubView\.close\(\)\s*\}/
  )
  assert.match(chatHubViewController, /const close = \(\) => \{\s*hideNow\(\)[\s\S]*?await destroy\(\)/)
})

test('the chat entry menu restores the current page', () => {
  // Opening the entry list is the "show the page again" gesture.
  assert.match(
    workflowView,
    /<div class="workflow-side-rail__bottom">\s*<ChatHubEntry[\s\S]*?@open-current="onChatHubEntryOpened"/
  )
  assert.match(entry, /@visible-change="onMenuVisibleChange"/)
  assert.match(
    entry,
    /const onMenuVisibleChange = visible => \{\s*if \(visible\) \{\s*emit\('open-current'\)/
  )
  // The open action submits a restore to the ordered boundary instead of checking
  // the lagging applied visibility, which would drop the newest intent while a
  // hide is still queued.
  assert.match(
    workflowView,
    /const onChatHubEntryOpened = \(\) => \{\s*restoreChatHubEntry\(chatHubView, activeChatHub\.value\)\s*\}/
  )
  // The collapsed sidebar uses the same contract.
  assert.match(sidebar, /@open-current="\$emit\('open-current-chat-hub'\)"/)
  assert.match(workflowView, /@open-current-chat-hub="onChatHubEntryOpened"/)
  // Selecting an entry from the menu still shows that entry.
  assert.match(entry, /@command="onSelectCommand"/)
  assert.match(workflowView, /@select-chat-hub="onSelectChatHubEntry"/)
  assert.match(workflowView, /const onSelectChatHubEntry = hub => \{\s*showChatHub\(hub\)\s*\}/)
})

test('the docked page is created by the carrier without Tauri IPC', () => {
  // The page is built with wry, so it never receives the Tauri IPC that a Tauri webview
  // inside this window would inherit from the workflow capabilities.
  assert.match(carrier, /WebViewBuilder::new_with_web_context/)
  assert.match(carrier, /NewWindowResponse::Deny/)
  assert.match(carrier, /matches!\(url\.split\(':'\)\.next\(\), Some\("http"\) \| Some\("https"\)\)/)
  assert.doesNotMatch(workflowView, /show_chat_hub_webview/)
})

test('switching workflow views keeps the docked page in place', () => {
  // The page is docked next to the workflow UI, so tasks, automations, sidebar tabs, the
  // terminal and the path button only change the workflow view: the entry owns hiding.
  assert.doesNotMatch(workflowView, /watch\(workflowSidebarNavigationTab, hideChatHub\)/)
  assert.doesNotMatch(workflowView, /watch\(workflowSidebarActiveTab, hideChatHub\)/)
  assert.doesNotMatch(workflowView, /watch\(currentWorkflowId, hideChatHub\)/)
  assert.doesNotMatch(
    workflowView,
    /watch\(\(\) => workflowAutomationStore\.selectedAutomationId, hideChatHub\)/
  )
  assert.doesNotMatch(
    workflowView,
    /terminal\.visible,\s*visible => \{\s*if \(visible\) \{\s*hideChatHub\(\)/
  )

  // The one view change that still hides the page is a deleted entry, which clears the
  // current selection elsewhere.
  assert.match(
    workflowView,
    /watch\(\s*\(\) => chatHubStore\.activeHubId,\s*hubId => \{\s*if \(!hubId\) \{\s*hideChatHub\(\)/
  )

  const block = chatHubBlock()
  // Hiding or showing a chat entry must never send a workflow runtime signal.
  assert.doesNotMatch(block, /emitWorkflowSignal|sendSignal|SIGNAL_TYPES/)
  assert.doesNotMatch(block, /stopWorkflow|clearContext|resumeWorkflow|selectWorkflow\(/)
})

test('tasks, automations, sidebar tabs and the path button no longer hide the page', () => {
  for (const name of [
    'openWorkflowSidebarTab',
    'onTitlebarPrimaryPathClick',
    'onSelectWorkflowFromHistory',
    'onSelectAutomation'
  ]) {
    assert.doesNotMatch(handlerBody(name), /hideChatHub/, `${name} must not hide the docked page`)
  }

  // The rail and the compact sidebar entries only open the terminal, so they are no longer
  // hide regions for the page.
  assert.doesNotMatch(workflowView, /class="workflow-side-rail__terminal-entry"[^>]*@click/)
  assert.match(workflowView, /class="workflow-side-rail__terminal-entry">/)
  assert.doesNotMatch(sidebar, /class="compact-terminal-entry-group"[^>]*@click/)
  assert.match(sidebar, /class="compact-terminal-entry-group">/)
  assert.doesNotMatch(sidebar, /hide-chat-hub/)
})

test('the workflow-side chat view state is not persisted into workflow settings', () => {
  assert.match(workflowView, /import \{ useChatHubStore \} from '@\/stores\/chatHub'/)
  assert.doesNotMatch(workflowView, /settings\.chatHub/)
  assert.match(chatHubStore, /defineStore\('chat_hub'/)
})
