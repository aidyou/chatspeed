import { FrontendAppError, invokeWrapper } from '@/libs/tauri'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ElMessage } from 'element-plus'
import { defineStore } from 'pinia'
import { ref } from 'vue'

import { csStorageKey } from '@/config/config'
import { csGetStorage, csRemoveStorage, csSetStorage } from '@/libs/util'

/**
 * Store for managing application update state and operations.
 * Handles silent update checks, background downloads, and restart prompts.
 */
export const useUpdateStore = defineStore('update', () => {
  // State for managing update process
  const versionInfo = ref(null)
  const downloadProgress = ref(0)
  const downloadError = ref('')
  const isUpdateReady = ref(false)
  const ignoredVersion = ref(csGetStorage(csStorageKey.ignoreVersion))
  const appWindow = getCurrentWindow()

  // Event handlers for update process
  const handleUpdateAvailable = (payload) => {
    // If the user chose to ignore this version, do nothing.
    if (ignoredVersion.value === payload.version) {
      console.log(`Update available for ignored version: ${payload.version}`)
      return
    }
    console.log(`Update available: ${payload.version}. Download will start in the background.`)
    versionInfo.value = payload
    isUpdateReady.value = false // Reset ready state for the new update
  }

  const handleDownloadProgress = (payload) => {
    // The backend now sends a structured object.
    if (typeof payload === 'object' && payload.progress !== undefined) {
      // payload: { progress: 0.5, current: 1024, total: 2048 }
      const progress = Math.floor((payload.progress || 0) * 100)
      downloadProgress.value = Math.max(0, Math.min(100, progress))
    }
    downloadError.value = ''
    console.log('Download progress:', `${downloadProgress.value}%`)
  }

  const handleUpdateReady = () => {
    console.log('Update downloaded and ready to be installed.')
    isUpdateReady.value = true
    downloadProgress.value = 100 // Ensure progress is at 100%
    // Clear ignored version when update is ready
    ignoredVersion.value = null
    csRemoveStorage(csStorageKey.ignoreVersion)
  }

  const restartApp = async () => {
    console.log('User requested to restart and install the update.')
    try {
      await invokeWrapper('install_and_restart')
      // The backend will restart the app, so no need to call relaunch()
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to restart application: ${error.toFormattedString()}`, error.originalError);
        ElMessage.error(`${error.toFormattedString()}`);
      } else {
        console.error('Failed to restart application:', error)
        ElMessage.error(`${error.message || 'Unknown error'}`)
      }
    }
  }

  // Allows user to ignore the current update until the next one.
  const skipCurrentUpdate = () => {
    if (versionInfo.value) {
      console.log(`Ignoring version: ${versionInfo.value.version}`)
      ignoredVersion.value = versionInfo.value.version
      csSetStorage(csStorageKey.ignoreVersion, versionInfo.value.version)
      // Reset state as we are ignoring this update
      isUpdateReady.value = false
      versionInfo.value = null
    }
  }


  appWindow.listen('update://available', ({ payload }) => {
    handleUpdateAvailable(payload)
  })

  appWindow.listen('update://download-progress', ({ payload }) => {
    handleDownloadProgress(payload)
  })

  appWindow.listen('update://ready', () => {
    handleUpdateReady()
  })

  return {
    // State
    versionInfo,
    downloadProgress,
    downloadError,
    isUpdateReady,
    ignoredVersion,

    // Actions
    handleUpdateAvailable,
    handleDownloadProgress,
    handleUpdateReady,
    restartApp,
    skipCurrentUpdate
  }
})
