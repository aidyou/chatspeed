const finiteNumber = value => (Number.isFinite(Number(value)) ? Number(value) : null)

const normalizeTotals = value => {
  if (!value || typeof value !== 'object') return null
  const inputTokens = finiteNumber(value.input_tokens)
  const outputTokens = finiteNumber(value.output_tokens)
  const cacheTokens = finiteNumber(value.cache_tokens)
  const totalTokens = finiteNumber(value.total_tokens)
  if ([inputTokens, outputTokens, cacheTokens, totalTokens].some(item => item === null)) return null
  return {
    inputTokens,
    outputTokens,
    cacheTokens,
    totalTokens,
    estimatedCost: finiteNumber(value.estimated_cost),
    effectiveCostPerMillion: finiteNumber(value.effective_cost_per_million),
    unpricedTokens: finiteNumber(value.unpriced_tokens) || 0
  }
}

const normalizeModelBreakdown = value => {
  if (!value || typeof value !== 'object') return null
  const inputTokens = finiteNumber(value.input_tokens)
  const outputTokens = finiteNumber(value.output_tokens)
  const cacheTokens = finiteNumber(value.cache_tokens)
  if ([inputTokens, outputTokens, cacheTokens].some(item => item === null)) return null
  return {
    providerId: finiteNumber(value.provider_id),
    backendModel: String(value.backend_model || ''),
    inputTokens,
    outputTokens,
    cacheTokens,
    pricingStatus: String(value.pricing_status || 'missing'),
    inputPerMillion: finiteNumber(value.input_per_million),
    outputPerMillion: finiteNumber(value.output_per_million),
    cachePerMillion: finiteNumber(value.cache_per_million),
    multiplier: finiteNumber(value.multiplier),
    estimatedCost: finiteNumber(value.estimated_cost)
  }
}

export const normalizeUsageSummary = value => {
  if (!value || typeof value !== 'object' || Number(value.version) !== 1) return null
  const selfUsage = normalizeTotals(value.self_usage)
  const withSubAgents = normalizeTotals(value.with_sub_agents)
  if (!selfUsage || !withSubAgents) return null
  return {
    terminalStatus: String(value.terminal_status || ''),
    durationMs: finiteNumber(value.duration_ms),
    selfUsage,
    withSubAgents,
    hasSubAgents: value.has_sub_agents === true,
    isPartial: value.is_partial === true,
    modelBreakdowns: Array.isArray(value.model_breakdowns)
      ? value.model_breakdowns.map(normalizeModelBreakdown).filter(Boolean)
      : []
  }
}

export const isTerminalUsageSummary = value => {
  const summary = normalizeUsageSummary(value)
  return !!summary && ['completed', 'failed', 'cancelled', 'interrupted'].includes(summary.terminalStatus)
}
