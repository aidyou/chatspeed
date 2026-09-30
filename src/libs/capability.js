/**
 * Dependency-free helpers shared by the Agent Skills capability UI.
 *
 * They are kept out of the Pinia store so the contract (error codes, verdict
 * gating, idempotency keys, outcome projection) can be unit-tested with
 * `node --test` without a renderer or a Tauri runtime.
 */

/**
 * Parses the capability error envelope (`{code, message}`) into an `Error`
 * carrying `code`, so the page branches on the stable machine code instead of
 * matching a message string (AC-13).
 *
 * @param {unknown} error
 * @returns {Error & {code: string}}
 */
/**
 * Indexes the capability MCP projection by record id.
 *
 * The legacy `list_mcp_servers` wire remains the source of the editable record,
 * while the projection adds what only the capability service can say. Indexing
 * by id rather than name keeps the merge correct across a rename.
 *
 * @param {Array<Object>|null|undefined} views
 * @returns {Record<number, Object>}
 */
export function mcpViewIndex(views) {
  const index = {}
  for (const view of views ?? []) {
    if (view && typeof view.id === 'number') index[view.id] = view
  }
  return index
}

/**
 * What one MCP row may honestly report.
 *
 * `runtime.observed` distinguishes "the runtime answered stopped" from "nobody
 * looked", which the cached legacy status field cannot. A missing view degrades
 * to the caller's fallback instead of inventing a state.
 *
 * @param {Object|null|undefined} view
 * @returns {{state: string, observed: boolean, drift: string|null, toolsFreshness: string|null, toolCount: number|null, observedAtMs: number|null}}
 */
export function mcpDisplayState(view) {
  if (!view) {
    return {
      state: 'unknown',
      observed: false,
      drift: null,
      toolsFreshness: null,
      toolCount: null,
      observedAtMs: null
    }
  }
  const observed = view.runtime?.observed === true
  return {
    state: observed ? view.runtime.state || 'unknown' : 'unobserved',
    observed,
    drift: view.drift ?? null,
    toolsFreshness: view.tools?.freshness ?? null,
    toolCount: typeof view.tools?.count === 'number' ? view.tools.count : null,
    // The wire is canonical snake_case; this helper is the boundary that
    // converts it once for the Vue side.
    observedAtMs: view.runtime?.observed_at_ms ?? null
  }
}


export function parseCapabilityError(error) {
  const raw = typeof error === 'string' ? error : JSON.stringify(error);
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.code === 'string') {
      const wrapped = new Error(parsed.message || parsed.code);
      wrapped.code = parsed.code;
      return wrapped;
    }
  } catch {
    // Not a capability envelope; fall through to a generic error.
  }
  const wrapped = new Error(String(raw));
  wrapped.code = 'unknown';
  return wrapped;
}

/**
 * Mints an idempotency key so a retried click cannot double an effect (AC-2).
 *
 * @param {string} [prefix]
 * @param {() => number} [now]
 */
export function newIdempotencyKey(prefix = 'ui', now = () => Date.now()) {
  const random = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${now().toString(36)}-${random}`;
}

/**
 * The per-target outcomes of a mutation result.
 *
 * An install records them under `install.outcomes`, an uninstall directly under
 * `outcomes`; both shapes are normalized here so the page never has to guess.
 *
 * @param {{result?: Record<string, unknown>}|null|undefined} mutation
 * @returns {Array<Record<string, unknown>>}
 */
export function mutationOutcomes(mutation) {
  const result = mutation?.result;
  if (!result || typeof result !== 'object') return [];
  if (Array.isArray(result.outcomes)) return result.outcomes;
  const install = result.install;
  if (install && Array.isArray(install.outcomes)) return install.outcomes;
  return [];
}

/**
 * Whether a check report authorizes an install.
 *
 * Only an explicit `pass` does: `blocked` and `inconclusive` both refuse, and a
 * missing report never authorizes (INV-4).
 *
 * @param {{verdict?: string}|null|undefined} report
 */
export function verdictAllowsInstall(report) {
  return report?.verdict === 'pass';
}

/**
 * The default target selection: the ChatSpeed directory only.
 *
 * External tools are never selected implicitly (AC-3/INV-5), so the default is
 * derived from the registry's own `default_selected` flags and intersects with
 * what the backend reports as supported.
 *
 * @param {Array<{id: string, supported: boolean, default_selected: boolean}>} targets
 * @returns {Array<string>}
 */
export function defaultTargetSelection(targets) {
  return (targets ?? [])
    .filter(target => target?.supported && target?.default_selected)
    .map(target => target.id);
}
