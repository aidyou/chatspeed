/**
 * ChatHub provider for the shared dock coordinator.
 *
 * The provider only translates a dock command into the ChatHub command payload, so the
 * ordering and latest-intent rules live in one place (`src/libs/dockedViews.js`) instead of
 * here. The page geometry is measured by the dock and passed in: `bounds` is the rectangle
 * the native page has to cover, while `width`, `topInset` and `cornerRadius` are the
 * existing carrier hints the backend still receives.
 *
 * Each tab is a page of its own, keyed by a stable `tabId`, so the carrier keeps one
 * browsing session per chat entry.
 */
export function createChatHubProvider({ invoke } = {}) {
  const call = (command, args) =>
    typeof invoke === 'function' ? invoke(command, args) : Promise.resolve()

  return {
    /** Shows, re-places or navigates the page of one tab. */
    show: (tabId, payload = {}) =>
      call('show_chat_hub_page', {
        url: payload.url,
        width: payload.width,
        topInset: payload.topInset,
        cornerRadius: payload.cornerRadius,
        tabId,
        bounds: payload.bounds
      }),
    /** Hides every ChatHub page while keeping its session alive. */
    hide: () => call('hide_chat_hub_page'),
    /** Releases exactly one page, with its browsing session. */
    destroy: tabId => call('destroy_chat_hub_page', { tabId }),
    /** Releases every page. */
    destroyAll: () => call('destroy_chat_hub_page'),
    /** Reloads the page the tab currently shows, keeping its url. */
    reload: tabId => call('reload_chat_hub_page', { tabId })
  }
}