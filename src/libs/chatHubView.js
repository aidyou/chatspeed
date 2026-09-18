/**
 * Ordered command boundary for the docked ChatHub page.
 *
 * `show`, `hide`, `setWidth` and `destroy` cross the Tauri IPC boundary and complete
 * asynchronously, so a pair of fire-and-forget calls can finish out of order and leave
 * the page in a state the user never asked for. The clearest example is a late `hide`
 * resolving after a newer `show`: the page stays hidden while the view layer believes
 * it is visible. Two racing `show` calls are the second case: the older entry can end
 * up on screen.
 *
 * Every command therefore goes through one serial queue, and each applied state is
 * only committed when the request revision it belongs to is still the latest one, so
 * an older completion can never override the newest user intent.
 */
export function createChatHubViewController({
  show,
  hide,
  destroy,
  setWidth,
  getWidth,
  onVisibleChange,
  onError
} = {}) {
  let queue = Promise.resolve()
  let revision = 0
  let desiredVisible = false
  let desiredUrl = ''
  let lastUrl = ''
  let appliedVisible = false
  let appliedUrl = ''

  const commit = (visible, url) => {
    if (appliedVisible === visible && appliedUrl === url) {
      return
    }
    appliedVisible = visible
    appliedUrl = url
    if (typeof onVisibleChange === 'function') {
      onVisibleChange(visible, url)
    }
  }

  const fail = (action, error) => {
    if (typeof onError === 'function') {
      onError(error, action)
    }
  }

  /**
   * Appends a command to the single queue.
   *
   * The queue itself never rejects, so one failing command cannot break the
   * ordering of the commands requested afterwards.
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
   * The width the page should use, read when a command runs so a burst of splitter
   * drags collapses into the newest width instead of replaying every step.
   */
  const currentWidth = () => (typeof getWidth === 'function' ? getWidth() : null)

  /**
   * Applies the desired visibility/url as the next queued task.
   */
  const applyDesired = () => {
    const mine = revision
    const wantVisible = desiredVisible
    const url = desiredUrl

    return enqueue(async () => {
      // A newer request already superseded this one, so the newer task owns the
      // outcome: applying this state would only cause a visible flicker.
      if (mine !== revision) {
        return
      }

      if (!wantVisible || !url) {
        try {
          await hide()
        } catch (error) {
          fail('hide', error)
        }
        if (mine === revision) {
          commit(false, '')
        }
        return
      }

      const width = currentWidth()
      if (!width) {
        // Without a usable width the page cannot be docked, so it stays hidden and
        // only the workflow UI is shown.
        if (mine === revision) {
          commit(false, '')
        }
        return
      }

      try {
        await show(url, width)
      } catch (error) {
        fail('show', error)
        if (mine === revision) {
          commit(false, '')
        }
        return
      }

      // A newer request may have arrived while this one was in flight; only the
      // newest request decides what the user sees.
      if (mine === revision) {
        commit(true, url)
      }
    })
  }

  /**
   * Shows the given entry url. The newest call always wins.
   */
  const select = url => {
    if (!url) {
      return Promise.resolve()
    }
    revision += 1
    desiredVisible = true
    desiredUrl = url
    lastUrl = url
    return applyDesired()
  }

  /**
   * Re-shows the entry that was shown last, used when the chat entry is opened
   * again after the page was hidden by a task, automation or terminal click.
   *
   * A restore for the entry that is already desired dedupes to nothing, while a
   * restore that arrives while a hide is still queued supersedes that hide: the
   * newest intent wins even though the applied state has not caught up yet.
   */
  const restore = url => {
    const target = url || lastUrl
    if (!target) {
      return Promise.resolve()
    }
    if (desiredVisible && desiredUrl === target) {
      return Promise.resolve()
    }
    return select(target)
  }

  /**
   * Hides the page while keeping its session alive.
   */
  const hideNow = () => {
    revision += 1
    desiredVisible = false
    desiredUrl = ''
    return applyDesired()
  }

  /**
   * Actively closes the page: hide first, then release the embedded webview.
   */
  const close = () => {
    hideNow()
    return enqueue(async () => {
      try {
        await destroy()
      } catch (error) {
        fail('destroy', error)
      }
      commit(false, '')
    })
  }

  /**
   * Applies the current width to a visible page, used while the splitter is dragged.
   *
   * A hidden page is never shown by a width change, and the width is read when the
   * command runs, so the queue only ever applies the newest one.
   */
  const resize = () =>
    enqueue(async () => {
      if (!appliedVisible || !appliedUrl) {
        return
      }
      const width = currentWidth()
      if (!width) {
        return
      }
      try {
        await setWidth(width)
      } catch (error) {
        fail('width', error)
      }
    })

  return {
    select,
    restore,
    hide: hideNow,
    close,
    resize,
    isVisible: () => appliedVisible,
    /** Resolves when every queued command has settled. Serialization helper. */
    settled: () => queue
  }
}

/**
 * Chat-entry open action: bring the active entry back into view.
 *
 * This is the entry point used by the Workflow sidebar, and it deliberately does
 * not consult the last applied visibility. That flag only flips when the queued
 * native command finished, so a hide requested a moment earlier (terminal, task,
 * automation or sidebar tab) still reports the page as visible; a guard on it would
 * drop the newest user intent and leave the page hidden. Submitting the restore
 * instead lets the controller supersede the pending hide and de-dupe the case where
 * the page is already the desired one.
 */
export function restoreChatHubEntry(controller, hub) {
  if (!controller || !hub || !hub.url) {
    return Promise.resolve()
  }
  return controller.restore(hub.url)
}
