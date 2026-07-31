import test from 'node:test'
import assert from 'node:assert/strict'
import { isTerminalUsageSummary, normalizeUsageSummary } from './usageSummary.js'

const summary = {
  version: 1,
  terminal_status: 'completed',
  duration_ms: 1200,
  self_usage: {
    input_tokens: 100,
    output_tokens: 20,
    cache_tokens: 40,
    total_tokens: 120,
    estimated_cost: 0.1,
    effective_cost_per_million: 833.333,
    unpriced_tokens: 0
  },
  with_sub_agents: {
    input_tokens: 150,
    output_tokens: 30,
    cache_tokens: 60,
    total_tokens: 180,
    estimated_cost: 0.2,
    effective_cost_per_million: 1111.111,
    unpriced_tokens: 0
  },
  has_sub_agents: true,
  is_partial: false,
  model_breakdowns: [
    {
      provider_id: 1,
      backend_model: 'gpt-test',
      input_tokens: 100,
      output_tokens: 20,
      cache_tokens: 40,
      pricing_status: 'priced',
      input_per_million: 2,
      output_per_million: 8,
      cache_per_million: 0.2,
      multiplier: 1,
      estimated_cost: 0.1
    }
  ]
}

test('normalizes the canonical snake_case usage summary', () => {
  const normalized = normalizeUsageSummary(summary)
  assert.equal(normalized.selfUsage.totalTokens, 120)
  assert.equal(normalized.withSubAgents.estimatedCost, 0.2)
  assert.equal(normalized.modelBreakdowns[0].backendModel, 'gpt-test')
  assert.equal(normalized.modelBreakdowns[0].inputPerMillion, 2)
  assert.equal(normalized.hasSubAgents, true)
})

test('accepts every terminal child state while rejecting running summaries', () => {
  for (const terminalStatus of ['completed', 'failed', 'cancelled', 'interrupted']) {
    assert.equal(isTerminalUsageSummary({ ...summary, terminal_status: terminalStatus }), true)
  }
  assert.equal(isTerminalUsageSummary({ ...summary, terminal_status: 'running' }), false)
})

test('preserves an explicit zero-token sub-agent summary for the combined UI row', () => {
  const normalized = normalizeUsageSummary({
    ...summary,
    self_usage: { ...summary.self_usage, input_tokens: 0, output_tokens: 0, cache_tokens: 0, total_tokens: 0 },
    with_sub_agents: { ...summary.with_sub_agents, input_tokens: 0, output_tokens: 0, cache_tokens: 0, total_tokens: 0 },
    has_sub_agents: true
  })
  assert.equal(normalized.hasSubAgents, true)
  assert.equal(normalized.withSubAgents.totalTokens, 0)
})
