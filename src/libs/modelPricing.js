const PRICE_SCALE = 1000000

const toFiniteNumber = value => {
  const num = Number(value)
  return Number.isFinite(num) ? num : 0
}

const hasConfiguredPricing = pricing => {
  if (!pricing || typeof pricing !== 'object') return false
  return [pricing.inputPerMillion, pricing.outputPerMillion, pricing.cachePerMillion].some(
    value => Number(value) > 0
  )
}

const setPricingMapEntry = (map, key, pricing, configured) => {
  if (!key) return
  const current = map.get(key)
  if (!current || (!current.configured && configured)) {
    map.set(key, { pricing, configured })
  }
}

export const createDefaultPricing = () => ({
  inputPerMillion: 0,
  outputPerMillion: 0,
  cachePerMillion: 0,
  reasoningPerMillion: null,
  cacheWritePerMillion: 0,
  audioInputPerMillion: 0,
  audioOutputPerMillion: 0,
  multiplier: 1,
  pricingSource: null,
  reasoningPricingMode: 'output',
  tiers: []
})

const normalizePricingTier = tier => {
  if (!tier || typeof tier !== 'object') return null
  const contextSize = Math.max(0, Math.trunc(toFiniteNumber(tier.contextSize ?? tier.context_size)))
  if (!contextSize) return null
  return {
    contextSize,
    inputPerMillion: Math.max(0, toFiniteNumber(tier.inputPerMillion ?? tier.input_per_million)),
    outputPerMillion: Math.max(0, toFiniteNumber(tier.outputPerMillion ?? tier.output_per_million)),
    cachePerMillion: Math.max(0, toFiniteNumber(tier.cachePerMillion ?? tier.cache_per_million)),
    reasoningPerMillion:
      tier.reasoningPerMillion == null && tier.reasoning_per_million == null
        ? null
        : Math.max(0, toFiniteNumber(tier.reasoningPerMillion ?? tier.reasoning_per_million)),
    cacheWritePerMillion: Math.max(0, toFiniteNumber(tier.cacheWritePerMillion ?? tier.cache_write_per_million)),
    audioInputPerMillion: Math.max(0, toFiniteNumber(tier.audioInputPerMillion ?? tier.audio_input_per_million)),
    audioOutputPerMillion: Math.max(0, toFiniteNumber(tier.audioOutputPerMillion ?? tier.audio_output_per_million))
  }
}

const normalizePricingTiers = tiers =>
  (Array.isArray(tiers) ? tiers : [])
    .map(normalizePricingTier)
    .filter(Boolean)
    .sort((left, right) => left.contextSize - right.contextSize)

export const normalizePricing = pricing => ({
  inputPerMillion: Math.max(0, toFiniteNumber(pricing?.inputPerMillion)),
  outputPerMillion: Math.max(0, toFiniteNumber(pricing?.outputPerMillion)),
  cachePerMillion: Math.max(0, toFiniteNumber(pricing?.cachePerMillion)),
  reasoningPerMillion:
    pricing?.reasoningPerMillion == null
      ? null
      : Math.max(0, toFiniteNumber(pricing.reasoningPerMillion)),
  cacheWritePerMillion: Math.max(0, toFiniteNumber(pricing?.cacheWritePerMillion)),
  audioInputPerMillion: Math.max(0, toFiniteNumber(pricing?.audioInputPerMillion)),
  audioOutputPerMillion: Math.max(0, toFiniteNumber(pricing?.audioOutputPerMillion)),
  multiplier: Math.max(0, toFiniteNumber(pricing?.multiplier) || 1),
  pricingSource: pricing?.pricingSource ?? pricing?.source ?? null,
  reasoningPricingMode:
    pricing?.reasoningPricingMode === 'separate' ? 'separate' : 'output',
  tiers: normalizePricingTiers(pricing?.tiers)
})

export const buildPricingMaps = providers => {
  const byProviderId = new Map()
  const byProviderName = new Map()

  ;(providers || []).forEach(provider => {
    const providerId = String(provider?.id ?? '')
    const providerName = provider?.name || ''
    ;(provider?.models || []).forEach(model => {
      const configured = hasConfiguredPricing(model?.pricing)
      const pricing = normalizePricing(model?.pricing)
      const modelId = model?.id || ''
      const modelName = model?.name || ''

      if (providerId && modelId) {
        setPricingMapEntry(byProviderId, `${providerId}::${modelId}`, pricing, configured)
      }
      if (providerName && modelId) {
        setPricingMapEntry(byProviderName, `${providerName}::${modelId}`, pricing, configured)
      }
      if (providerName && modelName) {
        setPricingMapEntry(byProviderName, `${providerName}::${modelName}`, pricing, configured)
      }
    })
  })

  return { byProviderId, byProviderName }
}

export const findPricingForUsageRow = (pricingMaps, row) => {
  const providerId = String(row?.providerId ?? '').trim()
  const provider = row?.provider || ''
  const backendModel = row?.backendModel || ''
  return (
    (providerId
      ? pricingMaps?.byProviderId?.get(`${providerId}::${backendModel}`)?.pricing
      : null) ||
    pricingMaps?.byProviderName?.get(`${provider}::${backendModel}`)?.pricing ||
    createDefaultPricing()
  )
}

export const estimateCostFromPricing = (
  usage,
  pricing = createDefaultPricing()
) => {
  const normalizedPricing = normalizePricing(pricing)
  const inputTokens = Math.max(0, toFiniteNumber(usage?.inputTokens))
  const outputTokens = Math.max(0, toFiniteNumber(usage?.outputTokens))
  const cacheTokens = Math.max(0, toFiniteNumber(usage?.cacheTokens))
  const cacheWriteTokens = Math.max(0, toFiniteNumber(usage?.cacheWriteTokens))
  const reasoningTokens = Math.max(0, toFiniteNumber(usage?.reasoningTokens))
  const audioInputTokens = Math.max(0, toFiniteNumber(usage?.audioInputTokens))
  const audioOutputTokens = Math.max(0, toFiniteNumber(usage?.audioOutputTokens))
  const billableInputTokens = Math.max(0, inputTokens - cacheTokens - cacheWriteTokens - audioInputTokens)
  const billableOutputTokens = Math.max(0, outputTokens - reasoningTokens - audioOutputTokens)
  const reasoningRate =
    normalizedPricing.reasoningPricingMode === 'separate' && normalizedPricing.reasoningPerMillion != null
      ? normalizedPricing.reasoningPerMillion
      : normalizedPricing.outputPerMillion
  const audioOutputRate =
    normalizedPricing.audioOutputPerMillion || normalizedPricing.outputPerMillion
  const cacheWriteRate =
    normalizedPricing.cacheWritePerMillion || normalizedPricing.inputPerMillion
  const audioInputRate =
    normalizedPricing.audioInputPerMillion || normalizedPricing.inputPerMillion

  return (
    (billableInputTokens * normalizedPricing.inputPerMillion +
      cacheTokens * normalizedPricing.cachePerMillion +
      cacheWriteTokens * cacheWriteRate +
      audioInputTokens * audioInputRate) / PRICE_SCALE +
    (billableOutputTokens * normalizedPricing.outputPerMillion +
      reasoningTokens * reasoningRate +
      audioOutputTokens * audioOutputRate) / PRICE_SCALE
  ) * normalizedPricing.multiplier
}

export const formatCurrency = value => {
  const num = toFiniteNumber(value)
  return `$${num.toFixed(num >= 100 ? 2 : 4)}`
}

export const formatCurrencyCompact = value => {
  const num = toFiniteNumber(value)
  if (num == 0) {
    return '$0.00';
  }
  if (num >= 1000) return `$${(num / 1000).toFixed(2)}K`
  if (num >= 1) return `$${num.toFixed(2)}`
  if (num >= 0.01) return `$${num.toFixed(4)}`
  return `$${num.toFixed(6)}`
}
