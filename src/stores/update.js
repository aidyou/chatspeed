import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessage } from 'element-plus'

import { csStorageKey } from '@/config/config'
import { csGetStorage, csSetStorage, csRemoveStorage } from '@/libs/util'

/**
 * Store for managing application update state and operations.
 * Handles update notifications, download progress, and restart process.
 */
export const useUpdateStore = defineStore('update', () => {
  // State for managing update process
  const versionInfo = ref(null)
  const downloadProgress = ref(0)
  const downloadError = ref('')
  const showUpdateDialog = ref(false)
  const showProgressDialog = ref(false)
  const showRestartDialog = ref(false)
  const ignoredVersion = ref(csGetStorage(csStorageKey.ignoreVersion))

  // Event handlers for update process
  const handleUpdateAvailable = (payload) => {
    // If the user chooses to ignore this version, do not display the update dialog
    if (ignoredVersion.value === payload.version) {
      return
    }
    versionInfo.value = payload
    showUpdateDialog.value = true
  }

  const handleDownloadProgress = (payload) => {
    // Handle both old string format and new object format
    let progress = 0
    if (typeof payload === 'string') {
      // Old format: "50"
      progress = parseInt(payload) || 0
    } else if (typeof payload === 'object' && payload.progress !== undefined) {
      // New format: { progress: 0.5, current: 1024, total: 2048 }
      progress = Math.floor((payload.progress || 0) * 100)
    } else if (typeof payload === 'number') {
      // Direct number format
      progress = Math.floor(payload)
    }

    downloadProgress.value = Math.max(0, Math.min(100, progress))
    downloadError.value = ''
    console.log('Download progress:', downloadProgress.value + '%')
  }

  const handleUpdateReady = () => {
    showProgressDialog.value = false
    showRestartDialog.value = true
    // Clear ignored version when update is ready
    ignoredVersion.value = null
    csRemoveStorage(csStorageKey.ignoreVersion)
  }

  // User interaction handlers
  const confirmUpdate = async () => {
    try {
      showUpdateDialog.value = false
      showProgressDialog.value = true
      downloadProgress.value = 0
      downloadError.value = ''

      // Clear ignored version when user confirms update
      ignoredVersion.value = null
      csRemoveStorage(csStorageKey.ignoreVersion)

      await invoke('confirm_update', { versionInfo: versionInfo.value })
    } catch (error) {
      console.error('Update failed:', error)
      downloadError.value = error.message || '下载或安装更新失败'
      showProgressDialog.value = false
      ElMessage.error(`下载或安装更新失败: ${error.message || '未知错误'}`)
    }
  }

  const cancelUpdate = ({ skip = false } = {}) => {
    // Save ignored version to local storage only if skip is true
    if (skip && versionInfo.value) {
      ignoredVersion.value = versionInfo.value.version
      csSetStorage(csStorageKey.ignoreVersion, versionInfo.value.version)
    }
    showUpdateDialog.value = false
    showProgressDialog.value = false
    downloadProgress.value = 0
    downloadError.value = ''
  }

  const restartApp = async () => {
    try {
      await invoke('restart_app')
    } catch (error) {
      ElMessage.error(error.message)
    }
  }

  const postponeRestart = () => {
    showRestartDialog.value = false
  }

  // Reset all update-related states
  const resetDialogs = () => {
    showUpdateDialog.value = false
    showProgressDialog.value = false
    showRestartDialog.value = false
    downloadProgress.value = 0
    downloadError.value = ''
  }

  return {
    // State
    versionInfo,
    downloadProgress,
    downloadError,
    showUpdateDialog,
    showProgressDialog,
    showRestartDialog,
    ignoredVersion,

    // Actions
    handleUpdateAvailable,
    handleDownloadProgress,
    handleUpdateReady,
    confirmUpdate,
    cancelUpdate,
    restartApp,
    postponeRestart,
    resetDialogs
  }
})
