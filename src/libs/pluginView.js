/**
 * Plugin provider for the shared dock coordinator.
 *
 * Like the ChatHub provider, this only maps a dock command to the plugin host command;
 * ordering is owned by `src/libs/dockedViews.js`. The plugin host proves `pluginId` and
 * `entry` against its runtime inventory and never trusts a frontend url, so a plugin page
 * keeps the isolation the host gives it.
 */
export function createPluginProvider({ invoke } = {}) {
  const call = (command, args) =>
    typeof invoke === 'function' ? invoke(command, args) : Promise.resolve()

  return {
    /** Shows, re-places or replaces the panel of one tab at the measured rectangle. */
    show: (tabId, payload = {}) =>
      call('plugin_ui_open', {
        tabId,
        pluginId: payload.pluginId,
        entry: payload.entry,
        bounds: payload.bounds
      }),
    /** Hides every panel while keeping its tab session alive. */
    hide: () => call('plugin_ui_hide'),
    /** Releases exactly one panel and revokes its capability. */
    close: tabId => call('plugin_ui_close', { tabId }),
    /** Releases every panel and revokes every capability. */
    clear: () => call('plugin_ui_clear')
  }
}

/**
 * The plugin UIs the dock may open: built-in bundles the runtime verified and left enabled.
 *
 * The filter fails closed: a snapshot the runtime did not confirm, or one taken while the
 * runtime reported an error, yields nothing so no unverified page can ever be opened.
 */
export function verifiedPluginUis(inventory, unavailable = false) {
  if (unavailable || inventory?.schema_version !== 1 || !Array.isArray(inventory.plugins)) {
    return []
  }
  return inventory.plugins.filter(
    plugin => plugin.kind === 'builtin' && plugin.state === 'enabled' && plugin.ui?.verified === true
  )
}