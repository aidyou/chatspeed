<template>
  <div class="proxy-stats">
    <div class="stats-header">
      <h3>{{ $t('settings.proxy.stats.title') }}</h3>
      <div class="header-actions">
        <el-button :icon="Refresh" circle size="small" @click="fetchDailyStats" :loading="loading" />
        <el-select v-model="selectedDays" size="small" @change="fetchDailyStats"
          style="width: 120px; margin-left: 10px">
          <el-option :label="$t('settings.proxy.stats.last7Days')" :value="7" />
          <el-option :label="$t('settings.proxy.stats.last30Days')" :value="30" />
          <el-option :label="$t('settings.proxy.stats.last90Days')" :value="90" />
        </el-select>
      </div>
    </div>

    <el-table :data="dailyStats" style="width: 100%" v-loading="loading" @expand-change="handleExpandChange">
      <el-table-column type="expand">
        <template #default="props">
          <div class="expand-detail">
            <h4>{{ $t('settings.proxy.stats.providerDetail', { date: props.row.date }) }}</h4>
            <el-table :data="providerStats[props.row.date] || []" size="small" border
              v-loading="providerLoading[props.row.date]">
              <el-table-column prop="provider" :label="$t('settings.proxy.stats.provider')" min-width="120" />
              <el-table-column prop="protocol" :label="$t('settings.proxy.stats.protocol')" width="100">
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
              <el-table-column prop="requestCount" :label="$t('settings.proxy.stats.requests')" width="100" />
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
          </div>
        </template>
      </el-table-column>
      <el-table-column prop="date" :label="$t('settings.proxy.stats.date')" width="110" />
      <el-table-column prop="providerCount" :label="$t('settings.proxy.stats.providers')" width="150" align="center" />
      <el-table-column prop="topProvider" :label="$t('settings.proxy.stats.topProvider')" min-width="120"
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
import { ref, onMounted, watch } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useI18n } from 'vue-i18n'
import { Refresh } from '@element-plus/icons-vue'

const { t } = useI18n()

const loading = ref(false)
const selectedDays = ref(7)
const dailyStats = ref([])
const providerStats = ref({})
const providerLoading = ref({})

const errorDialogVisible = ref(false)
const errorLoading = ref(false)
const errorStats = ref([])
const selectedErrorDate = ref('')

const formatTokens = (val) => {
  if (!val || isNaN(val)) return '0'
  if (val >= 100000000) {
    return (val / 100000000).toFixed(2) + ' 亿'
  }
  if (val >= 100000) {
    return (val / 10000).toFixed(2) + ' 万'
  }
  return val.toLocaleString()
}

const fetchDailyStats = async () => {
  loading.value = true
  try {
    dailyStats.value = await invokeWrapper('get_ccproxy_daily_stats', { days: selectedDays.value })
  } catch (error) {
    console.error('Failed to fetch proxy stats:', error)
  } finally {
    loading.value = false
  }
}

const fetchProviderStats = async (date) => {
  if (providerStats.value[date]) return

  providerLoading.value[date] = true
  try {
    const stats = await invokeWrapper('get_ccproxy_provider_stats_by_date', { date })
    providerStats.value[date] = stats
  } catch (error) {
    console.error('Failed to fetch provider stats:', error)
  } finally {
    providerLoading.value[date] = false
  }
}

const showErrorDetail = async (date) => {
  selectedErrorDate.value = date
  errorDialogVisible.value = true
  errorLoading.value = true
  try {
    errorStats.value = await invokeWrapper('get_ccproxy_error_stats_by_date', { date })
  } catch (error) {
    console.error('Failed to fetch error stats:', error)
  } finally {
    errorLoading.value = false
  }
}

// Watch for row expansion to fetch provider stats
watch(dailyStats, () => {
  // Reset provider stats when main data changes if needed
}, { deep: true })

// Helper to handle expansion manually if needed or use @expand-change
// For simplicity, we can fetch provider stats when a row is expanded
const handleExpandChange = (row, expandedRows) => {
  const isExpanded = expandedRows.some(r => r.date === row.date)
  if (isExpanded) {
    fetchProviderStats(row.date)
  }
}

onMounted(() => {
  fetchDailyStats()
})
</script>

<style lang="scss" scoped>
.proxy-stats {
  margin-top: var(--cs-space);
}

.stats-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--cs-space-md);

  h3 {
    margin: 0;
    font-size: var(--cs-font-size);
    font-weight: 600;
  }
}

.expand-detail {
  padding: var(--cs-space-md);
  background-color: var(--cs-bg-color-light);

  h4 {
    margin-top: 0;
    margin-bottom: var(--cs-space-sm);
    font-size: var(--cs-font-size-md);
  }
}
</style>
