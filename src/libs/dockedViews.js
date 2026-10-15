/**
 * One serial, latest-intent coordinator for every native view docked in the right dock.
 *
 * ChatHub pages, plugin panels and the trusted Agent Skills surface are three different native
 * providers (a wry webview the carrier builds for a chat site, a plugin host panel built from a
 * capability grant, and the application's own `/agent-skills` page in a Tauri child webview), but
 * they share the same right dock and therefore the same screen rectangle. Letting each provider run
 * its own queue would let a slow `hide` of one provider resolve after the `show` of the other and
 * leave the two native views stacked on top of each other.
 *
 * Every operation therefore enters one queue, and the queue applies only the newest dock
 * snapshot:
 *
 * - a newer intent always supersedes a queued one, so a burst of clicks, a drag or a
 *   resize burst collapses to the final state instead of replaying every step;
 * - the provider that must not stay visible is hidden and awaited before the active one
 *   is shown, so the dock never paints two native views at once;
 * - a tab that left the dock is closed by its own id, while the tabs that are only hidden
 *   keep their session, so hiding the dock preserves the ChatHub, plugin and Agent Skills
 *   sessions;
 * - a snapshot that resolves to nothing shown only hides the provider that is actually
 *   visible, so a resize of a hidden dock never resurrects a view;
 * - a `show` that failed for the visible tab is recorded together with its rectangle, so the
 *   layout passes that follow the same snapshot neither re-issue the failing command nor
 *   report the same error again, while another tab, another rectangle or an explicit reload
 *   still retries.
 *
 * The providers are injected, so the coordinator itself is free of Tauri imports and can
 * be exercised by the node tests.
 *
 * The snapshot shape is:
 *
 * ```
 * {
 *   tabs: [{ kind: 'chatHub' | 'plugin' | 'agentSkills' | 'trusted', tabId, url?, pluginId?, entry? }],
 *   activeTabId: string,          // the visible tab, empty when the dock is hidden
 *   geometry: { width, topInset, cornerRadius, bounds: {x,y,width,height} | null }
 * }
 * ```
 *
 * A `trusted` tab is host-native Vue content (it has no native webview), so it is treated
 * like "nothing native is visible": every provider is hidden while it is active.
 */
import { AGENT_SKILLS_PLUGIN_ID } from './agentSkillsView.js'

// ----------------------------------------------------------------------------
// Dock layout persistence
//
// The dock layout of one window survives a page reload through session storage: the tabs
// that were docked, which one was visible and whether the dock was on screen. Only the
// identity of a tab is stored, because a chat page url, a plugin entry and a capability
// token are all values the runtime verifies, so they are re-verified on restore instead of
// being replayed from storage. The coordinator above owns the native commands; these two
// helpers own the shape that is remembered and only touch plain data.
// ----------------------------------------------------------------------------

/** Storage shape of one remembered dock layout; a layout of another version is ignored. */
export const DOCK_LAYOUT_VERSION = 1
/**
 * Most ordinary plugin panels one remembered layout may restore. The dock and the plugin host
 * enforce the same limit, so a stored layout can never open more panels than a live click could.
 */
export const DOCK_MAX_PLUGIN_TABS = 8
/** Kind and tab id of the one trusted Agent Skills surface a remembered layout may hold. */
export const DOCK_AGENT_SKILLS_KIND = 'agentSkills'

// Tab id prefixes, shared with the workflow view so a restored tab answers to the same id a
// live click would give it.
const CHAT_HUB_TAB_PREFIX = 'chathub:'
const PLUGIN_TAB_PREFIX = 'plugin:'

/**
 * The remembered identity of one dock tab, or null for a tab the dock never restores.
 *
 * A `trusted` tab is host-native Vue content with no native view to rebuild, so it is not
 * remembered at all.
 */
const descriptorOf = tab => {
  if (!tab || typeof tab !== 'object') {
    return null
  }
  if (tab.kind === 'chatHub') {
    return typeof tab.hubId === 'number' ? { kind: 'chatHub', hubId: tab.hubId } : null
  }
  if (tab.kind === 'plugin') {
    return typeof tab.pluginId === 'string' && tab.pluginId
      ? { kind: 'plugin', pluginId: tab.pluginId }
      : null
  }
  if (tab.kind === DOCK_AGENT_SKILLS_KIND) {
    return { kind: DOCK_AGENT_SKILLS_KIND }
  }
  return null
}

/**
 * The remembered form of the current dock layout: the identity of every restorable tab, the
 * index of the visible one, and whether the dock was on screen.
 *
 * A url, a plugin entry and a capability are runtime-verified values, so none of them is
 * written: the restore re-reads them from the inventories that are authoritative then.
 */
export function serializeDockLayout({ tabs = [], activeTabId = '', visible = false } = {}) {
  const descriptors = []
  let activeIndex = -1
  for (const tab of Array.isArray(tabs) ? tabs : []) {
    const descriptor = descriptorOf(tab)
    if (!descriptor) {
      continue
    }
    if (tab.id === activeTabId) {
      activeIndex = descriptors.length
    }
    descriptors.push(descriptor)
  }
  return JSON.stringify({
    version: DOCK_LAYOUT_VERSION,
    visible: visible === true,
    activeIndex,
    tabs: descriptors
  })
}

/** Parses a remembered layout that may be a string, already parsed, or unusable. */
const parseStoredLayout = raw => {
  if (raw && typeof raw === 'object') {
    return raw
  }
  if (typeof raw !== 'string' || !raw) {
    return null
  }
  try {
    const parsed = JSON.parse(raw)
    return parsed && typeof parsed === 'object' ? parsed : null
  } catch {
    return null
  }
}

/**
 * Rebuilds dock tabs from a remembered layout, validated against the inventories that are
 * authoritative right now.
 *
 * Every stored tab has to be confirmed again: a chat hub id must still exist and an ordinary
 * plugin must still be offered by the runtime, from which the entry is taken rather than from
 * storage. The trusted Agent Skills surface is only restored from the runtime confirming its
 * plugin, never from the kind or the plugin id a stored layout claims, so a stored kind can
 * never grant trust the runtime did not give. Duplicates are dropped, ordinary plugin panels
 * are capped at the dock's own limit, and a stored layout that is malformed or points at
 * nothing the runtime still offers yields an empty dock.
 *
 * When the stored active tab did not survive, the nearest surviving neighbour takes its place:
 * the first tab after it, otherwise the last one before it. A dock that was hidden stays
 * hidden.
 */
export function hydrateDockLayout(raw, { hubs = [], plugins = [] } = {}) {
  const stored = parseStoredLayout(raw)
  if (!stored || stored.version !== DOCK_LAYOUT_VERSION || !Array.isArray(stored.tabs)) {
    return emptyLayout()
  }

  const hubIds = new Map((Array.isArray(hubs) ? hubs : []).map(hub => [String(hub?.id), hub?.id]))
  const pluginEntries = new Map(
    (Array.isArray(plugins) ? plugins : []).map(plugin => [plugin?.id, plugin?.ui?.entry || ''])
  )
  const trustedAvailable = pluginEntries.has(AGENT_SKILLS_PLUGIN_ID)

  const tabs = []
  const keptIds = new Set()
  // Original index of each surviving tab, so a dropped active tab still finds its neighbour.
  const survivors = []
  let pluginCount = 0

  stored.tabs.forEach((descriptor, originalIndex) => {
    if (!descriptor || typeof descriptor !== 'object') {
      return
    }
    const tab = restoreTab(descriptor, { hubIds, pluginEntries, trustedAvailable, pluginCount })
    if (!tab) {
      return
    }
    // A duplicate is dropped before it counts against the plugin limit, so a stored layout
    // that repeats one panel cannot push a distinct panel out of the dock.
    if (keptIds.has(tab.id)) {
      return
    }
    keptIds.add(tab.id)
    if (tab.kind === 'plugin') {
      pluginCount += 1
    }
    tabs.push(tab)
    survivors.push(originalIndex)
  })

  if (!tabs.length) {
    return emptyLayout()
  }

  const storedIndex = Number.isInteger(stored.activeIndex) ? stored.activeIndex : -1
  const activeTabId = activeTabFor(tabs, survivors, storedIndex)
  return { tabs, activeTabId, visible: stored.visible === true }
}

const emptyLayout = () => ({ tabs: [], activeTabId: '', visible: false })

/**
 * One validated dock tab from a stored descriptor, or null when the runtime does not confirm it.
 */
const restoreTab = (descriptor, { hubIds, pluginEntries, trustedAvailable, pluginCount }) => {
  if (descriptor.kind === 'chatHub') {
    if (typeof descriptor.hubId !== 'number' && typeof descriptor.hubId !== 'string') {
      return null
    }
    // The canonical id comes from the inventory, so a stored string id cannot break lookups.
    if (!hubIds.has(String(descriptor.hubId))) {
      return null
    }
    const hubId = hubIds.get(String(descriptor.hubId))
    return { id: `${CHAT_HUB_TAB_PREFIX}${hubId}`, kind: 'chatHub', hubId }
  }

  if (descriptor.kind === 'plugin') {
    // The trusted plugin never grants trust through the ordinary plugin kind, and an ordinary
    // panel only opens from the entry the runtime verified just now.
    if (typeof descriptor.pluginId !== 'string' || !descriptor.pluginId) {
      return null
    }
    if (descriptor.pluginId === AGENT_SKILLS_PLUGIN_ID || !pluginEntries.has(descriptor.pluginId)) {
      return null
    }
    if (pluginCount >= DOCK_MAX_PLUGIN_TABS) {
      return null
    }
    return {
      id: `${PLUGIN_TAB_PREFIX}${descriptor.pluginId}`,
      kind: 'plugin',
      pluginId: descriptor.pluginId,
      entry: pluginEntries.get(descriptor.pluginId)
    }
  }

  if (descriptor.kind === DOCK_AGENT_SKILLS_KIND) {
    // A stored layout that names another plugin beside the trusted kind is refused outright.
    if (descriptor.pluginId !== undefined && descriptor.pluginId !== AGENT_SKILLS_PLUGIN_ID) {
      return null
    }
    if (!trustedAvailable) {
      return null
    }
    return {
      id: DOCK_AGENT_SKILLS_KIND,
      kind: DOCK_AGENT_SKILLS_KIND,
      pluginId: AGENT_SKILLS_PLUGIN_ID
    }
  }

  return null
}

/** The surviving tab the stored active index points at, or its nearest surviving neighbour. */
const activeTabFor = (tabs, survivors, storedIndex) => {
  const exact = survivors.indexOf(storedIndex)
  if (exact !== -1) {
    return tabs[exact].id
  }
  const next = survivors.findIndex(index => index > storedIndex)
  const position = next !== -1 ? next : survivors.length - 1
  return tabs[position]?.id || ''
}

export function createDockedViewsCoordinator({ chatHub, plugin, agentSkills, onError } = {}) {
  const providers = { chatHub, plugin, agentSkills }

  const reportError = (action, error) => {
    try {
      if (typeof onError === 'function') {
        onError(error, action)
      }
    } catch {
      // An error observer must never break the queue.
    }
  }

  // The single queue every provider operation enters.
  let queue = Promise.resolve()
  // Newest intent revision; a queued reconcile older than this one is dropped.
  let revision = 0
  // Newest dock snapshot the caller asked for.
  let desired = null
  // Signature of the last snapshot this coordinator fully applied.
  let appliedSignature = ''
  // Signature of the currently shown tab, so a snapshot that only changed a background tab
  // does not re-issue the show of an unchanged visible tab.
  let appliedActiveSignature = ''
  // Signature of the visible tab and rectangle whose last show failed. The same snapshot, or
  // one that only changed a background tab, is then left alone instead of re-issuing the same
  // failing command and reporting the same error again for every layout pass. A different
  // active tab or rectangle has a different signature and retries on its own, and a reload or
  // a dock without a native active tab clears the record explicitly.
  let failedActiveSignature = ''
  // Provider and tab currently shown, or null when nothing native is visible.
  let visible = null
  // Native tabs this coordinator opened, keyed by `${kind}:${tabId}`.
  const openTabs = new Map()

  const keyOf = (kind, tabId) => `${kind}:${tabId}`

  /**
   * Appends an operation to the single queue. The queue never rejects, so a failing
   * operation cannot break the ordering of the operations requested afterwards.
   */
  const enqueue = task => {
    const next = queue.then(task, task)
    queue = next.then(
      () => undefined,
      () => undefined
    )
    return next
  }

  /**
   * Runs one provider operation and reports its failure without throwing, so one failed
   * command cannot abort the rest of a reconcile.
   */
  const attempt = async (action, operation) => {
    try {
      await operation()
      return true
    } catch (error) {
      reportError(action, error)
      return false
    }
  }

  const closeTab = tab => {
    if (tab.kind === 'chatHub') {
      return providers.chatHub.destroy(tab.tabId)
    }
    // A plugin panel and the Agent Skills surface are both closed by their own provider.
    return providers[tab.kind].close(tab.tabId)
  }

  const hideProvider = kind => providers[kind].hide()

  /**
   * Stable description of a snapshot. An unchanged snapshot is skipped, which is what
   * keeps a resize that did not change the measured rectangle from re-issuing commands.
   */
  const signatureOf = snapshot => {
    if (!snapshot) {
      return ''
    }
    const geometry = snapshot.geometry || {}
    return JSON.stringify({
      active: snapshot.activeTabId || '',
      width: geometry.width ?? null,
      cornerRadius: geometry.cornerRadius ?? 0,
      bounds: geometry.bounds || null,
      tabs: snapshot.tabs.map(tab => [
        tab.kind,
        tab.tabId,
        tab.url || '',
        tab.pluginId || '',
        tab.entry || ''
      ])
    })
  }

  const payloadFor = (tab, geometry) => {
    if (tab.kind === 'chatHub') {
      return {
        url: tab.url,
        width: geometry.width,
        topInset: geometry.topInset,
        cornerRadius: geometry.cornerRadius,
        bounds: geometry.bounds
      }
    }
    if (tab.kind === 'agentSkills') {
      // The trusted surface always renders the same application route, so it only needs the
      // measured rectangle and the corner radius the window draws.
      return {
        bounds: geometry.bounds,
        cornerRadius: geometry.cornerRadius
      }
    }
    return {
      pluginId: tab.pluginId,
      entry: tab.entry,
      bounds: geometry.bounds,
      cornerRadius: geometry.cornerRadius
    }
  }

  /**
   * Signature of just the visible tab and the geometry. A snapshot that only changed a
   * background tab leaves this unchanged, so the visible tab is not re-shown for nothing.
   */
  const activeSignatureOf = (tab, geometry) =>
    JSON.stringify([
      tab.kind,
      tab.tabId,
      tab.url || '',
      tab.pluginId || '',
      tab.entry || '',
      geometry.width ?? null,
      geometry.cornerRadius ?? 0,
      geometry.bounds || null
    ])

  /**
   * Whether the active tab can actually be shown natively. A trusted tab has no native
   * view, a chat site needs its url, and a hold without a measured rectangle cannot be
   * placed, so all three fall back to "hide everything".
   */
  const showable = (tab, geometry) =>
    !!tab &&
    tab.kind !== 'trusted' &&
    !!geometry &&
    !!geometry.bounds &&
    geometry.bounds.width > 0 &&
    geometry.bounds.height > 0 &&
    (tab.kind !== 'chatHub' || !!tab.url)

  const reconcile = snapshot => {
    const mine = revision
    return enqueue(async () => {
      // A newer snapshot already owns the outcome.
      if (mine !== revision) {
        return
      }

      const signature = signatureOf(snapshot)
      if (signature === appliedSignature) {
        return
      }

      // A tab that no longer exists in the dock releases its exact native view; a failure
      // keeps it tracked so a later snapshot retries instead of leaking a visible panel.
      let dirty = false
      const wanted = new Set(snapshot.tabs.map(tab => keyOf(tab.kind, tab.tabId)))
      for (const [key, tab] of [...openTabs]) {
        if (wanted.has(key)) {
          continue
        }
        const closed = await attempt(`${tab.kind}.close`, () => closeTab(tab))
        if (closed) {
          openTabs.delete(key)
        } else {
          dirty = true
        }
        if (closed && visible && visible.tabId === tab.tabId) {
          visible = null
        }
        if (mine !== revision) {
          return
        }
      }

      const active = snapshot.activeTabId
        ? snapshot.tabs.find(tab => tab.tabId === snapshot.activeTabId) || null
        : null

      if (!showable(active, snapshot.geometry)) {
        if (visible) {
          const hidden = await attempt(`${visible.kind}.hide`, () => hideProvider(visible.kind))
          dirty = dirty || !hidden
          if (mine !== revision) {
            return
          }
          if (hidden) {
            visible = null
          }
        }
        // A dock without a native active tab (a hidden dock, or the host-native trusted tab)
        // holds no standing show intent, so a recorded failure is forgotten and the next
        // activation starts clean. A readable snapshot that merely cannot be placed keeps the
        // record, so a repeated layout pass cannot re-issue the same failing show.
        if (!active || active.kind === 'trusted') {
          failedActiveSignature = ''
        }
        appliedSignature = dirty ? '' : signature
        return
      }

      // The provider that must not stay visible is hidden and awaited before the active
      // one is shown, so the dock never paints both native views at once.
      if (visible && visible.kind !== active.kind) {
        const hidden = await attempt(`${visible.kind}.hide`, () => hideProvider(visible.kind))
        dirty = dirty || !hidden
        if (mine !== revision) {
          return
        }
        if (hidden) {
          visible = null
        }
      }

      // A snapshot that only changed a background tab must not re-show the visible tab: the
      // active signature captures the visible tab and its geometry, so an unchanged one is
      // left alone.
      const activeSig = activeSignatureOf(active, snapshot.geometry)
      const alreadyVisible =
        !!visible &&
        visible.kind === active.kind &&
        visible.tabId === active.tabId &&
        activeSig === appliedActiveSignature

      if (alreadyVisible) {
        openTabs.set(keyOf(active.kind, active.tabId), { kind: active.kind, tabId: active.tabId })
      } else if (activeSig === failedActiveSignature) {
        // The visible tab already failed to open for this exact rectangle. Re-issuing the
        // command would fail the same way and report the same error, so the intent is settled
        // until the active tab, its rectangle or an explicit reload changes the signature.
      } else {
        const shown = await attempt(`${active.kind}.show`, () =>
          providers[active.kind].show(active.tabId, payloadFor(active, snapshot.geometry))
        )
        if (!shown) {
          dirty = true
          appliedActiveSignature = ''
          failedActiveSignature = activeSig
          if (visible && visible.tabId === active.tabId && visible.kind === active.kind) {
            visible = null
          }
        } else {
          openTabs.set(keyOf(active.kind, active.tabId), { kind: active.kind, tabId: active.tabId })
          appliedActiveSignature = activeSig
          failedActiveSignature = ''
          if (mine === revision) {
            visible = { kind: active.kind, tabId: active.tabId }
          }
        }
      }

      if (mine === revision) {
        appliedSignature = dirty ? '' : signature
      }
    })
  }

  /**
   * Requests the newest dock snapshot. The visible tab and the set of live native tabs are
   * reconciled against the previous snapshot, so hiding preserves sessions while a removed
   * tab is released.
   */
  const sync = snapshot => {
    revision += 1
    desired = snapshot
    return reconcile(snapshot)
  }

  /**
   * Reloads one tab.
   *
   * A ChatHub tab is reloaded in place (the site keeps its current url); a page that never
   * opened is opened again instead, since it has no live page to reload. A plugin tab cannot
   * be reloaded in place: its capability grant has to be renewed. The Agent Skills
   * surface is rebuilt from its application route for the same reason, so both release the
   * live view first and open the same logical tab again through this same queue, which keeps
   * the newest intent. A trusted tab has no native view and is remounted by the caller instead.
   */
  const reload = tabId =>
    enqueue(async () => {
      const snapshot = desired
      if (!snapshot) {
        return
      }
      const tab = snapshot.tabs.find(item => item.tabId === tabId)
      if (!tab) {
        return
      }

      if (tab.kind === 'trusted') {
        return
      }

      // A reload is an explicit new intent, so a recorded show failure no longer blocks the
      // tab and the command is allowed to run again.
      failedActiveSignature = ''

      if (tab.kind === 'chatHub') {
        // An in-place reload needs a live page. A page whose show failed has none, so the
        // active tab is opened again instead of reloading nothing.
        if (visible && visible.kind === 'chatHub' && visible.tabId === tabId) {
          await attempt('chatHub.reload', () => providers.chatHub.reload(tabId))
          return
        }
        if (snapshot.activeTabId !== tabId || !showable(tab, snapshot.geometry)) {
          return
        }
        const shown = await attempt('chatHub.show', () =>
          providers.chatHub.show(tabId, payloadFor(tab, snapshot.geometry))
        )
        if (!shown) {
          failedActiveSignature = activeSignatureOf(tab, snapshot.geometry)
          return
        }
        openTabs.set(keyOf(tab.kind, tabId), { kind: tab.kind, tabId })
        visible = { kind: tab.kind, tabId }
        appliedActiveSignature = activeSignatureOf(tab, snapshot.geometry)
        appliedSignature = signatureOf(snapshot)
        return
      }

      const provider = providers[tab.kind]
      await attempt(`${tab.kind}.close`, () => provider.close(tabId))
      openTabs.delete(keyOf(tab.kind, tabId))
      if (visible && visible.tabId === tabId) {
        visible = null
      }
      appliedActiveSignature = ''
      appliedSignature = ''

      if (snapshot.activeTabId !== tabId || !showable(tab, snapshot.geometry)) {
        return
      }
      const reopened = await attempt(`${tab.kind}.show`, () =>
        provider.show(tabId, payloadFor(tab, snapshot.geometry))
      )
      if (!reopened) {
        // The retry failed as well, so the record stands and the layout passes that follow
        // stay quiet instead of reporting the same error for every one of them.
        failedActiveSignature = activeSignatureOf(tab, snapshot.geometry)
        return
      }
      openTabs.set(keyOf(tab.kind, tabId), { kind: tab.kind, tabId })
      visible = { kind: tab.kind, tabId }
      appliedActiveSignature = activeSignatureOf(tab, snapshot.geometry)
      appliedSignature = signatureOf(snapshot)
    })

  /**
   * Releases every native view and forgets the tracked state, used when the workflow view
   * unmounts. The intent is cleared first so no queued reconcile can resurrect a view.
   */
  const dispose = () => {
    revision += 1
    desired = null
    return enqueue(async () => {
      await attempt('chatHub.dispose', () =>
        providers.chatHub.destroyAll ? providers.chatHub.destroyAll() : providers.chatHub.destroy()
      )
      await attempt('plugin.dispose', () =>
        providers.plugin.clear ? providers.plugin.clear() : providers.plugin.hide()
      )
      if (providers.agentSkills) {
        await attempt('agentSkills.dispose', () =>
          providers.agentSkills.close ? providers.agentSkills.close() : providers.agentSkills.hide()
        )
      }
      openTabs.clear()
      visible = null
      appliedActiveSignature = ''
      appliedSignature = ''
      failedActiveSignature = ''
    })
  }

  return {
    sync,
    reload,
    dispose,
    /** Resolves when every queued operation has settled. Serialization helper. */
    settled: () => queue,
    /** The provider and tab currently shown, or null. Diagnostics. */
    applied: () => (visible ? { ...visible } : null),
    /** The native tabs this coordinator opened. Diagnostics. */
    openTabs: () => [...openTabs.values()].map(tab => ({ ...tab }))
  }
}