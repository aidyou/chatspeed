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
                show-overflow-tooltip>
                <template #default="scope">
                  <span style="color: var(--cs-color-primary); font-weight: bold">{{
                    scope.row.clientModel
                    }}</span>
                </template>
              </el-table-column>
              <el-table-column prop="backendModel" :label="$t('settings.proxy.stats.backendModel')" min-width="200"
                show-overflow-tooltip />
              <el-table-column prop="protocol" :label="$t('settings.proxy.stats.protocol')" width="90">
                <template #default="scope">
                  <el-tag size="small" effect="plain">{{ scope.row.protocol }}</el-tag>
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
              <el-table-column prop="requestCount" :label="$t('settings.proxy.stats.requests')" width="80" />
              <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="90">
                <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="90">
                <template #default="scope">{{
                  formatTokens(scope.row.totalOutputTokens)
                  }}</template>
              </el-table-column>
                            <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="90">
                              <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
                            </el-table-column>
                            <el-table-column :label="$t('settings.proxy.stats.errors')" width="80">
                              <template #default="scope">
                                <el-link
                                  v-if="scope.row.errorCount > 0"
                                  type="danger"
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
                    <el-table-column prop="errorCount" :label="$t('settings.proxy.stats.errors')" width="100" />
                  </el-table>
              
                  <div class="charts-section">
                    <!-- 1. Trend chart (full width) -->
                    <div class="charts-row">
                      <div class="chart-card bar-chart">
                        <h4>{{ $t('settings.proxy.stats.dailyTokensTitle') }}</h4>
                        <div id="daily-tokens-column"></div>
                      </div>
                    </div>
              
                    <!-- 2. Distribution charts (side by side) -->
                    <div class="charts-row">
                      <div class="chart-card pie-chart">
                        <h4>{{ $t('settings.proxy.stats.modelUsageTitle') }}</h4>
                        <div id="model-usage-pie"></div>
                      </div>
                      <div class="chart-card pie-chart">
                        <h4>{{ $t('settings.proxy.stats.modelTokenUsageTitle') }}</h4>
                        <div id="model-token-usage-pie"></div>
                      </div>
                      <div class="chart-card pie-chart">
                        <h4>{{ $t('settings.proxy.stats.errorDistTitle') }}</h4>
                        <div id="error-dist-pie"></div>
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
import { ref, onMounted, onUnmounted, watch, nextTick, markRaw } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { Refresh, Delete } from '@element-plus/icons-vue'
import { Pie, Column } from '@antv/g2plot'
import { showMessage } from '@/libs/util'
import { ElMessageBox } from 'element-plus'

const { t } = useI18n()

const loading = ref(false)
const selectedDays = ref(0)
const autoRefreshEnabled = ref(false)
const dailyStats = ref([])
// Use reactive to ensure reactivity when dynamically adding keys
const providerStats = ref({})
const providerLoading = ref({})
const expandedDates = ref(new Set())

const errorDialogVisible = ref(false)
const errorLoading = ref(false)
const errorStats = ref([])
const selectedErrorDate = ref('')

// Chart instances
let modelPieChart = null
let modelTokenPieChart = null
let errorPieChart = null
let tokenBarChart = null
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

    // Ensure DOM is updated before rendering charts
    await nextTick()
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
    const [modelUsage, modelTokenUsage, errorDist] = await Promise.all([
      invokeWrapper('get_ccproxy_model_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_model_token_usage_stats', { days: selectedDays.value }),
      invokeWrapper('get_ccproxy_error_distribution_stats', { days: selectedDays.value })
    ])

    // Prepare token data for stacked bar chart
    const tokenData = []
    if (dailyStats.value && dailyStats.value.length > 0) {
      dailyStats.value
        .slice()
        .reverse()
        .forEach(day => {
          tokenData.push({
            date: day.date,
            type: t('settings.proxy.stats.inputTokens'),
            value: Number(day.totalInputTokens || 0)
          })
          tokenData.push({
            date: day.date,
            type: t('settings.proxy.stats.outputTokens'),
            value: Number(day.totalOutputTokens || 0)
          })
          tokenData.push({
            date: day.date,
            type: t('settings.proxy.stats.cacheTokens'),
            value: Number(day.totalCacheTokens || 0)
          })
        })
    }

    // Render or update daily token trend chart (Column)
    if (!tokenBarChart) {
      const container = document.getElementById('daily-tokens-column')
      if (container) {
        tokenBarChart = markRaw(
          new Column('daily-tokens-column', {
            data: tokenData,
            isStack: true,
            xField: 'date',
            yField: 'value',
            seriesField: 'type',
            legend: {
              position: 'bottom'
            },
            label: {
              position: 'middle',
              layout: [{ type: 'interval-adjust-position' }, { type: 'interval-hide-overlap' }],
              formatter: datum => formatTokens(datum.value)
            },
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatTokens(datum.value) }
              }
            }
          })
        )
        tokenBarChart.render()
      }
    } else {
      tokenBarChart.changeData(tokenData)
      tokenBarChart.render()
    }

    // Render or update model usage pie chart
    if (!modelPieChart) {
      const container = document.getElementById('model-usage-pie')
      if (container) {
        modelPieChart = markRaw(
          new Pie('model-usage-pie', {
            appendPadding: 10,
            data: modelUsage || [],
            angleField: 'value',
            colorField: 'type',
            radius: 0.8,
            label: { type: 'outer', content: '{name}: {percentage}' },
            interactions: [{ type: 'element-active' }]
          })
        )
        modelPieChart.render()
      }
    } else {
      modelPieChart.changeData(modelUsage || [])
      modelPieChart.render()
    }

    // Render or update model token usage pie chart
    if (!modelTokenPieChart) {
      const container = document.getElementById('model-token-usage-pie')
      if (container) {
        modelTokenPieChart = markRaw(
          new Pie('model-token-usage-pie', {
            appendPadding: 10,
            data: modelTokenUsage || [],
            angleField: 'value',
            colorField: 'type',
            radius: 0.8,
            label: { type: 'outer', content: '{name}: {percentage}' },
            interactions: [{ type: 'element-active' }],
            tooltip: {
              formatter: datum => {
                return { name: datum.type, value: formatTokens(datum.value) }
              }
            }
          })
        )
        modelTokenPieChart.render()
      }
    } else {
      modelTokenPieChart.changeData(modelTokenUsage || [])
      modelTokenPieChart.render()
    }

    // Render or update error distribution pie chart
    if (!errorPieChart) {
      const container = document.getElementById('error-dist-pie')
      if (container) {
        errorPieChart = markRaw(
          new Pie('error-dist-pie', {
            appendPadding: 10,
            data: errorDist || [],
            angleField: 'value',
            colorField: 'type',
            radius: 0.8,
            label: { type: 'outer', content: '{name}: {value}' },
            interactions: [{ type: 'element-active' }]
          })
        )
        errorPieChart.render()
      }
    } else {
      errorPieChart.changeData(errorDist || [])
      errorPieChart.render()
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
  if (modelPieChart) modelPieChart.destroy()
  if (modelTokenPieChart) modelTokenPieChart.destroy()
  if (errorPieChart) errorPieChart.destroy()
  if (tokenBarChart) tokenBarChart.destroy()
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

.charts-section {
  margin-top: var(--cs-space-lg);
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-md);
}

.charts-row {
  display: flex;
  gap: var(--cs-space-md);
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

  div {
    height: 300px;
  }

  &.bar-chart {
    min-width: 100%;

    div {
      height: 350px;
    }
  }
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
