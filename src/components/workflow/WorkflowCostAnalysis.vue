<template>
  <section class="workflow-cost-analysis">
    <div class="workflow-cost-analysis__summary">
      <span class="workflow-cost-analysis__summary-item">
        {{ formatDuration(summary.durationMs) }}
      </span>
      <span class="workflow-cost-analysis__summary-item">
        {{ t('workflow.costAnalysis.tokens', { count: formatTokens(summary.withSubAgents.totalTokens) }) }}
      </span>
    </div>

    <div class="workflow-cost-analysis__totals">
      <div class="workflow-cost-analysis__row workflow-cost-analysis__row--primary">
        <span class="workflow-cost-analysis__row-label">{{ t('workflow.costAnalysis.self') }}</span>
        <strong class="workflow-cost-analysis__row-cost">{{ formatCost(summary.selfUsage.estimatedCost) }}</strong>
        <span class="workflow-cost-analysis__row-rate">{{ formatRate(summary.selfUsage.effectiveCostPerMillion) }}</span>
      </div>
      <div v-if="summary.hasSubAgents" class="workflow-cost-analysis__row">
        <span class="workflow-cost-analysis__row-label">{{ t('workflow.costAnalysis.withSubAgents') }}</span>
        <strong class="workflow-cost-analysis__row-cost">{{ formatCost(summary.withSubAgents.estimatedCost) }}</strong>
        <span class="workflow-cost-analysis__row-rate">{{ formatRate(summary.withSubAgents.effectiveCostPerMillion) }}</span>
      </div>
    </div>

    <div v-if="summary.modelBreakdowns.length" class="workflow-cost-analysis__models">
      <div v-for="model in summary.modelBreakdowns" :key="modelKey(model)" class="workflow-cost-analysis__model">
        <div class="workflow-cost-analysis__model-header">
          <span class="workflow-cost-analysis__model-name">{{ model.backendModel }}</span>
          <strong class="workflow-cost-analysis__model-cost">{{ formatCost(model.estimatedCost) }}</strong>
        </div>
        <div class="workflow-cost-analysis__model-tokens">
          <span>{{ t('workflow.costAnalysis.input') }} {{ formatTokens(model.inputTokens) }}</span>
          <span>{{ t('workflow.costAnalysis.cache') }} {{ formatTokens(model.cacheTokens) }}</span>
          <span>{{ t('workflow.costAnalysis.output') }} {{ formatTokens(model.outputTokens) }}</span>
          <span v-if="model.cacheHitRate !== null">
            {{ t('workflow.costAnalysis.cacheHitRate') }} {{ formatPercent(model.cacheHitRate) }}
          </span>
        </div>
        <div class="workflow-cost-analysis__model-rates">
          <span>{{ t('workflow.costAnalysis.input') }} {{ formatRate(model.inputPerMillion) }}</span>
          <span>{{ t('workflow.costAnalysis.cache') }} {{ formatRate(model.cachePerMillion) }}</span>
          <span>{{ t('workflow.costAnalysis.output') }} {{ formatRate(model.outputPerMillion) }}</span>
          <span v-if="model.multiplier !== null">×{{ model.multiplier }}</span>
        </div>
      </div>
    </div>
    <p v-if="summary.isPartial" class="workflow-cost-analysis__warning">
      {{ t('workflow.costAnalysis.partial') }}
    </p>
  </section>
</template>

<script setup>
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
defineProps({ summary: { type: Object, required: true } })
const formatTokens = value => new Intl.NumberFormat().format(value || 0)
const formatPercent = value =>
  `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value)}%`
const modelKey = model => `${model.providerId ?? 'unknown'}:${model.backendModel}`
const formatCost = value =>
  value === null || value === undefined ? t('workflow.costAnalysis.unpriced') : `$${value.toFixed(6)}`
const formatRate = value =>
  value === null || value === undefined
    ? t('workflow.costAnalysis.unpricedRate')
    : t('workflow.costAnalysis.ratePerMillion', { rate: `$${value.toFixed(4)}` })
const formatDuration = value => {
  if (!Number.isFinite(value) || value < 0) return t('workflow.costAnalysis.unknownDuration')
  const seconds = Math.floor(value / 1000)
  return t('workflow.costAnalysis.duration', {
    minutes: Math.floor(seconds / 60),
    seconds: seconds % 60
  })
}
</script>
