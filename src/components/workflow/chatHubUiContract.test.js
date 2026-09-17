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
const page = read('../../../src-tauri/src/chat_hub/page.rs')
const layoutStyles = read('../../styles/workflow/layout.scss')
const globalStyles = read('../../style/chatspeed/style.scss')

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
  // The entry uses the shared icon component at the large size; the glyph itself is a
  // presentation choice and stays free to change.
  assert.match(entry, /<cs name="[a-z-]+" size="var\(--cs-font-size-lg\)" \/>/)
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
    /const chatHubView = createChatHubViewController\(\{[\s\S]*?invokeWrapper\('show_chat_hub_page', \{\s*url,\s*width,\s*topInset: chatHubTopInset\(\),\s*cornerRadius: chatHubCornerRadius\(\)\s*\}\)[\s\S]*?hide: \(\) => invokeWrapper\('hide_chat_hub_page'\)[\s\S]*?destroy: \(\) => invokeWrapper\('destroy_chat_hub_page'\)[\s\S]*?setWidth: width => invokeWrapper\('set_chat_hub_page_width', \{ width \}\)/
  )
  assert.match(workflowView, /getWidth: \(\) => chatHubStore\.pageWidth/)
  // The visible flag follows the applied native state instead of a local guess.
  assert.match(workflowView, /onVisibleChange: visible => \{\s*chatHubVisible\.value = visible\s*\}/)
  assert.match(workflowView, /const hideChatHub = \(\) => \{\s*chatHubView\.hide\(\)\s*\}/)
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
  // The reserved width is published on the document root, because the overlays and popovers
  // Element Plus teleports to the document body have to find it outside this component.
  assert.match(
    workflowView,
    /watchEffect\(\(\) => \{\s*const reserved = chatHubReservedWidth\.value[\s\S]*?root\.style\.setProperty\('--cs-chathub-reserved-width'[\s\S]*?root\.style\.removeProperty\('--cs-chathub-reserved-width'\)/
  )
  assert.doesNotMatch(workflowView, /chatHubLayoutStyle/)
  assert.match(workflowView, /<div class="workflow-layout">/)
  // The page starts below the app titlebar, so the reserved space narrows the workflow
  // content only and the titlebar keeps the full window width.
  assert.match(
    layoutStyles,
    /\.workflow-main \{[\s\S]*?margin-right: var\(--cs-chathub-reserved-width, 0px\)/
  )
  // A stacked page must not cover the app chrome, so the view reports how much room the
  // titlebar with the window controls needs.
  assert.match(workflowView, /getPropertyValue\('--cs-titlebar-height'\)/)
  assert.match(workflowView, /topInset: chatHubTopInset\(\)/)
  // The same carrier also paints over the rounded window border at its bottom-right
  // corner, so the view reports the radius the window container actually draws. A
  // platform whose window keeps square corners reports nothing and the page stays
  // rectangular there.
  assert.match(workflowView, /getComputedStyle\(container\)\.borderBottomRightRadius/)
  assert.match(workflowView, /cornerRadius: chatHubCornerRadius\(\)/)
})

test('overlays and toasts stay inside the workflow UI while the page is docked', () => {
  // A dialog, a message box and a drawer center or slide within the window, so their right part
  // would end up under the native page: every layer that positions itself in the window is given
  // the width the page reserves.
  assert.match(
    globalStyles,
    /\.el-overlay \{[\s\S]*?width: calc\(100% - var\(--cs-chathub-reserved-width, 0px\)\) !important;/
  )
  assert.match(
    globalStyles,
    /\.el-overlay-dialog,\s*\.el-overlay-message-box \{\s*width: calc\(100% - var\(--cs-chathub-reserved-width, 0px\)\) !important;/
  )
  // A toast centers itself on the window and a notification is anchored to its right edge.
  assert.match(
    globalStyles,
    /\.el-message\.is-center \{\s*left: calc\(\(100% - var\(--cs-chathub-reserved-width, 0px\)\) \/ 2\) !important;/
  )
  assert.match(
    globalStyles,
    /\.el-notification\.right \{\s*right: calc\(16px \+ var\(--cs-chathub-reserved-width, 0px\)\) !important;/
  )
})

test('the entry of the shown site toggles the page and the entry list releases it', () => {
  // The docked page has no window chrome of its own, so its entry icon toggles it: a page
  // that is on screen is hidden and a hidden one comes back. The icon carries no cross
  // because the entry list below already offers the close action.
  assert.match(entry, /class="chat-hub-entry__current-surface" @click="emit\('toggle'\)"/)
  assert.doesNotMatch(entry, /chat-hub-entry__current-close/)
  assert.doesNotMatch(entry, /@click\.stop/)
  assert.match(entry, /const emit = defineEmits\(\['select', 'close', 'toggle'\]\)/)
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

test('the titlebar buttons next to the docked page point their tooltips left', () => {
  const right = section(workflowView, '<template #right>', '    </Titlebar>')
  const tooltips = right.match(/<el-tooltip/g) || []

  assert.ok(tooltips.length > 0, 'the titlebar right side has no tooltips to guard')
  // The page is a native view over everything below the titlebar, so a tooltip pointing down
  // would be hidden by it as soon as the page is open.
  assert.equal((right.match(/placement="left"/g) || []).length, tooltips.length)
  assert.doesNotMatch(right, /placement="bottom"/)
  // The window paints its titlebar as an opaque layer above the app, so a tooltip that stays
  // inside that strip has to be given a layer above it.
  assert.equal(
    (right.match(/popper-class="workflow-titlebar-tooltip"/g) || []).length,
    tooltips.length
  )
  assert.match(
    workflowView,
    /\.workflow-titlebar-tooltip\.el-popper \{\s*\/\*[\s\S]*?\*\/\s*z-index: var\(--cs-upper-layer-zindex\) !important;/
  )
})

test('the chat entry icon only opens the entry list', () => {
  // The icon is the entry list and nothing else: opening it must not show or hide the docked
  // page, so the list stays a pure entry picker.
  assert.doesNotMatch(entry, /@visible-change/)
  assert.doesNotMatch(entry, /onMenuVisibleChange/)
  assert.doesNotMatch(entry, /'open-current'/)
  assert.doesNotMatch(workflowView, /onChatHubEntryOpened/)
  assert.doesNotMatch(workflowView, /open-current-chat-hub/)
  assert.doesNotMatch(sidebar, /open-current/)

  // Selecting an entry from the menu still shows that entry.
  assert.match(entry, /@command="onSelectCommand"/)
  assert.match(workflowView, /@select-chat-hub="onSelectChatHubEntry"/)
  assert.match(workflowView, /const onSelectChatHubEntry = hub => \{\s*showChatHub\(hub\)\s*\}/)

  // The entry of the site that is docked stays the page control, which is what keeps the page
  // reachable now that the icon no longer brings it back.
  assert.match(entry, /class="chat-hub-entry__current-surface" @click="emit\('toggle'\)"/)
  assert.match(workflowView, /@toggle-chat-hub="onChatHubEntryToggled"/)
  assert.match(sidebar, /@toggle="\$emit\('toggle-chat-hub'\)"/)
})

test('the docked page is created by the carrier without Tauri IPC', () => {
  // The page is built with wry, so it never receives the Tauri IPC that a Tauri webview
  // inside this window would inherit from the workflow capabilities.
  assert.match(page, /WebViewBuilder::new_with_web_context/)
  assert.match(page, /NewWindowResponse::Deny/)
  assert.match(page, /matches!\(url\.split\(':'\)\.next\(\), Some\("http"\) \| Some\("https"\)\)/)
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
