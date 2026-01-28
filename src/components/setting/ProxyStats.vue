<template>
  <div class="proxy-stats">
    <div class="stats-header">
      <h3>{{ $t('settings.proxy.stats.title') }}</h3>
      <div class="header-actions">
        <el-button :icon="Refresh" circle size="small" @click="fetchDailyStats" :loading="loading" />
        <el-select v-model="selectedDays" size="small" @change="fetchDailyStats"
          style="width: 120px; margin-left: 10px">
          <el-option :label="$t('settings.proxy.stats.last1Day')" :value="1" />
          <el-option :label="$t('settings.proxy.stats.last7Days')" :value="7" />
          <el-option :label="$t('settings.proxy.stats.last30Days')" :value="30" />
          <el-option :label="$t('settings.proxy.stats.last90Days')" :value="90" />
        </el-select>
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
                  <span style="color: var(--cs-color-primary); font-weight: bold">{{ scope.row.clientModel }}</span>
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
                    {{ scope.row.toolCompatMode === 1 ? $t('settings.proxy.stats.yes') : $t('settings.proxy.stats.no')
                    }}
                  </el-tag>
                </template>
              </el-table-column>
              <el-table-column prop="requestCount" :label="$t('settings.proxy.stats.requests')" width="80" />
              <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="90">
                <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="90">
                <template #default="scope">{{ formatTokens(scope.row.totalOutputTokens) }}</template>
              </el-table-column>
              <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="90">
                <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
              </el-table-column>
              <el-table-column prop="errorCount" :label="$t('settings.proxy.stats.errors')" width="80" />
            </el-table>
          </div>
        </template>
      </el-table-column>
      <el-table-column prop="date" :label="$t('settings.proxy.stats.date')" width="110" />
      <el-table-column prop="providerCount" :label="$t('settings.proxy.stats.providers')" width="90" align="center" />
      <el-table-column prop="topProvider" :label="$t('settings.proxy.stats.topProvider')" min-width="180"
        show-overflow-tooltip />
      <el-table-column :label="$t('settings.proxy.stats.inputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalInputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.outputTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalOutputTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.cacheTokens')" width="100">
        <template #default="scope">{{ formatTokens(scope.row.totalCacheTokens) }}</template>
      </el-table-column>
      <el-table-column :label="$t('settings.proxy.stats.errors')" width="100">
        <template #default="scope">
          <el-link v-if="scope.row.errorCount > 0" type="danger" @click="showErrorDetail(scope.row.date)">
            {{ scope.row.errorCount }}
          </el-link>
          <span v-else>0</span>
        </template>
      </el-table-column>
    </el-table>

    <div class="charts-section">
      <!-- 1. 趋势图 (全宽) -->
      <div class="charts-row">
        <div class="chart-card bar-chart">
          <h4>{{ $t('settings.proxy.stats.dailyTokensTitle') }}</h4>
          <div id="daily-tokens-column"></div>
        </div>
      </div>

      <!-- 2. 分布图 (并排) -->
      <div class="charts-row">
        <div class="chart-card pie-chart">
          <h4>{{ $t('settings.proxy.stats.modelUsageTitle') }}</h4>
          <div id="model-usage-pie"></div>
        </div>
        <div class="chart-card pie-chart">
          <h4>{{ $t('settings.proxy.stats.errorDistTitle') }}</h4>
          <div id="error-dist-pie"></div>
        </div>
      </div>
    </div>

    <!-- Error Detail Dialog -->
    <el-dialog v-model="errorDialogVisible"
      :title="$t('settings.proxy.stats.errorDetailTitle', { date: selectedErrorDate })" width="600px" append-to-body>
      <el-table :data="errorStats" size="small" border v-loading="errorLoading">
        <el-table-column prop="statusCode" :label="$t('settings.proxy.stats.statusCode')" width="100" />
        <el-table-column prop="errorMessage" :label="$t('settings.proxy.stats.errorMessage')" />
        <el-table-column prop="errorCount" :label="$t('settings.proxy.stats.count')" width="100" />
      </el-table>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@element-plus/icons-vue'
import { Pie, Column } from '@antv/g2plot'

const { t } = useI18n()

const loading = ref(false)
const selectedDays = ref(7)
const dailyStats = ref([])
// 使用 reactive 确保动态添加 key 时的响应式
const providerStats = ref({})
const providerLoading = ref({})

const errorDialogVisible = ref(false)
const errorLoading = ref(false)
const errorStats = ref([])
const selectedErrorDate = ref('')

// Chart instances
let modelPieChart = null
let errorPieChart = null
let tokenBarChart = null

const formatTokens = (val) => {
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

const fetchDailyStats = async () => {
  loading.value = true
  dailyStats.value = []
  providerStats.value = {}
  providerLoading.value = {}
  try {
    const res = await invokeWrapper('get_ccproxy_daily_stats', { days: selectedDays.value })
    dailyStats.value = res || []
    // 确保 DOM 更新后再渲染图表
    await nextTick()
    await updateCharts()
  } catch (error) {
    console.error('Failed to fetch proxy stats:', error)
  } finally {
    loading.value = false
  }
}

const updateCharts = async () => {
  try {
    const modelUsage = await invokeWrapper('get_ccproxy_model_usage_stats', { days: selectedDays.value })
    const errorDist = await invokeWrapper('get_ccproxy_error_distribution_stats', { days: selectedDays.value })

    // Prepare token data for stacked bar chart
    const tokenData = []
    if (dailyStats.value && dailyStats.value.length > 0) {
      dailyStats.value.slice().reverse().forEach(day => {
        tokenData.push({ date: day.date, type: t('settings.proxy.stats.inputTokens'), value: Number(day.totalInputTokens || 0) })
        tokenData.push({ date: day.date, type: t('settings.proxy.stats.outputTokens'), value: Number(day.totalOutputTokens || 0) })
        tokenData.push({ date: day.date, type: t('settings.proxy.stats.cacheTokens'), value: Number(day.totalCacheTokens || 0) })
      })
    }

    // 渲染或更新每日 Token 趋势图 (Column)
    if (!tokenBarChart) {
      const container = document.getElementById('daily-tokens-column');
      if (container) {
        tokenBarChart = new Column('daily-tokens-column', {
          data: tokenData,
          isStack: true,
          xField: 'date',
          yField: 'value',
          seriesField: 'type',
          label: {
            position: 'middle',
            layout: [{ type: 'interval-adjust-position' }, { type: 'interval-hide-overlap' }],
            formatter: (datum) => formatTokens(datum.value)
          },
          tooltip: {
            formatter: (datum) => {
              return { name: datum.type, value: formatTokens(datum.value) };
            },
          },
        })
        tokenBarChart.render()
      }
    } else {
      tokenBarChart.changeData(tokenData)
    }

    // 渲染或更新模型比例图 (Pie)
    if (!modelPieChart) {
      const container = document.getElementById('model-usage-pie');
      if (container) {
        modelPieChart = new Pie('model-usage-pie', {
          appendPadding: 10,
          data: modelUsage || [],
          angleField: 'value',
          colorField: 'type',
          radius: 0.8,
          label: { type: 'outer', content: '{name}: {percentage}' },
          interactions: [{ type: 'element-active' }],
        })
        modelPieChart.render()
      }
    } else {
      modelPieChart.changeData(modelUsage || [])
    }

    // 渲染或更新错误分布图 (Pie)
    if (!errorPieChart) {
      const container = document.getElementById('error-dist-pie');
      if (container) {
        errorPieChart = new Pie('error-dist-pie', {
          appendPadding: 10,
          data: errorDist || [],
          angleField: 'value',
          colorField: 'type',
          radius: 0.8,
          label: { type: 'outer', content: '{name}: {value}' },
          interactions: [{ type: 'element-active' }],
        })
        errorPieChart.render()
      }
    } else {
      errorPieChart.changeData(errorDist || [])
    }
  } catch (error) {
    console.error('Failed to update charts:', error)
  }
}

const fetchProviderStats = async (date) => {
  if (providerStats.value[date]) return

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

const showErrorDetail = async (date) => {
  selectedErrorDate.value = date
  errorDialogVisible.value = true
  errorLoading.value = true
  try {
    const stats = await invokeWrapper('get_ccproxy_error_stats_by_date', { date })
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
    fetchProviderStats(row.date)
  }
}

onMounted(() => {
  fetchDailyStats()
})

onUnmounted(() => {
  if (modelPieChart) modelPieChart.destroy()
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
</style>
