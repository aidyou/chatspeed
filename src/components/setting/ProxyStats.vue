<template>
  <div class="proxy-stats">
    <div class="stats-header">
      <h3>{{ $t('settings.proxy.stats.title') }}</h3>
      <div class="header-actions">
        <el-space>
          <el-select v-model="selectedDays" size="small" @change="fetchDailyStats(false)"
            style="width: 140px; margin-left: 10px">
            <el-option :label="$t('settings.proxy.stats.today')" :value="0" />
            <el-option :label="$t('settings.proxy.stats.last1Day')" :value="1" />
            <el-option :label="$t('settings.proxy.stats.last7Days')" :value="7" />
            <el-option :label="$t('settings.proxy.stats.last30Days')" :value="30" />
            <el-option :label="$t('settings.proxy.stats.last90Days')" :value="90" />
            <el-option :label="$t('settings.proxy.stats.last365Days')" :value="365" />
            <el-option :label="$t('settings.proxy.stats.allTime')" :value="-1" />
          </el-select>
          <el-button :icon="Refresh" circle size="small" @click="fetchDailyStats(false)" :loading="loading" />
          <el-checkbox v-model="autoRefreshEnabled" size="small" style="margin-right: 15px">
            {{ $t('settings.proxy.stats.autoRefresh') }}
          </el-checkbox>
        </el-space>

        <el-dropdown trigger="click" @command="handleDeleteStats" style="margin-left: 10px">
          <el-button :icon="Delete" type="danger" plain size="small" circle />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item :command="7">{{ $t('settings.proxy.stats.deleteOlderThan7Days') }}</el-dropdown-item>
              <el-dropdown-item :command="30">{{ $t('settings.proxy.stats.deleteOlderThan30Days') }}</el-dropdown-item>
              <el-dropdown-item :command="90">{{ $t('settings.proxy.stats.deleteOlderThan90Days') }}</el-dropdown-item>
              <el-dropdown-item :command="365">{{ $t('settings.proxy.stats.deleteOlderThan365Days')
                }}</el-dropdown-item>
              <el-dropdown-item :command="-1" divided style="color: var(--el-color-danger)">{{
                $t('settings.proxy.stats.deleteAll') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>

    <!-- KPI Cards -->
    <div class="kpi-cards">
      <div class="kpi-card">
        <div class="kpi-icon" style="background-color: rgba(64, 158, 255, 0.1); color: #409eff">
          <el-icon>
            <DataLine />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value">{{ formatNumber(kpiData.totalRequests) }}</div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.totalRequests') }}</div>
        </div>
      </div>
      <div class="kpi-card">
        <div class="kpi-icon" style="background-color: rgba(103, 194, 58, 0.1); color: #67c23a">
          <el-icon>
            <Coin />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value">{{ formatTokens(kpiData.totalTokens) }}</div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.totalTokens') }}</div>
        </div>
      </div>
      <div class="kpi-card">
        <div class="kpi-icon" style="background-color: rgba(245, 108, 108, 0.1); color: #f56c6c">
          <el-icon>
            <Warning />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value" :class="{ 'text-danger': kpiData.errorRate > 5 }">
            {{ kpiData.errorRate.toFixed(2) }}%
          </div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.errorRate') }}</div>
        </div>
      </div>
      <div class="kpi-card">
        <div class="kpi-icon" style="background-color: rgba(230, 162, 60, 0.1); color: #e6a23c">
          <el-icon>
            <Collection />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value">{{ kpiData.activeModels }}</div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.activeModels') }}</div>
        </div>
      </div>
    </div>

    <el-table :data="dailyStats" style="width: 100%" v-loading="loading" @expand-change="handleExpandChange"
      row-key="date">
      <el-table-column type="expand" fixed="left">
        <template #default="props">
          <div class="expand-detail">
            <el-table :data="providerStats[props.row.date] || []" size="small" border
              v-loading="providerLoading[props.row.date]">
              <el-table-column prop="provider" :label="$t('settings.proxy.stats.provider')" width="100"
                show-overflow-tooltip />
              <el-table-column prop="clientModel" :label="$t('settings.proxy.stats.clientModel')" min-width="150"
                show-overflow-tooltip sortable>
                <template #default="scope">
                  <span style="color: var(--cs-color-primary); font-weight: bold">{{
                    scope.row.clientModel
                    }}</span>
                </template>
              </el-table-column>
              <el-table-column prop="backendModel" :label="$t('settings.proxy.stats.backendModel')" min-width="200"
                show-overflow-tooltip sortable />
              <el-table-column prop="protocol" :label="$t('settings.proxy.stats.protocol')" width="90">
                <template #default="scope">
                  <el-tag size="small" :color="getProtocolColor(scope.row.protocol)"
                    :style="{ color: getProtocolTextColor(scope.row.protocol), borderColor: getProtocolTextColor(scope.row.protocol) + '50' }">
                    {{ scope.row.protocol }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.toolCompat')" width="90" align="center">
                <template #default="scope">
                  <el-tag :type="scope.row.toolCompatMode === 1 ? 'warning' : 'info'" size="small">
                    {{
                      scope.row.toolCompatMode === 1
                        ? $t('settings.proxy.stats.yes')
                        : $t('settings.proxy.stats.no')
                    }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column prop="requestCount" :label="$t('settings.proxy.stats.requests')" width="100" sortable />
              <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="90" sortable
                sort-by="totalInputTokens">
                <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="90" sortable
                sort-by="totalOutputTokens">
                <template #default="scope">{{
                  formatTokens(scope.row.totalOutputTokens)
                  }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="90" sortable
                sort-by="totalCacheTokens">
                <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.errors')" width="100" sortable sort-by="errorCount">
                <template #default="scope">
                  <el-link v-if="scope.row.errorCount > 0" type="danger"
                    @click="showErrorDetail(props.row.date, scope.row.clientModel, scope.row.backendModel)">
                    {{ scope.row.errorCount }}
                  </el-link>
                  <span v-else>0</span>
                </template>
              </el-table-column>
            </el-table>
          </div>
        </template>
      </el-table-column>
      <el-table-column prop="date" :label="$t('settings.proxy.stats.date')" width="110" />
      <el-table-column prop="providerCount" :label="$t('settings.proxy.stats.providers')" width="90" align="center" />
      <el-table-column prop="topProvider" :label="$t('settings.proxy.stats.topProvider')" min-width="180"
        show-overflow-tooltip />
      <el-table-column prop="totalRequestCount" :label="$t('settings.proxy.stats.requests')" width="100" />
      <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalOutputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
      </el-table-column>
      <el-table-column prop="errorCount" :label="$t('settings.proxy.stats.errors')" width="100" />
    </el-table>

    <div class="charts-section">
      <!-- 1. Trend charts in Tabs (Token first) -->
      <div class="charts-row">
        <div class="chart-card tab-chart">
          <el-tabs v-model="activeTrendTab" type="border-card">
            <el-tab-pane :label="$t('settings.proxy.stats.dailyTokensTitle')" name="dailyTokens">
              <div class="tab-chart-content">
                <div v-if="tokenTrend.length" class="trend-stack-chart">
                  <div v-for="day in tokenTrend" :key="day.date" class="trend-stack-item">
                    <div class="trend-stack-bars">
                      <div
                        v-for="segment in day.segments"
                        :key="segment.type"
                        class="trend-stack-segment"
                        :style="{ height: `${segment.percent}%`, backgroundColor: segment.color }" />
                    </div>
                    <div class="trend-stack-total">{{ formatTokens(day.total) }}</div>
                    <div class="trend-stack-label">{{ day.date }}</div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.dailyRequestsTitle')" name="dailyRequests">
              <div class="tab-chart-content">
                <div v-if="requestsTrend.length" class="trend-metric-list">
                  <div v-for="item in requestsTrend" :key="item.date" class="trend-metric-row">
                    <div class="trend-metric-header">
                      <span>{{ item.date }}</span>
                      <span>{{ formatNumber(item.requests) }} / {{ item.errorRate.toFixed(2) }}%</span>
                    </div>
                    <div class="trend-metric-track">
                      <div
                        class="trend-metric-fill requests"
                        :style="{ width: `${toPercent(item.requests, maxRequestsValue)}%` }" />
                    </div>
                    <div class="trend-metric-track error">
                      <div
                        class="trend-metric-fill errors"
                        :style="{ width: `${toPercent(item.errorRate, maxErrorRateValue)}%` }" />
                    </div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
          </el-tabs>
        </div>
      </div>

      <!-- 2. Distribution charts in Tabs -->
      <div class="charts-row">
        <div class="chart-card tab-chart">
          <el-tabs v-model="activeDistributionTab" type="border-card">
            <el-tab-pane :label="$t('settings.proxy.stats.modelTokenUsageTitle')" name="modelTokenUsage">
              <div class="tab-chart-content">
                <div v-if="modelTokenUsageStats.length" class="rank-bar-list">
                  <div v-for="item in modelTokenUsageStats" :key="item.type" class="rank-bar-row">
                    <div class="rank-bar-header">
                      <span class="rank-bar-label">{{ item.type }}</span>
                      <span class="rank-bar-value">{{ formatTokens(item.value) }}</span>
                    </div>
                    <div class="rank-bar-track">
                      <div
                        class="rank-bar-fill"
                        :style="{ width: `${toPercent(item.value, maxModelTokenUsageValue)}%`, backgroundColor: getSeriesColor(1) }" />
                    </div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.modelUsageTitle')" name="modelUsage">
              <div class="tab-chart-content">
                <div v-if="modelUsageStats.length" class="rank-bar-list">
                  <div v-for="item in modelUsageStats" :key="item.type" class="rank-bar-row">
                    <div class="rank-bar-header">
                      <span class="rank-bar-label">{{ item.type }}</span>
                      <span class="rank-bar-value">{{ formatNumber(item.value) }}</span>
                    </div>
                    <div class="rank-bar-track">
                      <div
                        class="rank-bar-fill"
                        :style="{ width: `${toPercent(item.value, maxModelUsageValue)}%`, backgroundColor: getSeriesColor(0) }" />
                    </div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.providerTokenUsageTitle')" name="providerTokenUsage">
              <div class="tab-chart-content">
                <div v-if="providerTokenUsageStats.length" class="rank-bar-list">
                  <div v-for="item in providerTokenUsageStats" :key="item.type" class="rank-bar-row">
                    <div class="rank-bar-header">
                      <span class="rank-bar-label">{{ item.type }}</span>
                      <span class="rank-bar-value">{{ formatTokens(item.value) }}</span>
                    </div>
                    <div class="rank-bar-track">
                      <div
                        class="rank-bar-fill"
                        :style="{ width: `${toPercent(item.value, maxProviderTokenUsageValue)}%`, backgroundColor: getSeriesColor(2) }" />
                    </div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.errorDistTitle')" name="errorDist">
              <div class="tab-chart-content">
                <div v-if="errorDistributionStats.length" class="rank-bar-list">
                  <div v-for="item in errorDistributionStats" :key="item.type" class="rank-bar-row">
                    <div class="rank-bar-header">
                      <span class="rank-bar-label">{{ item.type }}</span>
                      <span class="rank-bar-value">{{ formatNumber(item.value) }}</span>
                    </div>
                    <div class="rank-bar-track">
                      <div
                        class="rank-bar-fill"
                        :style="{ width: `${toPercent(item.value, maxErrorDistributionValue)}%`, backgroundColor: getCssVar('--cs-error-color') || '#f56c6c' }" />
                    </div>
                  </div>
                </div>
                <el-empty v-else :description="$t('common.noData')" />
              </div>
            </el-tab-pane>
          </el-tabs>
        </div>
      </div>
    </div>

    <!-- Error Detail Dialog -->
    <el-dialog v-model="errorDialogVisible"
      :title="$t('settings.proxy.stats.errorDetailTitle', { date: selectedErrorDate })" width="700px" append-to-body
      class="error-detail-dialog">
      <el-table :data="errorStats" size="small" border v-loading="errorLoading" max-height="60vh" style="width: 100%">
        <el-table-column prop="statusCode" :label="$t('settings.proxy.stats.statusCode')" width="80" />
        <el-table-column prop="errorMessage" :label="$t('settings.proxy.stats.errorMessage')">
          <template #default="scope">
            <div class="error-msg-text">{{ scope.row.errorMessage }}</div>
          </template>
        </el-table-column>
        <el-table-column prop="errorCount" :label="$t('settings.proxy.stats.count')" width="80" />
      </el-table>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, ref, onMounted, onUnmounted, watch } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { Refresh, Delete } from '@element-plus/icons-vue'
import { showMessage } from '@/libs/util'
import { ElMessageBox } from 'element-plus'
import { DataLine, Coin, Warning, Collection } from '@element-plus/icons-vue'

const { t } = useI18n()

const STORAGE_KEY_AUTO_REFRESH = 'ccproxy_stats_auto_refresh'

const loading = ref(false)
const selectedDays = ref(0)
const autoRefreshEnabled = ref(localStorage.getItem(STORAGE_KEY_AUTO_REFRESH) === 'true')
const dailyStats = ref([])
// Use reactive to ensure reactivity when dynamically adding keys
const providerStats = ref({})
const providerLoading = ref({})
const expandedDates = ref(new Set())

const errorDialogVisible = ref(false)
const errorLoading = ref(false)
const errorStats = ref([])
const selectedErrorDate = ref('')

// KPI data
const kpiData = ref({
  totalRequests: 0,
  totalTokens: 0,
  errorRate: 0,
  activeModels: 0
})

// Active tab for trend charts (Token first)
const activeTrendTab = ref('dailyTokens')

// Active tab for distribution charts
const activeDistributionTab = ref('modelTokenUsage')

const requestsTrend = ref([])
const tokenTrend = ref([])
const modelUsageStats = ref([])
const modelTokenUsageStats = ref([])
const providerTokenUsageStats = ref([])
const errorDistributionStats = ref([])
let refreshTimer = null

const startRefreshTimer = () => {
  if (refreshTimer) clearInterval(refreshTimer)
  if (!autoRefreshEnabled.value) {
    refreshTimer = null
    return
  }
  refreshTimer = setInterval(() => {
    fetchDailyStats(true)
  }, 10000)
}

const formatTokens = val => {
  if (val === undefined || val === null || isNaN(val)) return '0'
  const num = Number(val)
  if (num >= 100000000) {
    return (num / 100000000).toFixed(2) + ' 亿'
  }
  if (num >= 100000) {
    return (num / 10000).toFixed(2) + ' 万'
  }
  return num.toLocaleString()
}

const formatNumber = val => {
  if (val === undefined || val === null || isNaN(val)) return '0'
  const num = Number(val)
  if (num >= 100000000) {
    return (num / 100000000).toFixed(2) + ' 亿'
  }
  if (num >= 10000) {
    return (num / 10000).toFixed(2) + ' 万'
  }
  return num.toLocaleString()
}

// Get CSS variable color value
const getCssVar = (varName) => {
  return getComputedStyle(document.documentElement).getPropertyValue(varName).trim() || ''
}

const getSeriesColor = (index) => {
  const palette = [
    getCssVar('--cs-info-color') || '#409eff',
    getCssVar('--cs-success-color') || '#67c23a',
    getCssVar('--cs-warning-color') || '#e6a23c',
    getCssVar('--cs-error-color') || '#f56c6c',
    '#8b5cf6',
    '#14b8a6'
  ]
  return palette[index % palette.length]
}

const getMaxValue = items => items.reduce((max, item) => Math.max(max, Number(item.value || 0)), 0)

const toPercent = (value, max) => {
  if (!max || max <= 0) return 0
  return Math.max(0, Math.min(100, (Number(value || 0) / max) * 100))
}

const maxRequestsValue = computed(() => requestsTrend.value.reduce((max, item) => Math.max(max, item.requests), 0))
const maxErrorRateValue = computed(() => requestsTrend.value.reduce((max, item) => Math.max(max, item.errorRate), 0))
const maxModelUsageValue = computed(() => getMaxValue(modelUsageStats.value))
const maxModelTokenUsageValue = computed(() => getMaxValue(modelTokenUsageStats.value))
const maxProviderTokenUsageValue = computed(() => getMaxValue(providerTokenUsageStats.value))
const maxErrorDistributionValue = computed(() => getMaxValue(errorDistributionStats.value))

// Calculate KPI data from daily stats and model usage stats
const calculateKPI = (modelUsage = []) => {
  if (!dailyStats.value || dailyStats.value.length === 0) {
    kpiData.value = {
      totalRequests: 0,
      totalTokens: 0,
      errorRate: 0,
      activeModels: 0
    }
    return
  }

  let totalRequests = 0
  let totalTokens = 0
  let totalErrors = 0

  dailyStats.value.forEach(day => {
    totalRequests += Number(day.totalRequestCount || 0)
    totalTokens += Number(day.totalInputTokens || 0) + Number(day.totalOutputTokens || 0) + Number(day.totalCacheTokens || 0)
    totalErrors += Number(day.errorCount || 0)
  })

  const errorRate = totalRequests > 0 ? (totalErrors / totalRequests) * 100 : 0

  // Count unique models from model usage stats
  const activeModels = modelUsage.length > 0 ? modelUsage.length : 0

  kpiData.value = {
    totalRequests,
    totalTokens,
    errorRate,
    activeModels
  }
}

const getProtocolColor = (protocol) => {
  const colorMap = {
    'openai': 'rgba(16, 163, 127, 0.1)',
    'claude': 'rgba(229, 119, 25, 0.1)',
    'gemini': 'rgba(0, 108, 255, 0.1)',
    'ollama': 'rgba(80, 85, 242, 0.1)'
  }
  return colorMap[protocol] || ''
}

const getProtocolTextColor = (protocol) => {
  const colorMap = {
    'openai': '#10a37f',
    'claude': '#e57719',
    'gemini': '#006cff',
    'ollama': '#5055f2'
  }
  return colorMap[protocol] || 'var(--el-text-color-regular)'
}

const fetchDailyStats = async (isAutoRefresh = false) => {
  if (isAutoRefresh === false) {
    startRefreshTimer()
    loading.value = true
    dailyStats.value = []
    providerStats.value = {}
    providerLoading.value = {}
  }
  try {
    const res = await invokeWrapper('get_ccproxy_daily_stats', { days: selectedDays.value })
    // Use spread to ensure Vue detects array update even if content is similar
    dailyStats.value = res ? [...res] : []

    // If auto-refreshing, also refresh data for currently expanded rows
    if (isAutoRefresh === true && expandedDates.value.size > 0) {
      for (const date of expandedDates.value) {
        fetchProviderStats(date, true)
      }
    }

    await updateCharts()
  } catch (error) {
    console.error('Failed to fetch proxy stats:', error)
  } finally {
    if (isAutoRefresh === false) {
      loading.value = false
    }
  }
}

const updateCharts = async () => {
  try {
    const [modelUsage, modelTokenUsage, providerTokenUsage, errorDist] = await Promise.all([
      invokeWrapper('get_ccproxy_model_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_model_token_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_provider_token_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_error_distribution_stats', { days: selectedDays.value })
    ])

    // Calculate KPI data with model usage stats
    calculateKPI(modelUsage)

    requestsTrend.value = (dailyStats.value || [])
      .slice()
      .reverse()
      .map(day => {
        const requests = Number(day.totalRequestCount || 0)
        const errors = Number(day.errorCount || 0)
        return {
          date: day.date,
          requests,
          errorRate: requests > 0 ? Number(((errors / requests) * 100).toFixed(2)) : 0
        }
      })

    tokenTrend.value = (dailyStats.value || [])
      .slice()
      .reverse()
      .map(day => {
        const segments = [
          {
            type: t('settings.proxy.stats.inputTokens'),
            value: Number(day.totalInputTokens || 0),
            color: getSeriesColor(0)
          },
          {
            type: t('settings.proxy.stats.outputTokens'),
            value: Number(day.totalOutputTokens || 0),
            color: getSeriesColor(1)
          },
          {
            type: t('settings.proxy.stats.cacheTokens'),
            value: Number(day.totalCacheTokens || 0),
            color: getSeriesColor(2)
          }
        ]
        const total = segments.reduce((sum, item) => sum + item.value, 0)
        return {
          date: day.date,
          total,
          segments: segments
            .filter(item => item.value > 0)
            .map(item => ({
              ...item,
              percent: total > 0 ? (item.value / total) * 100 : 0
            }))
        }
      })

    // Sort model usage data by value descending for horizontal bar chart
    modelUsageStats.value = (modelUsage || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 10) // Top 10

    // Sort model token usage data by value descending
    modelTokenUsageStats.value = (modelTokenUsage || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 10) // Top 10

    // Sort provider token usage data by value descending
    providerTokenUsageStats.value = (providerTokenUsage || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)

    // Sort error distribution data by value descending
    errorDistributionStats.value = (errorDist || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)
  } catch (error) {
    console.error('Failed to update charts:', error)
  }
}

const fetchProviderStats = async (date, force = false) => {
  if (providerStats.value[date] && !force) return

  providerLoading.value = { ...providerLoading.value, [date]: true }

  try {
    const stats = await invokeWrapper('get_ccproxy_provider_stats_by_date', { date })
    providerStats.value = { ...providerStats.value, [date]: stats || [] }
  } catch (error) {
    console.error('Failed to fetch provider stats:', error)
  } finally {
    providerLoading.value = { ...providerLoading.value, [date]: false }
  }
}

const showErrorDetail = async (date, clientModel = null, backendModel = null) => {
  selectedErrorDate.value = date
  errorDialogVisible.value = true
  errorLoading.value = true
  try {
    const stats = await invokeWrapper('get_ccproxy_error_stats_by_date', {
      date,
      clientModel,
      backendModel
    })
    errorStats.value = stats || []
  } catch (error) {
    console.error('Failed to fetch error stats:', error)
  } finally {
    errorLoading.value = false
  }
}

const handleExpandChange = (row, expandedRows) => {
  const isExpanded = expandedRows.some(r => r.date === row.date)
  if (isExpanded) {
    expandedDates.value.add(row.date)
    fetchProviderStats(row.date)
  } else {
    expandedDates.value.delete(row.date)
  }
}

const handleDeleteStats = async days => {
  const confirmMessage =
    days === -1 ? t('settings.proxy.stats.deleteAllConfirm') : t('settings.proxy.stats.deleteConfirm', { days })

  try {
    await ElMessageBox.confirm(confirmMessage, t('common.confirm'), {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    })

    loading.value = true
    await invokeWrapper('delete_ccproxy_stats', { days })
    showMessage(t('common.deleteSuccess'), 'success')
    fetchDailyStats(false)
  } catch (error) {
    if (error !== 'cancel') {
      console.error('Failed to delete proxy stats:', error)
      showMessage(error.message || String(error), 'error')
    }
  } finally {
    loading.value = false
  }
}

watch(autoRefreshEnabled, (val) => {
  localStorage.setItem(STORAGE_KEY_AUTO_REFRESH, val ? 'true' : 'false')
  if (val) {
    startRefreshTimer()
  } else {
    if (refreshTimer) {
      clearInterval(refreshTimer)
      refreshTimer = null
    }
  }
})

onMounted(() => {
  fetchDailyStats()
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})
</script>

<style lang="scss" scoped>
.proxy-stats {
  margin-top: var(--cs-space);

  :deep(.el-table) {
    overscroll-behavior: none;

    .el-table__inner-wrapper,
    .el-table__body-wrapper,
    .el-scrollbar__bar {
      overscroll-behavior: none;
    }
  }
}

.stats-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--cs-space-md);

  h3 {
    margin: 0;
    font-size: var(--cs-font-size-lg);
    font-weight: 600;
  }
}

// KPI Cards
.kpi-cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--cs-space-md);
  margin-bottom: var(--cs-space-lg);
  padding: 4px;

  @media (max-width: 1000px) {
    grid-template-columns: repeat(2, 1fr);
  }

  @media (max-width: 600px) {
    grid-template-columns: 1fr;
  }
}

.kpi-card {
  display: flex;
  align-items: center;
  gap: var(--cs-space-md);
  background-color: var(--cs-primary-bg-color);
  border: 1px solid var(--cs-border-color-light);
  border-radius: var(--cs-border-radius-md);
  padding: var(--cs-space-md);
  transition: box-shadow 0.3s ease;

  &:hover {
    box-shadow: 0 2px 12px var(--cs-shadow-color);
  }

  .kpi-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: var(--cs-border-radius-md);
    font-size: 24px;
  }

  .kpi-content {
    flex: 1;

    .kpi-value {
      font-size: 24px;
      font-weight: 600;
      color: var(--cs-text-color-primary);
      line-height: 1.2;

      &.text-danger {
        color: var(--cs-error-color);
      }
    }

    .kpi-label {
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-secondary);
      margin-top: 4px;
    }
  }
}

.charts-section {
  margin-top: var(--cs-space-lg);
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-lg);
}

.charts-row {
  display: flex;
  gap: var(--cs-space-lg);
  flex-wrap: wrap;
}

.chart-card {
  flex: 1;
  min-width: 300px;
  background-color: var(--cs-primary-bg-color);
  border: 1px solid var(--cs-border-color-light);
  border-radius: var(--cs-border-radius-md);
  padding: var(--cs-space-md);

  h4 {
    margin-top: 0;
    margin-bottom: var(--cs-space-md);
    font-size: var(--cs-font-size-md);
    color: var(--cs-text-color-secondary);
    text-align: center;
  }

  &.tab-chart {
    min-width: 100%;
    padding: 0;

    :deep(.el-tabs) {
      border: none;

      .el-tabs__header {
        margin: 0;
        background-color: var(--cs-bg-color-light);
        border-bottom: 1px solid var(--cs-border-color-light);

        .el-tabs__nav {
          border: none;
        }

        .el-tabs__item {
          border: none;
          padding: 0 20px;
          height: 40px;
          line-height: 40px;
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-secondary);
          transition: all 0.3s;

          &.is-active {
            color: var(--cs-color-primary);
            background-color: var(--cs-primary-bg-color);
            font-weight: 500;
          }

          &:hover {
            color: var(--cs-color-primary);
          }
        }
      }

      .el-tabs__content {
        padding: var(--cs-space-md);
      }
    }

    .tab-chart-content {
      min-height: 380px;
    }
  }
}

.trend-stack-chart {
  display: flex;
  align-items: flex-end;
  gap: 12px;
  height: 100%;
  overflow-x: auto;
  padding-bottom: 4px;
}

.trend-stack-item {
  min-width: 72px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}

.trend-stack-bars {
  width: 40px;
  height: 220px;
  display: flex;
  flex-direction: column-reverse;
  overflow: hidden;
  border-radius: 12px;
  background: var(--cs-bg-color-light);
  border: 1px solid var(--cs-border-color-light);
}

.trend-stack-segment {
  width: 100%;
  min-height: 2px;
}

.trend-stack-total,
.trend-stack-label {
  width: 100%;
  text-align: center;
  font-size: 12px;
}

.trend-stack-total {
  color: var(--cs-text-color-primary);
  font-weight: 600;
}

.trend-stack-label {
  color: var(--cs-text-color-secondary);
}

.trend-metric-list,
.rank-bar-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.trend-metric-row,
.rank-bar-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.trend-metric-header,
.rank-bar-header {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 13px;
}

.rank-bar-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rank-bar-value {
  flex-shrink: 0;
  color: var(--cs-text-color-secondary);
}

.trend-metric-track,
.rank-bar-track {
  width: 100%;
  height: 10px;
  background: var(--cs-bg-color-light);
  border-radius: 999px;
  overflow: hidden;
}

.trend-metric-track.error,
.rank-bar-track {
  height: 8px;
}

.trend-metric-fill,
.rank-bar-fill {
  height: 100%;
  border-radius: inherit;
}

.trend-metric-fill.requests {
  background: var(--cs-info-color);
}

.trend-metric-fill.errors {
  background: var(--cs-error-color);
}

.expand-detail {
  padding: var(--cs-space-xs);
  background-color: var(--cs-bg-color-light);
}

.error-msg-text {
  word-break: break-all;
  white-space: pre-wrap;
  max-height: 200px;
  overflow-y: auto;
  font-family: var(--cs-font-family-mono);
  font-size: 12px;
  line-height: 1.5;
}
</style>
