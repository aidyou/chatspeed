/**
 * Agent Skills provider for the shared dock coordinator, plus the filter that decides whether the
 * trusted surface may be offered at all.
 *
 * Agent Skills is the built-in, trusted management surface. It is a native child webview docked
 * into the right dock like a ChatHub page or a plugin panel, but unlike a plugin panel it renders
 * the application's own `/agent-skills` route instead of a plugin asset bundle. This provider only
 * maps a dock command to the carrier command; the ordering and latest-intent rules stay in
 * `src/libs/dockedViews.js`.
 */
import { verifiedPluginUis } from './pluginView.js'

/** The built-in plugin whose trusted surface the docked view renders. */
export const AGENT_SKILLS_PLUGIN_ID = 'agent-skills'

/**
 * Whether a runtime snapshot still confirms the built-in Agent Skills surface.
 *
 * The decision is the same one the backend and the child page make: an inventory the runtime did
 * not confirm (or one that reported an error) keeps nothing, and a plugin that is no longer enabled
 * with a verified UI no longer qualifies. It fails closed, so an unconfirmed snapshot never offers
 * the surface.
 */
export function agentSkillsVerified(inventory, unavailable = false) {
  return verifiedPluginUis(inventory, unavailable).some(
    plugin => plugin.id === AGENT_SKILLS_PLUGIN_ID
  )
}

export function createAgentSkillsProvider({ invoke } = {}) {
  const call = (command, args) =>
    typeof invoke === 'function' ? invoke(command, args) : Promise.resolve()

  return {
    /** Shows or re-places the surface at the measured rectangle. */
    show: (tabId, payload = {}) =>
      call('show_agent_skills_view', {
        bounds: payload.bounds,
        cornerRadius: payload.cornerRadius ?? 0
      }),
    /** Hides the surface while keeping its browsing session alive. */
    hide: () => call('hide_agent_skills_view'),
    /** Releases the surface. The Workflow window it is docked into is never touched. */
    close: () => call('close_agent_skills_view')
  }
}