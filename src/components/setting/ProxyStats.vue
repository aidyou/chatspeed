<template>
  <div class="proxy-stats">
    <div class="stats-header">
      <h3>{{ $t('settings.proxy.stats.title') }}</h3>
      <div class="header-actions">
        <el-space>
          <el-select
            v-model="selectedDays"
            size="small"
            @change="fetchDailyStats(false)"
            style="width: 140px; margin-left: 10px">
            <el-option :label="$t('settings.proxy.stats.today')" :value="0" />
            <el-option :label="$t('settings.proxy.stats.last1Day')" :value="1" />
            <el-option :label="$t('settings.proxy.stats.last7Days')" :value="7" />
            <el-option :label="$t('settings.proxy.stats.last30Days')" :value="30" />
            <el-option :label="$t('settings.proxy.stats.last90Days')" :value="90" />
            <el-option :label="$t('settings.proxy.stats.last365Days')" :value="365" />
            <el-option :label="$t('settings.proxy.stats.allTime')" :value="-1" />
          </el-select>
          <el-button
            :icon="Refresh"
            circle
            size="small"
            @click="fetchDailyStats(false)"
            :loading="loading" />
          <el-checkbox v-model="autoRefreshEnabled" size="small" style="margin-right: 15px">
            {{ $t('settings.proxy.stats.autoRefresh') }}
          </el-checkbox>
        </el-space>

        <el-dropdown trigger="click" @command="handleDeleteStats" style="margin-left: 10px">
          <el-button :icon="Delete" type="danger" plain size="small" circle />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item :command="7">{{
                $t('settings.proxy.stats.deleteOlderThan7Days')
              }}</el-dropdown-item>
              <el-dropdown-item :command="30">{{
                $t('settings.proxy.stats.deleteOlderThan30Days')
              }}</el-dropdown-item>
              <el-dropdown-item :command="90">{{
                $t('settings.proxy.stats.deleteOlderThan90Days')
              }}</el-dropdown-item>
              <el-dropdown-item :command="365">{{
                $t('settings.proxy.stats.deleteOlderThan365Days')
              }}</el-dropdown-item>
              <el-dropdown-item :command="-1" divided style="color: var(--el-color-danger)">{{
                $t('settings.proxy.stats.deleteAll')
              }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>

    <!-- KPI Cards -->
    <div class="kpi-cards">
      <div class="kpi-card">
        <div class="kpi-icon" style="background-color: rgba(103, 194, 58, 0.1); color: #67c23a">
          <el-icon>
            <Coin />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value">{{ formatCurrencyCompact(kpiData.estimatedCost) }}</div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.estimatedCost') }}</div>
        </div>
      </div>
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
        <div class="kpi-icon" style="background-color: rgba(144, 147, 153, 0.1); color: #909399">
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
        <div class="kpi-icon" style="background-color: rgba(230, 162, 60, 0.1); color: #e6a23c">
          <el-icon>
            <Collection />
          </el-icon>
        </div>
        <div class="kpi-content">
          <div class="kpi-value">{{ formatPercent(kpiData.cacheHitRate) }}</div>
          <div class="kpi-label">{{ $t('settings.proxy.stats.cacheHitRate') }}</div>
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
    </div>

    <el-table
      :data="dailyStats"
      style="width: 100%"
      max-height="350"
      v-loading="loading"
      @expand-change="handleExpandChange"
      row-key="date">
      <el-table-column type="expand" fixed="left">
        <template #default="props">
          <div class="expand-detail">
            <el-table
              :data="providerStats[props.row.date] || []"
              size="small"
              border
              v-loading="providerLoading[props.row.date]">
              <el-table-column
                prop="provider"
                :label="$t('settings.proxy.stats.provider')"
                width="100"
                show-overflow-tooltip />
              <el-table-column
                prop="clientModel"
                :label="$t('settings.proxy.stats.clientModel')"
                min-width="150"
                show-overflow-tooltip
                sortable>
                <template #default="scope">
                  <span style="color: var(--cs-color-primary); font-weight: bold">{{
                    scope.row.clientModel
                  }}</span>
                </template>
              </el-table-column>
              <el-table-column
                prop="backendModel"
                :label="$t('settings.proxy.stats.backendModel')"
                min-width="200"
                show-overflow-tooltip
                sortable />
              <el-table-column
                prop="protocol"
                :label="$t('settings.proxy.stats.protocol')"
                width="90">
                <template #default="scope">
                  <el-tag
                    size="small"
                    :color="getProtocolColor(scope.row.protocol)"
                    :style="{
                      color: getProtocolTextColor(scope.row.protocol),
                      borderColor: getProtocolTextColor(scope.row.protocol) + '50'
                    }">
                    {{ scope.row.protocol }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.toolCompat')"
                width="90"
                align="center">
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
              <el-table-column
                prop="requestCount"
                :label="$t('settings.proxy.stats.requests')"
                width="100"
                sortable />
              <el-table-column
                :label="$t('settings.proxy.stats.inputTokens')"
                width="90"
                sortable
                sort-by="totalInputTokens">
                <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.outputTokens')"
                width="90"
                sortable
                sort-by="totalOutputTokens">
                <template #default="scope">{{
                  formatTokens(scope.row.totalOutputTokens)
                }}</template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.cacheTokens')"
                width="90"
                sortable
                sort-by="totalCacheTokens">
                <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.cacheHitRate')"
                width="110"
                sortable
                :sort-method="
                  (a, b) =>
                    getCacheHitRateValue(a.totalCacheTokens, a.totalInputTokens) -
                    getCacheHitRateValue(b.totalCacheTokens, b.totalInputTokens)
                ">
                <template #default="scope">
                  {{
                    formatPercent(
                      getCacheHitRateValue(scope.row.totalCacheTokens, scope.row.totalInputTokens)
                    )
                  }}
                </template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.estimatedCost')"
                width="130"
                sortable
                sort-by="estimatedCost">
                <template #default="scope">
                  {{ formatCurrency(scope.row.estimatedCost) }}
                </template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.proxy.stats.errors')"
                width="100"
                sortable
                sort-by="errorCount">
                <template #default="scope">
                  <el-link
                    v-if="scope.row.errorCount > 0"
                    type="danger"
                    @click="
                      showErrorDetail(props.row.date, scope.row.clientModel, scope.row.backendModel)
                    ">
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
      <el-table-column
        prop="providerCount"
        :label="$t('settings.proxy.stats.providers')"
        width="90"
        align="center" />
      <el-table-column
        prop="topProvider"
        :label="$t('settings.proxy.stats.topProvider')"
        min-width="180"
        show-overflow-tooltip />
      <el-table-column
        prop="totalRequestCount"
        :label="$t('settings.proxy.stats.requests')"
        width="100" />
      <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalOutputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.cacheHitRate')" width="110">
        <template #default="scope">
          {{
            formatPercent(
              getCacheHitRateValue(scope.row.totalCacheTokens, scope.row.totalInputTokens)
            )
          }}
        </template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.estimatedCost')" width="130">
        <template #default="scope">
          {{ formatCurrency(scope.row.estimatedCost) }}
        </template>
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
                <div v-show="activeTrendTab === 'dailyTokens'" id="daily-tokens-column"></div>
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.dailyCostTitle')" name="dailyCost">
              <div class="tab-chart-content">
                <div v-show="activeTrendTab === 'dailyCost'" id="daily-cost-line"></div>
              </div>
            </el-tab-pane>
            <el-tab-pane
              :label="$t('settings.proxy.stats.dailyRequestsTitle')"
              name="dailyRequests">
              <div class="tab-chart-content">
                <div
                  v-show="activeTrendTab === 'dailyRequests'"
                  id="daily-requests-dual-axis"></div>
              </div>
            </el-tab-pane>
          </el-tabs>
        </div>
      </div>

      <!-- 2. Distribution charts in Tabs -->
      <div class="charts-row">
        <div class="chart-card tab-chart">
          <el-tabs v-model="activeDistributionTab" type="border-card">
            <el-tab-pane
              :label="$t('settings.proxy.stats.modelTokenUsageTitle')"
              name="modelTokenUsage">
              <div class="tab-chart-content">
                <div
                  v-show="activeDistributionTab === 'modelTokenUsage'"
                  id="model-token-usage-bar"></div>
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.modelUsageTitle')" name="modelUsage">
              <div class="tab-chart-content">
                <div v-show="activeDistributionTab === 'modelUsage'" id="model-usage-bar"></div>
              </div>
            </el-tab-pane>
            <el-tab-pane
              :label="$t('settings.proxy.stats.providerTokenUsageTitle')"
              name="providerTokenUsage">
              <div class="tab-chart-content">
                <div
                  v-show="activeDistributionTab === 'providerTokenUsage'"
                  id="provider-token-usage-bar"></div>
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.proxy.stats.errorDistTitle')" name="errorDist">
              <div class="tab-chart-content">
                <div v-show="activeDistributionTab === 'errorDist'" id="error-dist-bar"></div>
              </div>
            </el-tab-pane>
          </el-tabs>
        </div>
      </div>
    </div>

    <!-- Error Detail Dialog -->
    <el-dialog
      v-model="errorDialogVisible"
      :title="$t('settings.proxy.stats.errorDetailTitle', { date: selectedErrorDate })"
      width="700px"
      append-to-body
      class="error-detail-dialog">
      <el-table
        :data="errorStats"
        size="small"
        border
        v-loading="errorLoading"
        max-height="60vh"
        style="width: 100%">
        <el-table-column
          prop="statusCode"
          :label="$t('settings.proxy.stats.statusCode')"
          width="80" />
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
import { markRaw, ref, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { Bar, DualAxes, Line } from '@antv/g2plot'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { Refresh, Delete } from '@element-plus/icons-vue'
import { showMessage } from '@/libs/util'
import { ElMessageBox } from 'element-plus'
import { DataLine, Coin, Warning, Collection } from '@element-plus/icons-vue'
import { useModelStore } from '@/stores/model'
import {
  buildPricingMaps,
  estimateCostFromPricing,
  formatCurrency,
  formatCurrencyCompact
} from '@/libs/modelPricing'

const { t } = useI18n()
const modelStore = useModelStore()

const STORAGE_KEY_AUTO_REFRESH = 'ccproxy_stats_auto_refresh'

const loading = ref(false)
const selectedDays = ref(0)
const autoRefreshEnabled = ref(localStorage.getItem(STORAGE_KEY_AUTO_REFRESH) === 'true')
const dailyStatsRaw = ref([])
const dailyStats = ref([])
const groupedStatsRaw = ref([])
// Use reactive to ensure reactivity when dynamically adding keys
const providerStats = ref({})
const providerStatsRaw = ref({})
const providerLoading = ref({})
const expandedDates = ref(new Set())
const pricingMaps = ref(buildPricingMaps(modelStore.providers))

const errorDialogVisible = ref(false)
const errorLoading = ref(false)
const errorStats = ref([])
const selectedErrorDate = ref('')

// KPI data
const kpiData = ref({
  totalRequests: 0,
  totalTokens: 0,
  errorRate: 0,
  cacheHitRate: 0,
  estimatedCost: 0
})

// Active tab for trend charts (Token first)
const activeTrendTab = ref('dailyTokens')

// Active tab for distribution charts
const activeDistributionTab = ref('modelTokenUsage')

let modelBarChart = null
let modelTokenBarChart = null
let providerTokenBarChart = null
let errorBarChart = null
let tokenBarChart = null
let costLineChart = null
let requestsDualAxisChart = null
let refreshTimer = null
let isRefreshing = false

const scheduleNextRefresh = () => {
  if (!autoRefreshEnabled.value) {
    refreshTimer = null
    return
  }
  refreshTimer = setTimeout(async () => {
    if (isRefreshing) {
      // Previous refresh still in progress, try again later
      scheduleNextRefresh()
      return
    }
    isRefreshing = true
    try {
      await fetchDailyStats(true)
    } finally {
      isRefreshing = false
      scheduleNextRefresh()
    }
  }, 10000)
}

const startRefreshTimer = () => {
  if (refreshTimer) {
    clearTimeout(refreshTimer)
    refreshTimer = null
  }
  if (!autoRefreshEnabled.value) {
    return
  }
  scheduleNextRefresh()
}

const formatTokens = val => {
  if (val === undefined || val === null || isNaN(val)) return '0'
  const num = Number(val)
  if (num >= 1000000) {
    return (num / 1000000).toFixed(2) + 'M'
  }
  if (num >= 1000) {
    return (num / 1000).toFixed(2) + 'K'
  }
  return num.toLocaleString()
}

const formatNumber = val => {
  if (val === undefined || val === null || isNaN(val)) return '0'
  const num = Number(val)
  if (num >= 1000000) {
    return (num / 1000000).toFixed(2) + 'M'
  }
  if (num >= 1000) {
    return (num / 1000).toFixed(2) + 'K'
  }
  return num.toLocaleString()
}

const estimateRowCost = row => {
  const pricing = pricingMaps.value.byProviderName.get(`${row.provider}::${row.backendModel}`)
  return estimateCostFromPricing(
    {
      inputTokens: row.totalInputTokens,
      outputTokens: row.totalOutputTokens,
      cacheTokens: row.totalCacheTokens
    },
    pricing
  )
}

const getCacheHitRateValue = (cacheTokens, inputTokens) => {
  const input = Number(inputTokens || 0)
  const cache = Number(cacheTokens || 0)
  if (input <= 0 || cache <= 0) return 0
  return (cache / input) * 100
}

const formatPercent = val => {
  const num = Number(val || 0)
  return `${num.toFixed(2)}%`
}

// Get CSS variable color value
const getCssVar = varName => {
  return getComputedStyle(document.documentElement).getPropertyValue(varName).trim() || ''
}

const formatAxisValue = val => {
  if (val === undefined || val === null || isNaN(val)) return '0'
  const num = Number(val)
  if (num >= 1000000) {
    return (num / 1000000).toFixed(2) + 'M'
  }
  if (num >= 1000) {
    return (num / 1000).toFixed(2) + 'K'
  }
  return num.toString()
}

const getAxisColors = () => {
  const isDark = document.documentElement.classList.contains('dark')
  return {
    gridStroke: isDark ? 'rgba(255, 255, 255, 0.15)' : 'rgba(0, 0, 0, 0.1)',
    lineStroke: isDark ? 'rgba(255, 255, 255, 0.25)' : 'rgba(0, 0, 0, 0.2)'
  }
}

const getCommonAxisConfig = () => {
  const colors = getAxisColors()
  return {
    grid: {
      line: {
        style: {
          lineDash: [4, 4],
          stroke: colors.gridStroke
        }
      }
    },
    line: {
      style: {
        stroke: colors.lineStroke,
        lineWidth: 1
      }
    }
  }
}

// Calculate KPI data from daily stats and model usage stats
const calculateKPI = (modelUsage = []) => {
  if (!dailyStats.value || dailyStats.value.length === 0) {
    kpiData.value = {
      totalRequests: 0,
      totalTokens: 0,
      errorRate: 0,
      cacheHitRate: 0,
      estimatedCost: 0
    }
    return
  }

  let totalRequests = 0
  let totalTokens = 0
  let totalErrors = 0
  let totalInputTokens = 0
  let totalCacheTokens = 0
  let estimatedCost = 0

  dailyStats.value.forEach(day => {
    totalRequests += Number(day.totalRequestCount || 0)
    totalInputTokens += Number(day.totalInputTokens || 0)
    totalCacheTokens += Number(day.totalCacheTokens || 0)
    totalTokens += Number(day.totalInputTokens || 0) + Number(day.totalOutputTokens || 0)
    totalErrors += Number(day.errorCount || 0)
    estimatedCost += Number(day.estimatedCost || 0)
  })

  const errorRate = totalRequests > 0 ? (totalErrors / totalRequests) * 100 : 0
  const cacheHitRate = getCacheHitRateValue(totalCacheTokens, totalInputTokens)

  kpiData.value = {
    totalRequests,
    totalTokens,
    errorRate,
    cacheHitRate,
    estimatedCost
  }
}

const enrichProviderRows = rows =>
  (rows || []).map(row => ({
    ...row,
    estimatedCost: estimateRowCost(row)
  }))

const syncDailyStatsWithCosts = () => {
  const groupedByDate = groupedStatsRaw.value.reduce((map, row) => {
    const date = row.date
    map.set(date, (map.get(date) || 0) + estimateRowCost(row))
    return map
  }, new Map())

  dailyStats.value = (dailyStatsRaw.value || []).map(day => ({
    ...day,
    estimatedCost: groupedByDate.get(day.date) || 0
  }))
}

const getProtocolColor = protocol => {
  const colorMap = {
    openai: 'rgba(16, 163, 127, 0.1)',
    claude: 'rgba(229, 119, 25, 0.1)',
    gemini: 'rgba(0, 108, 255, 0.1)',
    ollama: 'rgba(80, 85, 242, 0.1)'
  }
  return colorMap[protocol] || ''
}

const getProtocolTextColor = protocol => {
  const colorMap = {
    openai: '#10a37f',
    claude: '#e57719',
    gemini: '#006cff',
    ollama: '#5055f2'
  }
  return colorMap[protocol] || 'var(--el-text-color-regular)'
}

const fetchDailyStats = async (isAutoRefresh = false) => {
  if (isAutoRefresh === false) {
    startRefreshTimer()
    loading.value = true
    dailyStatsRaw.value = []
    dailyStats.value = []
    groupedStatsRaw.value = []
    providerStats.value = {}
    providerStatsRaw.value = {}
    providerLoading.value = {}
  }
  try {
    const [dailyRes, groupedRes] = await Promise.all([
      invokeWrapper('get_ccproxy_daily_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_grouped_stats', { days: selectedDays.value })
    ])
    dailyStatsRaw.value = dailyRes ? [...dailyRes] : []
    groupedStatsRaw.value = groupedRes ? [...groupedRes] : []
    syncDailyStatsWithCosts()

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
    const [modelUsage, errorDist] = await Promise.all([
      invokeWrapper('get_ccproxy_model_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_error_distribution_stats', { days: selectedDays.value })
    ])

    // Calculate KPI data with model usage stats
    calculateKPI(modelUsage)

    await nextTick()

    const requestsData = []
    const errorRateData = []
    const tokenBarData = []
    const tokenLineData = []
    const costLineData = []

    ;(dailyStats.value || [])
      .slice()
      .reverse()
      .forEach(day => {
        const totalRequests = Number(day.totalRequestCount || 0)
        const errorCount = Number(day.errorCount || 0)
        const errorRate = totalRequests > 0 ? (errorCount / totalRequests) * 100 : 0

        requestsData.push({
          date: day.date,
          value: totalRequests,
          type: t('settings.proxy.stats.requests')
        })
        errorRateData.push({
          date: day.date,
          value: Number(errorRate.toFixed(2)),
          type: t('settings.proxy.stats.errorRate')
        })

        tokenBarData.push({
          date: day.date,
          type: t('settings.proxy.stats.inputTokens'),
          value: Number(day.totalInputTokens || 0)
        })
        tokenBarData.push({
          date: day.date,
          type: t('settings.proxy.stats.outputTokens'),
          value: Number(day.totalOutputTokens || 0)
        })
        tokenLineData.push({
          date: day.date,
          type: t('settings.proxy.stats.cacheTokens'),
          value: Number(day.totalCacheTokens || 0)
        })
        costLineData.push({
          date: day.date,
          value: Number(day.estimatedCost || 0)
        })
      })

    if (!requestsDualAxisChart) {
      const container = document.getElementById('daily-requests-dual-axis')
      if (container) {
        requestsDualAxisChart = markRaw(
          new DualAxes('daily-requests-dual-axis', {
            data: [requestsData, errorRateData],
            xField: 'date',
            yField: ['value', 'value'],
            geometryOptions: [
              {
                geometry: 'column',
                color: getCssVar('--cs-info-color') || '#409eff',
                label: {
                  position: 'middle',
                  formatter: datum => {
                    const dayCount = dailyStats.value?.length || 0
                    return dayCount <= 5 ? formatNumber(datum.value) : ''
                  }
                }
              },
              {
                geometry: 'line',
                color: getCssVar('--cs-error-color') || '#f56c6c',
                lineStyle: { lineWidth: 3 },
                point: { size: 4, shape: 'circle' },
                label: {
                  position: 'top',
                  formatter: datum => {
                    const dayCount = dailyStats.value?.length || 0
                    return dayCount <= 5 ? `${datum.value}%` : ''
                  }
                }
              }
            ],
            xAxis: {
              ...getCommonAxisConfig(),
              grid: null
            },
            yAxis: [
              {
                ...getCommonAxisConfig(),
                label: {
                  formatter: val => formatAxisValue(val)
                }
              },
              {
                label: {
                  formatter: val => `${val}%`
                }
              }
            ],
            legend: {
              position: 'bottom',
              itemName: {
                formatter: (_text, item) => {
                  return item.value === 'value' && item.index === 0
                    ? t('settings.proxy.stats.requests')
                    : t('settings.proxy.stats.errorRate')
                }
              }
            },
            tooltip: {
              shared: true,
              showMarkers: true,
              customContent: (title, items) => {
                if (!items || items.length === 0) return ''
                let html = `<div style="padding: 8px 12px;"><div style="font-weight: 500; margin-bottom: 8px;">${title}</div>`
                items.forEach((item, index) => {
                  const color = item.color || '#999'
                  const name =
                    index === 0
                      ? t('settings.proxy.stats.requests')
                      : t('settings.proxy.stats.errorRate')
                  const value = item.value !== undefined ? item.value : ''
                  const displayValue = index === 1 ? `${value}%` : formatNumber(value)
                  html += `<div style="display: flex; align-items: center; margin-bottom: 4px;">
                    <span style="display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: ${color}; margin-right: 8px;"></span>
                    <span style="flex: 1;">${name}:</span>
                    <span style="font-weight: 500; margin-left: 12px;">${displayValue}</span>
                  </div>`
                })
                html += '</div>'
                return html
              }
            },
            slider: (dailyStats.value?.length || 0) > 10 ? { start: 0, end: 1 } : null
          })
        )
        requestsDualAxisChart.render()
      }
    } else {
      requestsDualAxisChart.update({ data: [requestsData, errorRateData] })
      // Update slider visibility based on data points
      const dayCount = dailyStats.value?.length || 0
      requestsDualAxisChart.update({
        slider: dayCount > 10 ? { start: 0, end: 1 } : null
      })
    }

    if (!tokenBarChart) {
      const container = document.getElementById('daily-tokens-column')
      if (container) {
        tokenBarChart = markRaw(
          new DualAxes('daily-tokens-column', {
            data: [tokenBarData, tokenLineData],
            xField: 'date',
            yField: ['value', 'value'],
            geometryOptions: [
              {
                geometry: 'column',
                isStack: true,
                seriesField: 'type',
                color: [
                  getCssVar('--cs-info-color') || '#409eff',
                  getCssVar('--cs-warning-color') || '#e6a23c'
                ],
                label: {
                  position: 'middle',
                  layout: [{ type: 'interval-adjust-position' }, { type: 'interval-hide-overlap' }],
                  formatter: datum => {
                    const dayCount = dailyStats.value?.length || 0
                    return dayCount <= 5 ? formatTokens(datum.value) : ''
                  }
                }
              },
              {
                geometry: 'line',
                seriesField: 'type',
                color: getCssVar('--cs-success-color') || '#67c23a',
                lineStyle: { lineWidth: 3 },
                point: { size: 4, shape: 'circle' }
              }
            ],
            xAxis: {
              ...getCommonAxisConfig(),
              grid: null
            },
            yAxis: [
              {
                ...getCommonAxisConfig(),
                label: {
                  formatter: val => formatAxisValue(val)
                }
              },
              {
                ...getCommonAxisConfig(),
                label: {
                  formatter: val => formatAxisValue(val)
                }
              }
            ],
            legend: {
              position: 'bottom'
            },
            tooltip: {
              shared: true,
              showMarkers: true,
              customContent: (title, items) => {
                if (!items || items.length === 0) return ''
                let html = `<div style="padding: 8px 12px;"><div style="font-weight: 500; margin-bottom: 8px;">${title}</div>`
                items.forEach(item => {
                  const color = item.color || '#999'
                  const name = item.name || t('settings.proxy.stats.cacheTokens')
                  const value = item.value !== undefined ? item.value : ''
                  html += `<div style="display: flex; align-items: center; margin-bottom: 4px;">
                    <span style="display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: ${color}; margin-right: 8px;"></span>
                    <span style="flex: 1;">${name}:</span>
                    <span style="font-weight: 500; margin-left: 12px;">${formatTokens(value)}</span>
                  </div>`
                })
                html += '</div>'
                return html
              }
            },
            slider: (dailyStats.value?.length || 0) > 10 ? { start: 0, end: 1 } : null
          })
        )
        tokenBarChart.render()
      }
    } else {
      tokenBarChart.update({ data: [tokenBarData, tokenLineData] })
      // Update slider visibility based on data points
      const dayCount = dailyStats.value?.length || 0
      tokenBarChart.update({
        slider: dayCount > 10 ? { start: 0, end: 1 } : null
      })
    }

    if (!costLineChart) {
      const container = document.getElementById('daily-cost-line')
      if (container) {
        costLineChart = markRaw(
          new Line('daily-cost-line', {
            data: costLineData,
            xField: 'date',
            yField: 'value',
            smooth: true,
            color: getCssVar('--cs-success-color') || '#67c23a',
            lineStyle: {
              lineWidth: 3
            },
            point: {
              size: 4,
              shape: 'circle'
            },
            xAxis: {
              ...getCommonAxisConfig(),
              grid: null
            },
            yAxis: {
              ...getCommonAxisConfig(),
              label: {
                formatter: val => formatCurrencyCompact(val)
              }
            },
            tooltip: {
              formatter: datum => ({
                name: t('settings.proxy.stats.estimatedCost'),
                value: formatCurrency(datum.value)
              })
            },
            slider: (dailyStats.value?.length || 0) > 10 ? { start: 0, end: 1 } : null
          })
        )
        costLineChart.render()
      }
    } else {
      costLineChart.changeData(costLineData)
      const dayCount = dailyStats.value?.length || 0
      costLineChart.update({
        slider: dayCount > 10 ? { start: 0, end: 1 } : null
      })
    }

    const sortedModelUsage = (modelUsage || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 10)

    if (!modelBarChart) {
      const container = document.getElementById('model-usage-bar')
      if (container) {
        modelBarChart = markRaw(
          new Bar('model-usage-bar', {
            data: sortedModelUsage,
            xField: 'value',
            yField: 'type',
            seriesField: 'type',
            legend: false,
            xAxis: {
              ...getCommonAxisConfig(),
              label: {
                formatter: val => formatAxisValue(val)
              }
            },
            yAxis: {
              ...getCommonAxisConfig()
            },
            label: {
              position: 'right',
              formatter: datum => formatNumber(datum.value)
            },
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatNumber(datum.value) }
              }
            }
          })
        )
        modelBarChart.render()
      }
    } else {
      modelBarChart.changeData(sortedModelUsage)
      modelBarChart.render()
    }

    const sortedModelTokenUsage = Array.from(
      groupedStatsRaw.value.reduce((map, row) => {
        const key = row.backendModel || '-'
        map.set(
          key,
          (map.get(key) || 0) +
            Number(row.totalInputTokens || 0) +
            Number(row.totalOutputTokens || 0)
        )
        return map
      }, new Map())
    )
      .map(([type, value]) => ({ type, value: Number(value) }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 10)

    if (!modelTokenBarChart) {
      const container = document.getElementById('model-token-usage-bar')
      if (container) {
        modelTokenBarChart = markRaw(
          new Bar('model-token-usage-bar', {
            data: sortedModelTokenUsage,
            xField: 'value',
            yField: 'type',
            seriesField: 'type',
            legend: false,
            xAxis: {
              ...getCommonAxisConfig(),
              label: {
                formatter: val => formatAxisValue(val)
              }
            },
            yAxis: {
              ...getCommonAxisConfig()
            },
            label: {
              position: 'right',
              formatter: datum => formatTokens(datum.value)
            },
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatTokens(datum.value) }
              }
            }
          })
        )
        modelTokenBarChart.render()
      }
    } else {
      modelTokenBarChart.changeData(sortedModelTokenUsage)
      modelTokenBarChart.render()
    }

    const sortedProviderTokenUsage = Array.from(
      groupedStatsRaw.value.reduce((map, row) => {
        const key = row.provider || '-'
        map.set(
          key,
          (map.get(key) || 0) +
            Number(row.totalInputTokens || 0) +
            Number(row.totalOutputTokens || 0)
        )
        return map
      }, new Map())
    )
      .map(([type, value]) => ({ type, value: Number(value) }))
      .sort((a, b) => b.value - a.value)

    if (!providerTokenBarChart) {
      const container = document.getElementById('provider-token-usage-bar')
      if (container) {
        providerTokenBarChart = markRaw(
          new Bar('provider-token-usage-bar', {
            data: sortedProviderTokenUsage,
            xField: 'value',
            yField: 'type',
            seriesField: 'type',
            legend: false,
            xAxis: {
              ...getCommonAxisConfig(),
              label: {
                formatter: val => formatAxisValue(val)
              }
            },
            yAxis: {
              ...getCommonAxisConfig()
            },
            label: {
              position: 'right',
              formatter: datum => formatTokens(datum.value)
            },
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatTokens(datum.value) }
              }
            }
          })
        )
        providerTokenBarChart.render()
      }
    } else {
      providerTokenBarChart.changeData(sortedProviderTokenUsage)
      providerTokenBarChart.render()
    }

    const sortedErrorDist = (errorDist || [])
      .map(item => ({ ...item, value: Number(item.value) }))
      .sort((a, b) => b.value - a.value)

    if (!errorBarChart) {
      const container = document.getElementById('error-dist-bar')
      if (container) {
        errorBarChart = markRaw(
          new Bar('error-dist-bar', {
            data: sortedErrorDist,
            xField: 'value',
            yField: 'type',
            seriesField: 'type',
            legend: false,
            color: getCssVar('--cs-error-color') || '#f56c6c',
            xAxis: {
              ...getCommonAxisConfig(),
              label: {
                formatter: val => formatAxisValue(val)
              }
            },
            yAxis: {
              ...getCommonAxisConfig()
            },
            label: {
              position: 'right',
              formatter: datum => formatNumber(datum.value)
            },
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatNumber(datum.value) }
              }
            }
          })
        )
        errorBarChart.render()
      }
    } else {
      errorBarChart.changeData(sortedErrorDist)
      errorBarChart.render()
    }
  } catch (error) {
    console.error('Failed to update charts:', error)
  }
}

const fetchProviderStats = async (date, force = false) => {
  if (providerStats.value[date] && !force) return

  providerLoading.value = { ...providerLoading.value, [date]: true }

  try {
    const stats = await invokeWrapper('get_ccproxy_provider_stats_by_date', { date })
    providerStatsRaw.value = { ...providerStatsRaw.value, [date]: stats || [] }
    providerStats.value = {
      ...providerStats.value,
      [date]: enrichProviderRows(stats || [])
    }
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
    days === -1
      ? t('settings.proxy.stats.deleteAllConfirm')
      : t('settings.proxy.stats.deleteConfirm', { days })

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

watch(autoRefreshEnabled, val => {
  localStorage.setItem(STORAGE_KEY_AUTO_REFRESH, val ? 'true' : 'false')
  if (val) {
    startRefreshTimer()
  } else {
    if (refreshTimer) {
      clearTimeout(refreshTimer)
      refreshTimer = null
    }
  }
})

watch(
  () => modelStore.providers,
  () => {
    pricingMaps.value = buildPricingMaps(modelStore.providers)
    syncDailyStatsWithCosts()
    providerStats.value = Object.fromEntries(
      Object.entries(providerStatsRaw.value).map(([date, rows]) => [date, enrichProviderRows(rows)])
    )
    if (dailyStats.value.length) {
      updateCharts()
    }
  },
  { deep: true }
)

onMounted(() => {
  fetchDailyStats()
})

onUnmounted(() => {
  if (refreshTimer) clearTimeout(refreshTimer)
  if (modelBarChart) modelBarChart.destroy()
  if (modelTokenBarChart) modelTokenBarChart.destroy()
  if (providerTokenBarChart) providerTokenBarChart.destroy()
  if (errorBarChart) errorBarChart.destroy()
  if (tokenBarChart) tokenBarChart.destroy()
  if (costLineChart) costLineChart.destroy()
  if (requestsDualAxisChart) requestsDualAxisChart.destroy()
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
  grid-template-columns: repeat(5, 1fr);
  gap: var(--cs-space-md);
  margin-bottom: var(--cs-space-lg);
  padding: 4px;

  @media (max-width: 1200px) {
    grid-template-columns: repeat(3, 1fr);
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

      > div {
        height: 380px;
      }
    }
  }
}

.rank-bar-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.rank-bar-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

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

.rank-bar-track {
  width: 100%;
  height: 10px;
  background: var(--cs-bg-color-light);
  border-radius: 999px;
  overflow: hidden;
}

.rank-bar-track {
  height: 8px;
}

.rank-bar-fill {
  height: 100%;
  border-radius: inherit;
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
