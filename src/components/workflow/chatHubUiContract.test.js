import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { compileStyleAsync, parse } from 'vue/compiler-sfc'

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8')

const workflowView = read('../../views/Workflow.vue')
const sidebar = read('./WorkflowSidebar.vue')
const entry = read('./ChatHubEntry.vue')
const splitter = read('./ChatHubSplitter.vue')
const dockedComponent = read('./DockedViews.vue')
const chatHubView = read('../../libs/chatHubView.js')
const dockedViews = read('../../libs/dockedViews.js')
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

/** The shared dock implementation of the workflow view. */
const chatHubBlock = () =>
  section(
    workflowView,
    '// Docked views (ChatHub sites, plugin UI and the Agent Skills surface in one right dock)',
    '// Component refs'
  )

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
    /<div class="workflow-side-rail__bottom">[\s\S]*?<ChatHubEntry[\s\S]*?class="workflow-side-rail__item workflow-side-rail__terminal"/
  )
  assert.match(
    sidebar,
    /<div class="compact-bottom-entries">[\s\S]*?<ChatHubEntry[\s\S]*?class="workflow-terminal-entry compact-terminal-entry"/
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

test('the dock measures the shared surface and hands the rectangle to the carrier', () => {
  // The dock is a plain Vue column; the native view is painted over the measured surface,
  // so this layer measures one rectangle instead of owning a webview api of its own.
  assert.match(workflowView, /<div class="workflow-chat-pane">/)
  assert.doesNotMatch(workflowView, /ChatHubPane/)
  assert.match(chatHubBlock(), /const dockGeometry = \(\) => \{[\s\S]*?getBoundingClientRect\(\)/)
  assert.match(chatHubBlock(), /\? \{ x: rect\.x, y: rect\.y, width: rect\.width, height: rect\.height \}/)
  assert.match(workflowView, /dockResizeObserver = new ResizeObserver\(onDockLayoutResize\)/)
  // The rectangle is only trusted once it is laid out; an unlaid surface reports no bounds,
  // which is what keeps a hidden view hidden.
  assert.match(chatHubBlock(), /rect && rect\.width > 0 && rect\.height > 0\s*\?\s*\{/)
  assert.doesNotMatch(workflowView, /bounds-change/)
  assert.doesNotMatch(workflowView, /chat-hub-visible/)
  assert.doesNotMatch(workflowView, /show_chat_hub_webview|update_chat_hub_webview_bounds/)
})

test('the dock is measured only after the window makes room and the DOM update', () => {
  assert.match(
    chatHubBlock(),
    /const syncDock = async \(\) => \{\s*if \(dockTornDown\) \{\s*return\s*\}\s*await syncDockWidth\(\)\s*await nextTick\(\)\s*if \(dockTornDown\) \{\s*return\s*\}\s*return dockCoordinator\.sync\(dockSnapshot\(\)\)\s*\}/
  )
  // A layout burst collapses into one pending sync, so a resize reported several times does
  // not run one snapshot per callback.
  assert.match(
    chatHubBlock(),
    /const onDockLayoutResize = \(\) => \{\s*if \(dockLayoutSyncPending\) \{\s*return\s*\}\s*dockLayoutSyncPending = true\s*nextTick\(\(\) => \{\s*dockLayoutSyncPending = false\s*if \(!dockNativeActive\.value\) \{\s*return\s*\}\s*void syncDock\(\)\s*\}\)\s*\}/
  )
  assert.match(workflowView, /let dockLayoutSyncPending = false/)
  // The coordinator refuses to show without a usable rectangle, so a layout change of a
  // hidden dock cannot bring a view back.
  assert.match(dockedViews, /const showable = \(tab, geometry\) =>/)
  assert.match(dockedViews, /if \(!showable\(active, snapshot\.geometry\)\) \{/)
})

test('the window makes room for the dock column instead of the workflow UI', () => {
  const syncWidth = handlerBody('syncDockWidth')
  const releaseWidth = chatHubBlock()

  // The width the workflow UI reserves is the width the window grows by, so the workflow UI keeps
  // its own size and a native view is placed in the column the window added for it.
  assert.match(
    syncWidth,
    /await invokeWrapper\('set_dock_width', \{ width: chatHubReservedWidth\.value \}\)/
  )
  // A window that cannot grow is reported, while the dock still opens and the layout still syncs.
  assert.match(
    syncWidth,
    /catch \(error\) \{\s*console\.error\('Failed to make room for the docked views:', error\)/
  )
  // The width the window took for the dock is handed back when the dock goes away.
  assert.match(
    releaseWidth,
    /const releaseDockWidth = \(\) =>\s*invokeWrapper\('set_dock_width', \{ width: 0 \}\)\.catch\(error => \{\s*console\.error\('Failed to hand the docked width back to the window:', error\)\s*\}\)/
  )
  assert.match(workflowView, /dockTornDown = true[\s\S]*?void releaseDockWidth\(\)/)
  // The frontend keeps reserving the column, so the dock never becomes part of the workflow
  // content and the overlaid dialogs stay clear of a native view.
  assert.match(
    chatHubBlock(),
    /const chatHubReservedWidth = computed\(\(\) =>\s*dockVisible\.value && dockActiveTab\.value \? chatHubStore\.pageWidth : 0\s*\)/
  )
  assert.match(chatHubBlock(), /root\.style\.setProperty\('--cs-chathub-reserved-width'/)
})

test('the right dock hosts every provider kind in one multi-tab surface', () => {
  assert.match(
    workflowView,
    /<DockedViews\s+ref="dockRef"[\s\S]*?:visible="dockVisible"[\s\S]*?:width="chatHubStore\.pageWidth"[\s\S]*?:tabs="dockTabViews"[\s\S]*?:active-tab-id="dockActiveTabId"[\s\S]*?>/
  )
  // Every tab stays visible so its close control is available even when it is the only tab.
  assert.match(dockedComponent, /<div v-if="tabs\.length > 0" class="docked-views__tabs" role="tablist">/)
  assert.match(dockedComponent, /class="docked-views__tab-label"[\s\S]*?@click="\$emit\('select', tab\.id\)"/)
  // Only a refresh action: no add button and no global close.
  assert.match(dockedComponent, /class="docked-views__action"[\s\S]*?:aria-label="\$t\('common\.refresh'\)"[\s\S]*?@click="\$emit\('reload'\)"/)
  assert.doesNotMatch(dockedComponent, /workflow\.plugin\.newTab|workflow\.plugin\.hide/)
  assert.doesNotMatch(dockedComponent, /<cs name="add"/)
  assert.doesNotMatch(workflowView, /onNewPluginTab|plugin-panel/)
})

test('the tab close control is a sibling button, never nested in the tab button', () => {
  // The close button opens after the tab button has already closed, so both remain
  // independently reachable by pointer and by keyboard.
  assert.match(
    dockedComponent,
    /class="docked-views__tab-label"[\s\S]*?<\/button>[\s\S]*?<button[\s\S]*?class="docked-views__tab-close"[\s\S]*?:aria-label="\$t\('workflow\.plugin\.close'\)"[\s\S]*?@click="\$emit\('close', tab\.id\)"/
  )
  assert.match(dockedComponent, /role="tab"[\s\S]*?:aria-selected="tab\.id === activeTabId"/)
  // The strip scrolls instead of pushing the toolbar out of the dock.
  assert.match(dockedComponent, /&__tabs \{[\s\S]*?overflow-x: auto;/)
})

test('showing a chat entry only drives the one shared coordinator', () => {
  // Every native command of every provider goes through one serial, latest-intent
  // coordinator (see src/libs/dockedViews.js), so a late reply of one provider can never
  // paint over a newer action of the other.
  assert.match(workflowView, /import \{ createChatHubProvider \} from '@\/libs\/chatHubView'/)
  assert.match(
    workflowView,
    /import \{\s*createDockedViewsCoordinator,\s*hydrateDockLayout,\s*serializeDockLayout\s*\} from '@\/libs\/dockedViews'/
  )
  assert.match(chatHubBlock(), /const dockCoordinator = createDockedViewsCoordinator\(\{[\s\S]*?chatHub: chatHubProvider,[\s\S]*?plugin: pluginProvider,/)
  assert.match(chatHubBlock(), /const chatHubProvider = createChatHubProvider\(\{ invoke: invokeWrapper \}\)/)

  // The command payloads live in the provider module, which forwards the tab id and the
  // measured bounds while keeping the existing carrier fields.
  assert.match(
    chatHubView,
    /show: \(tabId, payload = \{\}\) =>\s*call\('show_chat_hub_page', \{\s*url: payload\.url,\s*width: payload\.width,\s*topInset: payload\.topInset,\s*cornerRadius: payload\.cornerRadius,\s*tabId,\s*bounds: payload\.bounds\s*\}\)/
  )
  assert.match(chatHubView, /hide: \(\) => call\('hide_chat_hub_page'\)/)
  assert.match(chatHubView, /destroy: tabId => call\('destroy_chat_hub_page', \{ tabId \}\)/)
  assert.match(chatHubView, /destroyAll: \(\) => call\('destroy_chat_hub_page'\)/)
  assert.match(chatHubView, /reload: tabId => call\('reload_chat_hub_page', \{ tabId \}\)/)

  // No second fire-and-forget call site may bypass the coordinator.
  assert.equal(occurrences(workflowView, "invokeWrapper('show_chat_hub_page'"), 0)
  assert.equal(occurrences(workflowView, "invokeWrapper('hide_chat_hub_page'"), 0)
  assert.equal(occurrences(workflowView, "invokeWrapper('destroy_chat_hub_page'"), 0)
  assert.equal(occurrences(workflowView, "invokeWrapper('reload_chat_hub_page'"), 0)

  // Failures only report a message and keep the original workflow UI usable.
  assert.match(chatHubBlock(), /const dockErrorKey = action => \{[\s\S]*?return 'workflow\.chatHub\.showFailed'/)
  assert.match(
    chatHubBlock(),
    /onError: \(error, action\) => \{[\s\S]*?const key = dockErrorKey\(action\)[\s\S]*?showMessage\(t\(key\), 'error'\)/
  )
})

test('one coordinator serializes every provider and awaits the inactive hide', () => {
  assert.match(dockedViews, /const providers = \{ chatHub, plugin, agentSkills \}/)
  assert.match(dockedViews, /const enqueue = task => \{\s*const next = queue\.then\(task, task\)/)
  // The provider that must not stay visible is hidden and awaited before the active one is
  // shown.
  assert.match(
    dockedViews,
    /if \(visible && visible\.kind !== active\.kind\) \{\s*const hidden = await attempt\(`\$\{visible\.kind\}\.hide`, \(\) => hideProvider\(visible\.kind\)\)[\s\S]*?providers\[active\.kind\]\.show\(active\.tabId, payloadFor\(active, snapshot\.geometry\)\)/
  )
  // A newer intent supersedes a queued one.
  assert.match(dockedViews, /if \(mine !== revision\) \{\s*return\s*\}/)
})

test('closing a tab releases exactly that tab and the active close falls back to a neighbour', () => {
  assert.match(
    dockedViews,
    /const closeTab = tab => \{\s*if \(tab\.kind === 'chatHub'\) \{\s*return providers\.chatHub\.destroy\(tab\.tabId\)\s*\}[\s\S]*?return providers\[tab\.kind\]\.close\(tab\.tabId\)\s*\}/
  )
  assert.match(
    chatHubBlock(),
    /const closeDockTab = tabId => \{[\s\S]*?const neighbor = dockTabs\.value\[index\] \|\| dockTabs\.value\[index - 1\] \|\| null[\s\S]*?dockActiveTabId\.value = neighbor\.id/
  )
  assert.match(
    chatHubBlock(),
    /if \(neighbor\) \{[\s\S]*?\} else \{\s*dockActiveTabId\.value = ''\s*dockVisible\.value = false\s*\}/
  )
  // Hiding preserves every tab and its session.
  assert.match(chatHubBlock(), /const toggleDock = \(\) => \{\s*if \(dockVisible\.value\) \{\s*dockVisible\.value = false\s*return syncDock\(\)/)
})

test('each chat entry owns one tab and a repeated click reuses it', () => {
  assert.match(chatHubBlock(), /const chatHubTabId = hubId => `chathub:\$\{hubId\}`/)
  assert.match(
    chatHubBlock(),
    /const openDockTab = tab => \{\s*if \(!dockTabs\.value\.some\(item => item\.id === tab\.id\)\) \{\s*dockTabs\.value\.push\(tab\)\s*\}\s*return activateDockTab\(tab\.id\)\s*\}/
  )
  assert.match(
    chatHubBlock(),
    /const onSelectChatHubEntry = hub => \{[\s\S]*?return openDockTab\(\{ id: chatHubTabId\(hub\.id\), kind: 'chatHub', hubId: hub\.id \}\)/
  )
})

test('the dock names ChatHub tabs from their source and ordinary plugins from their id', () => {
  assert.match(chatHubBlock(), /return hubOf\(tab\.hubId\)\?\.name \|\| ''/)
  assert.match(chatHubBlock(), /return tab\.pluginId/)
})

test('the trusted skills surface opens as a native dock tab like every other provider', () => {
  // The dock itself stays a plain placeholder; the trusted surface is a native child view.
  assert.doesNotMatch(dockedComponent, /AgentSkills|trustedActive|trustedKey/)
  assert.doesNotMatch(dockedComponent, /iframe|src=|https?:|file:/)
  const launch = chatHubBlock().match(/if \(plugin\.id === AGENT_SKILLS_PLUGIN_ID\) \{([\s\S]*?)\n  \}/)?.[1]
  assert.ok(launch, 'Agent Skills has its own launcher branch')
  // It joins the dock as its own kind instead of opening a second window.
  assert.match(launch, /return openDockTab\(\{[\s\S]*?kind: AGENT_SKILLS_TAB_KIND,\s*pluginId: plugin\.id/)
  assert.doesNotMatch(launch, /invokeWrapper|chatHubStore|toggleDock|dockCoordinator/)
  assert.doesNotMatch(chatHubBlock(), /open_agent_skills_window|trustedReloadKey|kind: 'trusted'/)
})

test('the Vue dock clips its corner without requiring a native tab to open first', async () => {
  const { descriptor } = parse(dockedComponent)
  const style = descriptor.styles.find(block => block.lang === 'scss')
  assert.ok(style, 'the dock has an SCSS style block')
  const compiled = await compileStyleAsync({
    source: style.content,
    filename: 'DockedViews.vue',
    id: 'data-v-dock-corner-test',
    scoped: true,
    preprocessLang: 'scss'
  })
  assert.deepEqual(compiled.errors, [])
  assert.match(compiled.code, /\.docked-views\[data-v-dock-corner-test\]\s*\{[^}]*overflow: hidden;/)
  for (const platform of ['linux', 'macos']) {
    assert.match(
      compiled.code,
      new RegExp(`\\.app-container\\.${platform} \\.docked-views\\[data-v-dock-corner-test\\][^{]*\\{[^}]*border-bottom-right-radius: var\\(--cs-border-radius-lg\\);`)
    )
  }
  assert.doesNotMatch(compiled.code, /\.app-container\.windows[^}]*border-bottom-right-radius/)
})

test('the splitter owns the width and clamps it with the limits the backend enforces', () => {
  assert.match(
    workflowView,
    /<ChatHubSplitter\s+v-if="dockVisible"[\s\S]*?:right="chatHubReservedWidth"[\s\S]*?:width="chatHubStore\.pageWidth"[\s\S]*?:min-width="chatHubStore\.pageMinWidth"[\s\S]*?:min-host-width="chatHubStore\.pageMinHostWidth"[\s\S]*?@resize="onChatHubPageResize"/
  )
  assert.match(
    chatHubBlock(),
    /const onChatHubPageResize = width => \{\s*chatHubStore\.setPageWidth\(width\)\s*return syncDock\(\)\s*\}/
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

test('the reserved space always matches the dock and no bespoke layout style is used', () => {
  assert.match(
    chatHubBlock(),
    /const chatHubReservedWidth = computed\(\(\) =>\s*dockVisible\.value && dockActiveTab\.value \? chatHubStore\.pageWidth : 0\s*\)/
  )
  // The reserved width is published on the document root, because the overlays and popovers
  // Element Plus teleports to the document body have to find it outside this component.
  assert.match(
    chatHubBlock(),
    /watchEffect\(\(\) => \{\s*const reserved = chatHubReservedWidth\.value[\s\S]*?root\.style\.setProperty\('--cs-chathub-reserved-width'[\s\S]*?root\.style\.removeProperty\('--cs-chathub-reserved-width'\)/
  )
  assert.doesNotMatch(workflowView, /chatHubLayoutStyle/)
  // Every platform reserves the same way: the carrier no longer decides between a split
  // and a stacked layout on this side.
  assert.doesNotMatch(chatHubBlock(), /chatHubStore\.viewMode/)
  assert.match(workflowView, /<div class="workflow-layout">/)
  // The dock starts below the app titlebar, so the reserved space narrows the workflow
  // content only and the titlebar keeps the full window width.
  assert.match(
    layoutStyles,
    /\.workflow-main \{[\s\S]*?margin-right: var\(--cs-chathub-reserved-width, 0px\)/
  )
  assert.match(chatHubBlock(), /getPropertyValue\('--cs-titlebar-height'\)/)
  assert.match(chatHubBlock(), /topInset: chatHubTopInset\(\)/)
  // The same carrier also paints over the rounded window border at its bottom-right
  // corner, so the view reports the radius the window container actually draws. A
  // platform whose window keeps square corners reports nothing and the page stays
  // rectangular there.
  assert.match(chatHubBlock(), /getComputedStyle\(container\)\.borderBottomRightRadius/)
  assert.match(chatHubBlock(), /cornerRadius: chatHubCornerRadius\(\)/)
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

test('the ChatHub entry keeps the most recently docked ChatHub visible while another dock tab is active', () => {
  assert.match(
    chatHubBlock(),
    /const activeChatHubId = computed\(\(\) =>[\s\S]*?dockActiveTab\.value\?\.kind === 'chatHub'[\s\S]*?return \[\.\.\.dockTabs\.value\]\.reverse\(\)\.find\(tab => tab\.kind === 'chatHub'\)\?\.hubId \|\| 0/
  )
})

test('the entry of the shown site toggles the dock and the entry list releases its tab', () => {
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
  assert.match(chatHubBlock(), /const onChatHubEntryToggled = \(\) => toggleDock\(\)/)
  // The cross must not come back as a second close action on the icon; the icon keeps a
  // hover highlight so it still reads as clickable.
  assert.doesNotMatch(entry, /current-close/)
  assert.match(
    entry,
    /\.chat-hub-entry__current-surface \{[\s\S]*?&:hover \{\s*background-color: var\(--cs-hover-bg-color\);\s*\}/
  )

  // Releasing the visible chat tab stays in the entry list, which is the only close action
  // for a single tab.
  assert.match(entry, /const CLOSE_COMMAND = 'close'/)
  assert.match(entry, /:command="CLOSE_COMMAND"/)
  assert.match(entry, /if \(command === CLOSE_COMMAND\) \{\s*emit\('close'\)\s*return\s*\}/)
  assert.match(entry, /\$t\('workflow\.chatHub\.close'\)/)
  assert.match(workflowView, /@close="onCloseChatHub"/)
  assert.match(sidebar, /@close="\$emit\('close-chat-hub'\)"/)
  assert.match(workflowView, /@close-chat-hub="onCloseChatHub"/)
  // Closing the shown chat tab releases exactly that tab through the shared coordinator.
  assert.match(
    chatHubBlock(),
    /const onCloseChatHub = \(\) => \{[\s\S]*?item\.kind === 'chatHub' && item\.hubId === activeChatHubId\.value[\s\S]*?return tab \? closeDockTab\(tab\.id\) : Promise\.resolve\(\)/
  )
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
  // A new window request is handed to the platform browser from the Rust side instead of being
  // given a window of its own, which is still no IPC: the page stays a plain wry webview.
  assert.match(page, /open_in_browser\(&opener, &url\)/)
  assert.match(page, /matches!\(url\.split\(':'\)\.next\(\), Some\("http"\) \| Some\("https"\)\)/)
  assert.doesNotMatch(workflowView, /show_chat_hub_webview/)
})

test('switching workflow views keeps the docked views in place', () => {
  // The dock is docked next to the workflow UI, so tasks, automations, sidebar tabs, the
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

  // The one view change that still drops a tab is a deleted entry, which clears its tab.
  assert.match(
    workflowView,
    /watch\(\s*\(\) => chatHubStore\.list\.map\(hub => hub\.id\)\.join\(','\),\s*\(\) => \{[\s\S]*?!hubOf\(tab\.hubId\)[\s\S]*?void closeDockTab\(tab\.id\)/
  )

  const block = chatHubBlock()
  // Hiding or showing a docked view must never send a workflow runtime signal.
  assert.doesNotMatch(block, /emitWorkflowSignal|sendSignal|SIGNAL_TYPES/)
  assert.doesNotMatch(block, /stopWorkflow|clearContext|resumeWorkflow|selectWorkflow\(/)
})

test('tasks, automations, sidebar tabs and the path button no longer hide the dock', () => {
  for (const name of [
    'openWorkflowSidebarTab',
    'onTitlebarPrimaryPathClick',
    'onSelectWorkflowFromHistory',
    'onSelectAutomation'
  ]) {
    assert.doesNotMatch(
      handlerBody(name),
      /closeDockTab|toggleDock|dockCoordinator/,
      `${name} must not touch the docked views`
    )
  }

  // The rail and the compact sidebar entries only open the terminal, so they are no longer
  // hide regions for the page.
  assert.doesNotMatch(workflowView, /class="workflow-side-rail__terminal-entry"[^>]*@click/)
  assert.match(workflowView, /class="workflow-side-rail__terminal-entry">/)
  assert.doesNotMatch(sidebar, /class="compact-terminal-entry-group"[^>]*@click/)
  assert.match(sidebar, /class="compact-terminal-entry-group">/)
  assert.doesNotMatch(sidebar, /hide-chat-hub/)
})

test('the workflow-side dock view state is not persisted into workflow settings', () => {
  assert.match(workflowView, /import \{ useChatHubStore \} from '@\/stores\/chatHub'/)
  assert.doesNotMatch(workflowView, /settings\.chatHub/)
  assert.match(chatHubStore, /defineStore\('chat_hub'/)
})

test('the dock layout of one window is remembered in that window alone', () => {
  const block = chatHubBlock()

  // The layout is presentation state of one window, so it is keyed by the window label and kept in
  // that window's session storage instead of the workflow settings.
  assert.match(
    block,
    /const dockStorageKey = `chatspeed\.workflow\.dockLayout:\$\{settingStore\.windowLabel\}`/
  )
  // The helpers stay pure data: only the workflow view owns storage access.
  assert.doesNotMatch(dockedViews, /localStorage|sessionStorage/)

  // Every change is remembered synchronously, so a hard reload finds the same dock again.
  assert.match(
    block,
    /watch\(\[dockTabs, dockActiveTabId, dockVisible\], persistDockLayout, \{\s*deep: true,\s*flush: 'sync'\s*\}\)/
  )
  assert.match(block, /if \(dockTornDown \|\| dockHydrating\) \{\s*return\s*\}/)

  // Only the identities plus the active and visible state are written.
  assert.match(
    block,
    /serializeDockLayout\(\{\s*tabs: dockTabs\.value,\s*activeTabId: dockActiveTabId\.value,\s*visible: dockVisible\.value\s*\}\)/
  )
  assert.match(
    dockedViews,
    /export function serializeDockLayout\(\{ tabs = \[\], activeTabId = '', visible = false \} = \{\}\)/
  )
  // The remembered descriptor keeps no runtime-verified value: no url, no entry, no token.
  const descriptor = section(
    dockedViews,
    'const descriptorOf = tab => {',
    'The remembered form of the current dock layout'
  )
  assert.match(descriptor, /\{ kind: 'chatHub', hubId: tab\.hubId \}/)
  assert.match(descriptor, /\{ kind: 'plugin', pluginId: tab\.pluginId \}/)
  assert.doesNotMatch(descriptor, /url|entry|token/)
})

test('a saved dock is applied only once the inventories that own its tabs answered', () => {
  const block = chatHubBlock()

  // A pristine empty dock before a restore must never erase the layout that is still to come back.
  assert.match(
    block,
    /if \(!dockLayoutSettled && !dockTabs\.value\.length && !dockVisible\.value\) \{\s*return\s*\}/
  )
  assert.match(block, /dockLayoutSettled = true/)

  // Every stored tab is validated again against the inventories that are authoritative now.
  assert.match(block, /const restored = hydrateDockLayout\(raw, \{/)
  assert.match(
    dockedViews,
    /export function hydrateDockLayout\(raw, \{ hubs = \[\], plugins = \[\] \} = \{\}\)/
  )
  assert.match(block, /hubs: chatHubStore\.list,\s*plugins: pluginUiPlugins\.value/)

  // Until both inventories answered the restore is deferred, and a teardown stops it entirely.
  assert.match(
    block,
    /const maybeRestoreDockLayout = \(\) => \{\s*if \(dockLayoutSettled \|\| dockTornDown\) \{\s*return\s*\}\s*if \(!dockHubsReady \|\| !pluginStore\.inventory \|\| pluginStore\.lastError\) \{\s*return\s*\}\s*restoreDockLayout\(\)\s*\}/
  )
  assert.match(workflowView, /dockHubsReady = true/)
  assert.match(workflowView, /maybeRestoreDockLayout\(\)\n/)
  // The plugin inventory answers asynchronously, so its refresh is the other restore trigger.
  assert.match(
    handlerBody('refreshPlugins'),
    /await pluginStore\.loadInventory\(\)[\s\S]*?maybeRestoreDockLayout\(\)/
  )
  // A stored layout that no longer applies is dropped instead of being retried on every reload.
  assert.match(block, /sessionStorage\.removeItem\(dockStorageKey\)/)

  // The native views are rebuilt by the existing serial coordinator, which owns their ordering.
  assert.match(handlerBody('restoreDockLayout'), /void syncDock\(\)/)
})

test('restoring the dock never touches the workflow runtime and never replays a url', () => {
  const restore = handlerBody('restoreDockLayout')

  for (const body of [restore, handlerBody('persistDockLayout')]) {
    assert.doesNotMatch(body, /emitWorkflowSignal|sendSignal|SIGNAL_TYPES/)
    assert.doesNotMatch(body, /stopWorkflow|clearContext|resumeWorkflow|selectWorkflow\(/)
  }
  // A chat page url comes from the live hub list on restore, never from storage.
  assert.doesNotMatch(restore, /url|entry/)

  // A remembered tab answers to the same id a live click gives it.
  assert.match(chatHubBlock(), /const chatHubTabId = hubId => `chathub:\$\{hubId\}`/)
  assert.match(chatHubBlock(), /const pluginTabId = pluginId => `plugin:\$\{pluginId\}`/)
  assert.match(dockedViews, /const CHAT_HUB_TAB_PREFIX = 'chathub:'/)
  assert.match(dockedViews, /const PLUGIN_TAB_PREFIX = 'plugin:'/)
  assert.match(chatHubBlock(), /const AGENT_SKILLS_TAB_KIND = 'agentSkills'/)
  assert.match(dockedViews, /export const DOCK_AGENT_SKILLS_KIND = 'agentSkills'/)

  // Unmounting still releases every native view after the persisted state stopped being written.
  assert.match(
    workflowView,
    /onBeforeUnmount\(\(\) => \{[\s\S]*?dockTornDown = true[\s\S]*?void dockCoordinator\.dispose\(\)/
  )
})