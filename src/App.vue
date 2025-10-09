<template>
  <div class="app-container" :class="[windowType, windowStore.os]">
    <div
      class="titlebar"
      data-tauri-drag-region
      @pointerdown="handleDragStart"
      @pointerup="handleDragEnd" />
    <router-view />
  </div>
</template>

<script setup>
import 'katex/dist/katex.min.css'
import 'element-plus/theme-chalk/dark/css-vars.css'
import '@/style/element/css-vars.css'
import '@/style/chatspeed/style.scss'

import { ref, onMounted, onUnmounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useDark, usePreferredDark } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import { setI18nLanguage } from '@/i18n'

import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { listen } from '@tauri-apps/api/event'

import { ElMessage } from 'element-plus'

import { useSettingStore } from '@/stores/setting'
import { useModelStore } from '@/stores/model'
import { useSkillStore } from '@/stores/skill'
import { useUpdateStore } from '@/stores/update'
import { useWindowStore } from '@/stores/window'
import { useMcpStore } from './stores/mcp'

const router = useRouter()
const settingStore = useSettingStore()
const mcpStore = useMcpStore()
const modelStore = useModelStore()
const skillStore = useSkillStore()
const updateStore = useUpdateStore()
const windowStore = useWindowStore()
const listener = ref(null)

const { settings } = storeToRefs(settingStore)
const windowType = ref('main')

const isSystemDark = usePreferredDark()
const isDark = useDark({
  selector: 'html',
  attribute: 'class',
  valueDark: 'dark',
  valueLight: 'light',
  storageKey: null
})

const updateTrayFlag = ref(false)
watch(
  () => settings.value.interfaceLanguage,
  newLang => {
    console.log('settings.interfaceLanguage changed', newLang)
    setI18nLanguage(newLang)
    if (settingStore.windowLabel === 'main' && !updateTrayFlag.value) {
      updateTrayFlag.value = true
      settingStore
        .updateTray()
        .catch(err => {
          console.error(`Failed to update tray: ${err}`)
        })
        .finally(() => {
          updateTrayFlag.value = false
        })
    }
  },
  { immediate: true }
)

watch(isSystemDark, newVal => {
  if (settings.value.theme === 'system') {
    isDark.value = newVal
  }
})

watch(isDark, newVal => {
  setLighlightTheme(newVal ? 'dark' : 'light')
})

watch(
  () => settings.value.theme,
  newTheme => {
    console.log('settings.theme changed', newTheme)
    setTheme()
  }
)

watch(
  () => settings.value.codeDarkTheme,
  () => {
    console.log('settings.codeDarkTheme changed', settings.value.codeDarkTheme, isDark.value)
    if (isDark.value) {
      setLighlightTheme()
    }
  }
)

watch(
  () => settings.value.codeLightTheme,
  () => {
    console.log('settings.codeLightTheme changed', settings.value.codeLightTheme, isDark.value)
    if (!isDark.value) {
      setLighlightTheme()
    }
  }
)

onMounted(async () => {
  windowType.value = router.currentRoute.value.name

  await settingStore.getEnv(true)
  // update the setting store
  await settingStore.updateSettingStore()

  setTheme()

  if (settingStore.windowLabel === 'main' || settingStore.windowLabel === 'note') {
    // Listen for update events
    await listen('update://available', ({ payload }) => {
      updateStore.handleUpdateAvailable(payload)
    })

    await listen('update://download-progress', ({ payload }) => {
      updateStore.handleDownloadProgress(payload)
    })

    await listen('update://ready', () => {
      updateStore.handleUpdateReady()
    })
  }

  listener.value = await listen('sync_state', event => {
    const eventType = event?.payload?.type || ''

    // Global event handler
    if (eventType === 'mcp_status_changed') {
      mcpStore.updateServerStatus(event.payload.name, event.payload.status)
    } else if (eventType === 'mcp') {
      // Pass the metadata to the store's handler
      if (event.payload?.metadata) {
        mcpStore.handleSyncStateUpdate(event.payload.metadata)
      }
    }

    // Ignore events from current windows
    if (event.payload.windowLabel === getCurrentWebviewWindow().label) {
      return
    }
    console.log('sync_state', event)
    if (eventType === 'model') {
      modelStore.updateModelStore()
    } else if (eventType === 'skill') {
      skillStore.updateSkillStore()
    } else if (eventType === 'setting_changed') {
      settingStore.updateSettingStore(event.payload.setting)
    }
  })

  // listen for setup error
  await listen('setup-error', event => {
    ElMessage.error({
      message: `Setup error: ${event.payload}`,
      duration: 0,
      showClose: true
    })
  })

  window.addEventListener('keydown', handleShortcut)

  // 监听权限请求
  // await listen('accessibility-permission-required', () => {
  //   ElMessageBox.confirm(
  //     'This app needs accessibility permission to monitor text selection. Would you like to open System Settings?',
  //     'Permission Required',
  //     {
  //       confirmButtonText: 'Open Settings',
  //       cancelButtonText: 'Cancel',
  //       type: 'warning'
  //     }
  //   )
  //     .then(() => {
  //       // 调用后端打开系统设置
  //       invoke('open_accessibility_settings').catch(err => {
  //         ElMessage.error(`Failed to open settings: ${err}`)
  //       })
  //     })
  //     .catch(() => {
  //       ElMessage.info('You can grant permission later in System Settings')
  //     })
  // })

  // 监听权限错误
  // await listen('accessibility-error', event => {
  //   ElMessage.error({
  //     message: `Accessibility error: ${event.payload}`,
  //     duration: 0,
  //     showClose: true
  //   })
  // })
  // 监听监控错误
  // await listen('text-monitor-error', event => {
  //   ElMessage.error({
  //     message: `Monitor error: ${event.payload}`,
  //     duration: 0,
  //     showClose: true
  //   })
  // })
})

onUnmounted(() => {
  if (listener.value) {
    listener.value()
  }
  window.removeEventListener('keydown', handleShortcut)
})

const setTheme = () => {
  if (settings.value.theme === 'system') {
    isDark.value = isSystemDark.value
  } else {
    isDark.value = settings.value.theme === 'dark'
  }
  setLighlightTheme()
}

const handleShortcut = async event => {
  // ctrl+(Windows/Linux), common+,(macOS) open setting
  if (event.metaKey || event.ctrlKey) {
    if (event.code === 'Comma') {
      // Invoke the command to open the settings window for model configuration
      invoke('open_setting_window', { settingType: 'general' }).catch(error => {
        console.error('Failed to open settings window:', error)
      })
    }

    // command+w (macOS) or ctrl+w (Windows/Linux) to close window
    else if (event.code === 'KeyW') {
      const currentWindow = getCurrentWebviewWindow()
      const windowLabel = currentWindow.label

      // Only handle the close event of the current window
      if (windowLabel === 'main' || windowLabel === 'settings' || windowLabel === 'note') {
        // Check if the current window is indeed the active window
        const isFocused = await currentWindow.isFocused()
        if (isFocused) {
          currentWindow.close().catch(error => {
            console.error('Failed to close window:', error)
          })
        }
      }
    }
  }
}

const setLighlightTheme = () => {
  const theme = isDark.value ? settings.value.codeDarkTheme : settings.value.codeLightTheme
  // Remove existing styles
  const existingLink = document.querySelector('link[cs-highlight-theme]')
  if (existingLink) {
    document.head.removeChild(existingLink)
  }

  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.setAttribute('cs-highlight-theme', theme)
  link.href = `/highlight.js/${isDark.value ? 'dark' : 'light'}/${theme}.css`
  document.head.appendChild(link)
}

const handleDragStart = e => {
  windowStore.setMouseEventState(true)
}

const handleDragEnd = e => {
  windowStore.setMouseEventState(false)
}
</script>

<style lang="scss">
body {
  font-family: var(--cs-font-family);
  font-weight: 400;
}

.app-container {
  height: 100vh;
  width: 100vw;
  overflow: hidden;
  background: var(--cs-bg-color);
  border-radius: var(--cs-border-radius-md);
  box-sizing: border-box;

  .titlebar {
    height: var(--cs-titlebar-height);
    width: 100%;
    padding: 0;
    -webkit-user-select: none;
    user-select: none;
    background: transparent;
    box-sizing: border-box;
    border-radius: var(--cs-border-radius-md) var(--cs-border-radius-md) 0 0;
    position: fixed;
    top: 0;
    left: 0;
    z-index: var(--cs-titlebar-zindex);
  }

  &.linux,
  &.windows {
    // Correct border syntax and apply a subtle shadow
    border: 0.5px solid var(--cs-border-color);
    box-shadow: 0 0 5px var(--cs-shadow-color);
  }
}
</style>
