import assert from 'node:assert/strict'
import test from 'node:test'

import {
  buildPricingMaps,
  estimateCostFromPricing,
  findPricingForUsageRow
} from './modelPricing.js'

test('matches pricing by provider id and backend model', () => {
  const pricingMaps = buildPricingMaps([
    {
      id: 7,
      name: 'Provider A',
      models: [
        {
          id: 'model-a',
          name: 'Model A',
          pricing: {
            inputPerMillion: 2,
            outputPerMillion: 8,
            cachePerMillion: 0.2,
            multiplier: 1.5
          }
        }
      ]
    }
  ])

  assert.deepEqual(
    findPricingForUsageRow(pricingMaps, {
      providerId: 7,
      provider: 'Renamed Provider',
      backendModel: 'model-a'
    }),
    {
      inputPerMillion: 2,
      outputPerMillion: 8,
      cachePerMillion: 0.2,
      multiplier: 1.5
    }
  )
})

test('charges cached input once at the configured cache price', () => {
  const cost = estimateCostFromPricing(
    {
      inputTokens: 1_500_000,
      outputTokens: 250_000,
      cacheTokens: 500_000
    },
    {
      inputPerMillion: 2,
      outputPerMillion: 8,
      cachePerMillion: 0.2,
      multiplier: 1.5
    }
  )

  assert.ok(Math.abs(cost - 6.15) < Number.EPSILON * 8)
})
