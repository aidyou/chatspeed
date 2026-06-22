const PRICE_SCALE = 1000000

const toFiniteNumber = value => {
  const num = Number(value)
  return Number.isFinite(num) ? num : 0
}

export const createDefaultPricing = () => ({
  inputPerMillion: 0,
  outputPerMillion: 0,
  cachePerMillion: 0
})

export const normalizePricing = pricing => ({
  inputPerMillion: Math.max(0, toFiniteNumber(pricing?.inputPerMillion)),
  outputPerMillion: Math.max(0, toFiniteNumber(pricing?.outputPerMillion)),
  cachePerMillion: Math.max(0, toFiniteNumber(pricing?.cachePerMillion))
})

export const buildPricingMaps = providers => {
  const byProviderId = new Map()
  const byProviderName = new Map()

  ;(providers || []).forEach(provider => {
    const providerId = String(provider?.id ?? '')
    const providerName = provider?.name || ''
    ;(provider?.models || []).forEach(model => {
      const pricing = normalizePricing(model?.pricing)
      const modelId = model?.id || ''
      if (!modelId) return
      if (providerId) {
        byProviderId.set(`${providerId}::${modelId}`, pricing)
      }
      if (providerName) {
        byProviderName.set(`${providerName}::${modelId}`, pricing)
      }
    })
  })

  return { byProviderId, byProviderName }
}

export const estimateCostFromPricing = (
  usage,
  pricing = createDefaultPricing()
) => {
  const normalizedPricing = normalizePricing(pricing)
  const inputTokens = Math.max(0, toFiniteNumber(usage?.inputTokens))
  const outputTokens = Math.max(0, toFiniteNumber(usage?.outputTokens))
  const cacheTokens = Math.max(0, toFiniteNumber(usage?.cacheTokens))

  return (
    (inputTokens * normalizedPricing.inputPerMillion) / PRICE_SCALE +
    (outputTokens * normalizedPricing.outputPerMillion) / PRICE_SCALE +
    (cacheTokens * normalizedPricing.cachePerMillion) / PRICE_SCALE
  )
}

export const formatCurrency = value => {
  const num = toFiniteNumber(value)
  return `$${num.toFixed(num >= 100 ? 2 : 4)}`
}

export const formatCurrencyCompact = value => {
  const num = toFiniteNumber(value)
  if (num >= 1000) return `$${(num / 1000).toFixed(2)}K`
  if (num >= 1) return `$${num.toFixed(2)}`
  if (num >= 0.01) return `$${num.toFixed(4)}`
  return `$${num.toFixed(6)}`
}
