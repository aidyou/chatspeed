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
  multiplier: 1
})

export const normalizePricing = pricing => ({
  inputPerMillion: Math.max(0, toFiniteNumber(pricing?.inputPerMillion)),
  outputPerMillion: Math.max(0, toFiniteNumber(pricing?.outputPerMillion)),
  cachePerMillion: Math.max(0, toFiniteNumber(pricing?.cachePerMillion)),
  multiplier: Math.max(0, toFiniteNumber(pricing?.multiplier) || 1)
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
  const billableInputTokens = Math.max(0, inputTokens - cacheTokens)

  return (
    (billableInputTokens * normalizedPricing.inputPerMillion) / PRICE_SCALE +
    (outputTokens * normalizedPricing.outputPerMillion) / PRICE_SCALE +
    (cacheTokens * normalizedPricing.cachePerMillion) / PRICE_SCALE
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
