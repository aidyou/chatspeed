/**
 * One serial, latest-intent coordinator for every native view docked in the right dock.
 *
 * ChatHub pages and plugin panels are two different native providers (a wry webview the
 * carrier builds for a chat site, a plugin host panel built from a capability grant), but
 * they share the same right dock and therefore the same screen rectangle. Letting each
 * provider run its own queue would let a slow `hide` of one provider resolve after the
 * `show` of the other and leave the two native views stacked on top of each other.
 *
 * Every operation therefore enters one queue, and the queue applies only the newest dock
 * snapshot:
 *
 * - a newer intent always supersedes a queued one, so a burst of clicks, a drag or a
 *   resize burst collapses to the final state instead of replaying every step;
 * - the provider that must not stay visible is hidden and awaited before the active one
 *   is shown, so the dock never paints two native views at once;
 * - a tab that left the dock is closed by its own id, while the tabs that are only hidden
 *   keep their session, so hiding the dock preserves the ChatHub and plugin sessions;
 * - a snapshot that resolves to nothing shown only hides the provider that is actually
 *   visible, so a resize of a hidden dock never resurrects a view.
 *
 * The providers are injected, so the coordinator itself is free of Tauri imports and can
 * be exercised by the node tests.
 *
 * The snapshot shape is:
 *
 * ```
 * {
 *   tabs: [{ kind: 'chatHub' | 'plugin' | 'trusted', tabId, url?, pluginId?, entry? }],
 *   activeTabId: string,          // the visible tab, empty when the dock is hidden
 *   geometry: { width, topInset, cornerRadius, bounds: {x,y,width,height} | null }
 * }
 * ```
 *
 * A `trusted` tab is host-native Vue content (it has no native webview), so it is treated
 * like "nothing native is visible": both providers are hidden while it is active.
 */
export function createDockedViewsCoordinator({ chatHub, plugin, onError } = {}) {
  const providers = { chatHub, plugin }

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

  const closeTab = tab =>
    tab.kind === 'chatHub'
      ? providers.chatHub.destroy(tab.tabId)
      : providers.plugin.close(tab.tabId)

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

  const payloadFor = (tab, geometry) =>
    tab.kind === 'chatHub'
      ? {
          url: tab.url,
          width: geometry.width,
          topInset: geometry.topInset,
          cornerRadius: geometry.cornerRadius,
          bounds: geometry.bounds
        }
      : {
          pluginId: tab.pluginId,
          entry: tab.entry,
          bounds: geometry.bounds,
          cornerRadius: geometry.cornerRadius
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
      } else {
        const shown = await attempt(`${active.kind}.show`, () =>
          providers[active.kind].show(active.tabId, payloadFor(active, snapshot.geometry))
        )
        if (!shown) {
          dirty = true
          appliedActiveSignature = ''
          if (visible && visible.tabId === active.tabId && visible.kind === active.kind) {
            visible = null
          }
        } else {
          openTabs.set(keyOf(active.kind, active.tabId), { kind: active.kind, tabId: active.tabId })
          appliedActiveSignature = activeSig
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
   * A ChatHub tab is reloaded in place (the site keeps its current url). A plugin tab
   * cannot be reloaded in place: its capability grant has to be renewed, so the live panel
   * is released first and the same logical tab is opened again through this same queue.
   * A trusted tab has no native view and is remounted by the caller instead.
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

      if (tab.kind === 'chatHub') {
        await attempt('chatHub.reload', () => providers.chatHub.reload(tabId))
        return
      }

      await attempt('plugin.close', () => providers.plugin.close(tabId))
      openTabs.delete(keyOf('plugin', tabId))
      if (visible && visible.tabId === tabId) {
        visible = null
      }
      appliedActiveSignature = ''
      appliedSignature = ''

      if (snapshot.activeTabId !== tabId || !showable(tab, snapshot.geometry)) {
        return
      }
      const reopened = await attempt('plugin.show', () =>
        providers.plugin.show(tabId, payloadFor(tab, snapshot.geometry))
      )
      if (!reopened) {
        return
      }
      openTabs.set(keyOf('plugin', tabId), { kind: 'plugin', tabId })
      visible = { kind: 'plugin', tabId }
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
      openTabs.clear()
      visible = null
      appliedActiveSignature = ''
      appliedSignature = ''
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