<template>
  <div class="card">
    <div class="title">
      <span>Scraper Test</span>
      <!-- <el-tooltip content="Open Schema dir" placement="top">
        <span class="icon" @click="openSchema">
          <cs name="add" />
        </span>
      </el-tooltip> -->
    </div>

    <div class="list">
      <div class="item">
        <div class="label">Scraper Type</div>
        <div class="value">
          <el-radio-group v-model="requestType">
            <el-radio label="search">Search Result</el-radio>
            <el-radio label="normal">Normal</el-radio>
            <el-radio label="content">Extract Content</el-radio>
          </el-radio-group>
        </div>
      </div>

      <template v-if="requestType !== 'search'">
        <div class="item">
          <div class="label">Url</div>
          <div class="value">
            <el-input v-model="url" />
          </div>
        </div>
        <div class="item">
          <div class="label">Format</div>
          <div class="value">
            <el-select v-model="format">
              <el-option label="markdown" value="markdown"></el-option>
              <el-option label="text" value="text"></el-option>
            </el-select>
          </div>
        </div>
        <div class="item">
          <div class="label">Keep Link</div>
          <div class="value">
            <el-switch v-model="keepLink" />
          </div>
        </div>
        <div class="item">
          <div class="label">Keep Image</div>
          <div class="value">
            <el-switch v-model="keepImage" />
          </div>
        </div>
      </template>

      <template v-if="requestType === 'search'">
        <div class="item">
          <div class="label">Provider</div>
          <div class="value">
            <el-select v-model="provider">
              <el-option label="bing" value="bing"></el-option>
              <el-option label="duckduckgo" value="duckduckgo"></el-option>
              <el-option label="brave" value="brave"></el-option>
              <el-option label="so" value="so"></el-option>
              <el-option label="sogou" value="sogou"></el-option>
            </el-select>
          </div>
        </div>
        <div class="item">
          <div class="label">Keyword</div>
          <div class="value">
            <el-input v-model="keyword" />
          </div>
        </div>
        <div class="item">
          <div class="label">Page</div>
          <div class="value">
            <el-input v-model="page" type="number" />
          </div>
        </div>
        <div class="item">
          <div class="label">Number</div>
          <div class="value">
            <el-input v-model="number" max="10" type="number" />
          </div>
        </div>
        <div class="item">
          <div class="label">Time Period</div>
          <div class="value">
            <el-select v-model="timePeriod">
              <el-option label="unset" value=""></el-option>
              <el-option label="day" value="day"></el-option>
              <el-option label="week" value="week"></el-option>
              <el-option label="month" value="month"></el-option>
              <el-option label="year" value="year"></el-option>
            </el-select>
          </div>
        </div>
      </template>

      <div class="item">
        <div class="label"></div>
        <div class="value">
          <el-button type="primary" @click="runTest" :loading="loading">Run Test</el-button>
        </div>
      </div>

      <div class="item" v-if="result">
        <div class="label">Scraper Result</div>
        <div class="value">
          <el-input type="textarea" :rows="10" v-model="result" readonly resize="vertical" />
        </div>
      </div>
      <div class="item" v-if="content">
        <div class="label">Scraper Content</div>
        <div class="value">
          <el-input type="textarea" :rows="10" v-model="content" readonly resize="vertical" />
        </div>
      </div>
      <div class="item" v-if="error">
        <div class="label">Scraper Error</div>
        <div class="value">
          <el-input
            type="textarea"
            :rows="5"
            v-model="error"
            readonly
            resize="vertical"
            class="error-textarea"></el-input>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { showMessage } from '@/libs/util'

// import { openPath } from '@tauri-apps/plugin-opener'
// import { useSettingStore } from '@/stores/setting'
// import { storeToRefs } from 'pinia'
// const settingStore = useSettingStore()
// const { env } = storeToRefs(settingStore)

const requestType = ref('search')
const url = ref('https://chatspeed.aidyou.ai/')
const format = ref('markdown')
const keepLink = ref(true)
const keepImage = ref(false)
const provider = ref('bing')
const keyword = ref('tauri framework')
const page = ref(1)
const number = ref(5)
const timePeriod = ref('')
const loading = ref(false)
const result = ref(null)
const error = ref(null)

const content = computed(() => {
  if (!result.value) return null

  try {
    if (typeof result.value === 'object') {
      return result.value.content || ''
    }
    if (typeof result.value === 'string') {
      const json = JSON.parse(result.value)
      return json.content || ''
    }
  } catch (e) {
    console.error('Failed to parse result:', e)
    return ''
  }

  return ''
})

const runTest = async () => {
  loading.value = true
  result.value = null
  error.value = null

  let params
  if (requestType.value !== 'search') {
    if (!url.value) {
      showMessage('Url Required', 'error')
      loading.value = false
      return
    }
    params = {
      type: requestType.value,
      url: url.value,
      format: format.value,
      keep_link: keepLink.value,
      keep_image: keepImage.value
    }
  } else if (requestType.value === 'search') {
    if (!provider.value || !keyword.value) {
      showMessage('Provider and Keyword Required', 'error')
      loading.value = false
      return
    }
    params = {
      type: 'search',
      provider: provider.value,
      query: keyword.value,
      page: Number(page.value || 1),
      number: Number(number.value || 10),
      time_period: timePeriod.value || ''
    }
  }

  try {
    const response = await invoke('test_scrape', { requestData: params })
    result.value = requestType.value === 'search' ? response : JSON.parse(response)
  } catch (e) {
    error.value = e
    showMessage('Test Failed: ' + e.message)
  } finally {
    loading.value = false
  }
}

// const openSchema = async () => {
//   await openPath(env.value.schemaDir)
// }
</script>

<style lang="scss" scoped>
.error-textarea {
  :deep(.el-textarea__inner) {
    color: var(--el-color-danger);
  }
}

.list {
  .item {
    .label {
      flex: 1;
    }

    .value {
      flex: 4;
      display: flex;
      justify-content: flex-end;
    }
  }
}
</style>
