<template>
  <el-header class="header" :class="{ 'reverse-layout': !isMacOS }" v-show="showTitlebar">
    <!-- window controls -->
    <div class="window-controls upperLayer">
      <div class="control-icon close" @click="closeWindow" type="text" v-if="showCloseButtons">
        <cs name="close" size="9px" />
      </div>
      <div class="control-icon minimize" @click="minimizeWindow" v-if="showMinimizeButton">
        <cs name="minimize" size="10px" />
      </div>
      <div class="control-icon maximize" @click="toggleMaximize" v-if="showMaximizeButton">
        <cs :name="isFullscreen ? 'fullscreen' : 'fullscreen-off'" size="10px" />
      </div>
    </div>

    <!-- main content wrapper -->
    <div class="titlebar-content-wrapper">
      <!-- left button area -->
      <div class="left">
        <slot name="left"></slot>
      </div>

      <!-- center area -->
      <div class="center">
        <slot name="center"></slot>
      </div>

      <!-- right button area -->
      <div class="right">
        <slot name="right"></slot>

        <!-- menu show control -->
        <el-dropdown @command="handleCommand" trigger="click" v-if="showMenuButton">
          <div class="menu icon-btn upperLayer">
            <cs name="menu" />
          </div>
          <template #dropdown>
            <el-dropdown-menu class="dropdown">
              <template v-for="(menu, idx) in menus" :key="idx">
                <div class="divider" v-if="menu.name === 'divider'" />
                <el-dropdown-item :command="menu.name" v-else>
                  <div class="item">
                    <div class="name">
                      <cs :name="menu.name" />
                      {{ menu.label }}
                    </div>
                  </div>
                </el-dropdown-item>
              </template>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>
  </el-header>
</template>

<script setup>
import { computed, ref, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { useWindowStore } from '@/stores/window'
import { getCurrentWindow } from '@tauri-apps/api/window'

const { t } = useI18n()
const windowStore = useWindowStore()

// Compute OS-specific layout
const isMacOS = computed(() => windowStore.os === 'macos')
const isWindows = computed(() => windowStore.os === 'windows')

const props = defineProps({
  showCloseButtons: {
    type: Boolean,
    default: true,
  },
  showMaximizeButton: {
    type: Boolean,
    default: true,
  },
  showMinimizeButton: {
    type: Boolean,
    default: true,
  },
  showMenuButton: {
    type: Boolean,
    default: false,
  },
})

const availableMenus = [
  'assistant',
  'note',
  'divider',
  'setting',
  'model',
  'skill',
  'divider',
  'about',
  'quit',
]

const menus = computed(() => {
  return availableMenus.map(menu => {
    if (menu === 'divider') {
      return {
        label: '',
        name: 'divider',
      }
    }
    return {
      label: t(`menu.${menu}`),
      name: menu,
    }
  })
})

const showTitlebar = computed(() => {
  return !(isMacOS.value && isFullscreen.value)
})

const isFullscreen = ref(false)
const appWindow = getCurrentWindow()

// Check initial fullscreen state
const checkInitialFullscreen = async () => {
  try {
    const isFullscreenNow = await appWindow.isFullscreen()
    if (isFullscreen.value !== isFullscreenNow) {
      isFullscreen.value = isFullscreenNow
    }
  } catch (error) {
    console.error('Failed to check fullscreen status:', error)
  }
}

// Use debounce to handle state changes
let fullscreenTimeout
const handleFullscreenChange = isFullscreenState => {
  clearTimeout(fullscreenTimeout)
  fullscreenTimeout = setTimeout(() => {
    isFullscreen.value = isFullscreenState
  }, 100)
}

// Modify the implementation to listen for fullscreen state changes
const setupWindowListeners = async () => {
  let unlisten1, unlisten2

  try {
    // Listen for fullscreen state changes
    unlisten1 = await appWindow.listen('tauri://fullscreen', ({ payload }) => {
      handleFullscreenChange(payload)
    })

    // Listen for window resize events
    unlisten2 = await appWindow.listen('tauri://resize', async () => {
      await checkInitialFullscreen()
    })
  } catch (error) {
    console.error('Failed to setup window listeners:', error)
  }

  // Return cleanup function
  return () => {
    unlisten1?.()
    unlisten2?.()
  }
}

// Modify cleanup logic when the component is unmounted
let cleanupListeners
onBeforeUnmount(() => {
  cleanupListeners?.()
})

// Initialize
const init = async () => {
  try {
    cleanupListeners = await setupWindowListeners()
    await checkInitialFullscreen()
  } catch (error) {
    console.error('Failed to initialize:', error)
  }
}

// Minimize the window
const minimizeWindow = async () => {
  try {
    await appWindow.minimize()
  } catch (error) {
    console.error('Failed to minimize window:', error)
  }
}

// Toggle maximize/unmaximize
const toggleMaximize = async () => {
  try {
    const currentFullscreen = await appWindow.isFullscreen()

    if (!currentFullscreen) {
      document.documentElement.style.transition = 'all 0.3s ease-in-out'
      document.documentElement.style.transform = 'scale(0.98)'
      await new Promise(resolve => setTimeout(resolve, 50))

      await appWindow.setFullscreen(true)

      setTimeout(() => {
        document.documentElement.style.transform = 'scale(1)'
        setTimeout(() => {
          document.documentElement.style.transition = ''
          document.documentElement.style.transform = ''
        }, 300)
      }, 50)
    } else {
      await appWindow.setFullscreen(false)
    }

    isFullscreen.value = !currentFullscreen
  } catch (error) {
    console.error('Failed to toggle maximize:', error)
    document.documentElement.style.transition = ''
    document.documentElement.style.transform = ''
  }
}

// Close the window
const closeWindow = async () => {
  try {
    await appWindow.close()
  } catch (error) {
    console.error('Failed to close window:', error)
  }
}

/**
 * Executes actions based on the selected menu command.
 * @param {string} command - The command identifier.
 */
const handleCommand = async command => {
  console.log(command)
  try {
    switch (command) {
      case 'note':
        await invoke('open_note_window')
        break
      case 'assistant':
        await invoke('show_window', { label: 'assistant' })
        break
      case 'setting':
        await invoke('open_setting_window', { settingType: 'general' })
        break
      case 'model':
      case 'skill':
      case 'about':
        await invoke('open_setting_window', { settingType: command })
        break
      case 'quit':
        await invoke('quit_window')
        break
    }
  } catch (error) {
    console.error('Failed to handle command:', error)
  }
}

init()
</script>

<style lang="scss">
.header {
  position: relative;
  display: flex;
  flex-direction: row !important;
  flex-shrink: 0;
  justify-content: space-between;
  align-items: center;
  height: var(--cs-titlebar-height);
  padding: 0 var(--cs-space);
  background-color: var(--cs-titlebar-bg-color);
  border-bottom: 0.5px solid var(--cs-titlebar-border-color);
  box-shadow: 0 0 2px 0 var(--cs-titlebar-border-color);
  border-radius: var(--cs-border-radius-md) var(--cs-border-radius-md) 0 0;
  user-select: none;
  -webkit-user-select: none;
  overflow: hidden;
  box-sizing: border-box;
  -webkit-app-region: drag; // 允许拖动窗口

  .left,
  .right {
    flex: 0;

    display: flex;
    align-items: center;
    justify-content: center;
  }

  .center {
    flex: 1;
    text-align: center;
    overflow: hidden;
  }

  .window-controls {
    // position: absolute;
    // left: 7px;
    // top: 50%;
    // transform: translateY(-50%);
    display: flex;
    gap: 8px;
    z-index: 100;
    -webkit-app-region: no-drag;

    .control-icon {
      width: 14px;
      height: 14px;
      border-radius: 50%;
      display: flex;
      align-items: center;
      justify-content: center;

      .cs {
        opacity: 0;
        transition: opacity 0.2s ease;
      }

      &.close {
        background-color: #ef6051;

        &::before {
          font-size: 12px;
        }
      }

      &.minimize {
        background-color: #f4b730;
      }

      &.maximize {
        background-color: #4bbd38;
      }
    }

    &:hover {
      .control-icon {
        .cs {
          opacity: 1;
        }
      }
    }
  }

  .icon-btn {
    z-index: var(--cs-upper-layer-zindex);

    .cs {
      font-size: 18px !important;
      color: var(--cs-text-color-secondary);
    }
  }

  .titlebar-content-wrapper {
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: 0 var(--cs-space);

    .left,
    .right {
      flex: 0;
    }

    .center {
      flex: 1;
    }
  }

  &.reverse-layout {
    flex-direction: row-reverse !important;

    .window-controls {
      flex-direction: row-reverse !important;
    }

    .titlebar-content-wrapper {
      padding-left: 0;
    }
  }
}
</style>
