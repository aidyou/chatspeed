<template>
  <div class="card">
    <div class="title">Scraper Test</div>
    <div class="list">
      <div class="item">
        <div class="label">Scraper Type</div>
        <div class="value">
          <el-radio-group v-model="requestType">
            <el-radio label="content">Content</el-radio>
            <el-radio label="search">Search</el-radio>
          </el-radio-group>
        </div>
      </div>

      <template v-if="requestType === 'content'">
        <div class="item">
          <div class="label">Url</div>
          <div class="value">
            <el-input v-model="url" />
          </div>
        </div>
      </template>

      <template v-if="requestType === 'search'">
        <div class="item">
          <div class="label">Provider</div>
          <div class="value">
            <el-select v-model="provider">
              <el-option label="bing" value="bing"></el-option>
              <el-option label="google" value="google"></el-option>
              <el-option label="baidu" value="baidu"></el-option>
            </el-select>
          </div>
        </div>
        <div class="item">
          <div class="label">Keyword</div>
          <div class="value">
            <el-input v-model="keyword" />
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
          <el-input
            type="textarea"
            :rows="10"
            v-model="result"
            readonly
            resize="vertical"></el-input>
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
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { showMessage } from '@/libs/util'

const requestType = ref('content')
const url = ref('https://v2.tauri.app/start/')
const provider = ref('bing')
const keyword = ref('tauri framework')
const loading = ref(false)
const result = ref(null)
const error = ref(null)

const runTest = async () => {
  loading.value = true
  result.value = null
  error.value = null

  let params
  if (requestType.value === 'content') {
    if (!url.value) {
      showMessage('Url Required', 'error')
      loading.value = false
      return
    }
    params = { type: 'content', url: url.value }
  } else if (requestType.value === 'search') {
    if (!provider.value || !keyword.value) {
      showMessage('Provider and Keyword Required', 'error')
      loading.value = false
      return
    }
    params = { type: 'search', provider: provider.value, query: keyword.value }
  }

  try {
    const response = await invoke('test_scrape', { requestData: params })
    result.value = response
  } catch (e) {
    error.value = e
    showMessage('Test Failed: ' + e.message)
  } finally {
    loading.value = false
  }
}
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
