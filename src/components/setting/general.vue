<template>
  <div class="card">
    <div class="title">{{ $t('settings.general.generalSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">
          {{ $t('settings.general.language') }}
        </div>
        <div class="value">
          <el-select
            v-model="settings.interfaceLanguage"
            class="auto-width-select"
            placement="bottom"
            @change="onInterfaceLanguageChange">
            <el-option
              v-for="lang in softwareLanguages"
              :key="lang.code"
              :label="lang.name"
              :value="lang.code">
              <span>{{ lang.icon }}</span>
              <span>{{ lang.name }}</span>
            </el-option>
          </el-select>
        </div>
      </div>

      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.primaryLanguage') }}
            <small class="tooltip">{{ $t('settings.general.primaryLanguageTooltip') }}</small>
          </div>
        </div>
        <div class="value">
          <el-select
            v-model="settings.primaryLanguage"
            class="auto-width-select"
            placement="bottom"
            @change="onPrimaryLanguageChange">
            <el-option
              v-for="lang in availableLanguages"
              :key="lang.code"
              :label="lang.name"
              :value="lang.code">
              <span>{{ lang.icon }}</span>
              <span>{{ lang.name }}</span>
            </el-option>
          </el-select>
        </div>
      </div>

      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.secondaryLanguage') }}
            <small class="tooltip">{{ $t('settings.general.secondaryLanguageTooltip') }}</small>
          </div>
        </div>
        <div class="value">
          <el-select
            v-model="settings.secondaryLanguage"
            class="auto-width-select"
            placement="bottom"
            @change="onSecondaryLanguageChange">
            <el-option
              v-for="lang in availableLanguages"
              :key="lang.code"
              :label="lang.name"
              :value="lang.code">
              <span>{{ lang.icon }}</span>
              <span>{{ lang.name }}</span>
            </el-option>
          </el-select>
        </div>
      </div>
    </div>
  </div>

  <!-- theme settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.interfaceSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.general.theme') }}</div>
        <div class="value">
          <el-select
            v-model="settings.theme"
            class="auto-width-select"
            placement="bottom"
            @change="onThemeChange">
            <el-option v-for="(label, theme) in themes" :key="theme" :label="label" :value="theme">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.codeLightTheme') }}</div>
        <div class="value">
          <el-select
            v-model="settings.codeLightTheme"
            class="auto-width-select"
            placement="bottom"
            filterable
            @change="onCodeLightThemeChange">
            <el-option v-for="theme in codeThemes.light" :key="theme" :label="theme" :value="theme">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.codeDarkTheme') }}</div>
        <div class="value">
          <el-select
            v-model="settings.codeDarkTheme"
            class="auto-width-select"
            placement="bottom"
            filterable
            @change="onCodeDarkThemeChange">
            <el-option v-for="theme in codeThemes.dark" :key="theme" :label="theme" :value="theme">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.showMenuButton') }}
            <small class="tooltip">{{ $t('settings.general.showMenuButtonTooltip') }}</small>
          </div>
        </div>
        <div class="value">
          <el-switch v-model="settings.showMenuButton" @change="onShowMenuButtonChange" />
        </div>
      </div>
    </div>
  </div>

  <!-- conversation settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.conversationSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.historyMessages') }}
            <small class="tooltip">{{ $t('settings.general.historyMessagesTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 120px">
          <el-slider
            v-model="settings.historyMessages"
            :min="0"
            :max="10"
            @change="onHistoryMessagesChange" />
        </div>
      </div>
    </div>
  </div>

  <!-- network settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.networkSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyType') }}</div>
        <div class="value">
          <el-select
            v-model="settings.proxyType"
            class="auto-width-select"
            placement="bottom"
            @change="onProxyTypeChange">
            <el-option v-for="(label, type) in proxyTypes" :key="type" :label="label" :value="type">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyServer') }}</div>
        <div class="value">
          <el-input
            v-model="settings.proxyServer"
            @change="onProxyServerChange"
            :placeholder="$t('settings.general.proxyServerPlaceholder')" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyUsername') }}</div>
        <div class="value">
          <el-input
            v-model="settings.proxyUsername"
            @change="onProxyUsernameChange"
            :placeholder="$t('settings.general.proxyUsernamePlaceholder')" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyPassword') }}</div>
        <div class="value">
          <el-input
            v-model="settings.proxyPassword"
            @change="onProxyPasswordChange"
            :placeholder="$t('settings.general.proxyPasswordPlaceholder')" />
        </div>
      </div>
    </div>
  </div>

  <!-- shortcut settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.shortcutSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.general.mainWindowVisibleShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top">
            <el-input
              v-model="settings.mainWindowVisibleShortcut"
              readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'mainWindowVisibleShortcut')"
              @focus="isCapturing = true"
              @blur="isCapturing = false">
              <template #append>
                <el-button @click="clearShortcut('mainWindowVisibleShortcut')">
                  {{ $t('common.clear') }}
                </el-button>
              </template>
            </el-input>
          </el-tooltip>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.assistantWindowVisibleShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top">
            <el-input
              v-model="settings.assistantWindowVisibleShortcut"
              readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'assistantWindowVisibleShortcut')"
              @focus="isCapturing = true"
              @blur="isCapturing = false">
              <template #append>
                <el-button @click="clearShortcut('assistantWindowVisibleShortcut')">
                  {{ $t('common.clear') }}
                </el-button>
              </template>
            </el-input>
          </el-tooltip>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.noteWindowVisibleShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top">
            <el-input
              v-model="settings.noteWindowVisibleShortcut"
              readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'noteWindowVisibleShortcut')"
              @focus="isCapturing = true"
              @blur="isCapturing = false">
              <template #append>
                <el-button @click="clearShortcut('noteWindowVisibleShortcut')">
                  {{ $t('common.clear') }}
                </el-button>
              </template>
            </el-input>
          </el-tooltip>
        </div>
      </div>
    </div>
  </div>

  <!-- advanced settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.advancedSettings') }}</div>
    <div class="list">
      <!--<div class="item">
        <div class="label">{{ $t('settings.general.wordSelectionToolbar') }}</div>
        <div class="value">
          <el-switch
            v-model="settings.wordSelectionToolbar"
            @change="onWordSelectionToolbarChange" />
        </div>
      </div>-->
      <div class="item">
        <div class="label">{{ $t('settings.general.autoStart') }}</div>
        <div class="value">
          <el-switch v-model="settings.autoStart" @change="onAutoStartChange" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.autoUpdate') }}</div>
        <div class="value">
          <el-switch v-model="settings.autoUpdate" @change="onAutoUpdateChange" />
        </div>
      </div>
    </div>
  </div>

  <!-- backup settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.backupSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.general.backupDir') }}</div>
        <div class="value" style="width: 70%">
          <el-input
            v-model="settings.backupDir"
            :clearable="true"
            :placeholder="$t('settings.general.backupDirPlaceholder')"
            @change="onBackupDirChange"
            @click="selectBackupDir" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.backup') }}</div>
        <div class="value">
          <el-button @click="startBackup">
            {{ $t('settings.general.runBackup') }}
          </el-button>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.restore') }}</div>
        <div class="value">
          <el-select
            v-model="restoreDir"
            class="auto-width-select"
            placement="top"
            filterable
            @change="onRestore">
            <el-option
              v-for="backup in backups"
              :key="backup.value"
              :label="backup.label"
              :value="backup.value">
            </el-option>
          </el-select>
        </div>
      </div>
    </div>
  </div>
</template>
<script setup>
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'

import { documentDir } from '@tauri-apps/api/path'
import { enable, disable } from '@tauri-apps/plugin-autostart'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

import {
  getAvailableLanguages,
  getSoftwareLanguages,
  mapBrowserLangToStandard,
} from '@/i18n/langUtils'
import { showMessage } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

const { t } = useI18n()
import { useSettingStore } from '@/stores/setting'
import { useModelStore } from '@/stores/model'
import { useSkillStore } from '@/stores/skill'
const settingStore = useSettingStore()
const modelStore = useModelStore()
const skillStore = useSkillStore()

const { settings } = storeToRefs(settingStore)

const backups = ref([])
const restoreDir = ref('')

import codeThemes from '@/config/highlight.js/themes.json'
const themes = computed(() => ({
  system: t('settings.general.systemTheme'),
  light: t('settings.general.lightTheme'),
  dark: t('settings.general.darkTheme'),
}))

const proxyTypes = computed(() => ({
  none: t('settings.general.proxyTypes.none'),
  system: t('settings.general.proxyTypes.system'),
  http: t('settings.general.proxyTypes.http'),
}))

// get all available languages
const availableLanguages = getAvailableLanguages()
const softwareLanguages = getSoftwareLanguages()

onMounted(() => {
  getAllBackups()
})

/**
 * Sets the value of a setting
 */
const setSetting = (key, value) => {
  settingStore.setSetting(key, value).catch(err => {
    showMessage(t('settings.general.updateSettingFailed', { error: err }), 'error')
  })
}

const onInterfaceLanguageChange = value => {
  setSetting('interfaceLanguage', value || mapBrowserLangToStandard(navigator.language) || '')
}

const onPrimaryLanguageChange = value => {
  setSetting('primaryLanguage', value || mapBrowserLangToStandard(navigator.language) || '')
}

const onSecondaryLanguageChange = value => {
  setSetting('secondaryLanguage', value || 'en')
}

const onThemeChange = value => {
  setSetting('theme', value || 'system')
}

const onCodeLightThemeChange = value => {
  setSetting('codeLightTheme', value || 'default')
}

const onCodeDarkThemeChange = value => {
  setSetting('codeDarkTheme', value || 'default')
}

const isCapturing = ref(false)
const currentShortcutKey = ref(null)

/**
 * Maps special keys to their display names
 */
const KEY_DISPLAY_MAP = {
  '`': 'Backquote',
  '~': 'Backquote',
  '!': '1',
  '@': '2',
  '#': '3',
  $: '4',
  '%': '5',
  '^': '6',
  '&': '7',
  '*': '8',
  '(': '9',
  ')': '0',
  '-': 'Minus',
  _: 'Minus',
  '=': 'Equal',
  '+': 'Equal',
  '[': 'BracketLeft',
  '{': 'BracketLeft',
  ']': 'BracketRight',
  '}': 'BracketRight',
  '\\': 'Backslash',
  '|': 'Backslash',
  ';': 'Semicolon',
  ':': 'Semicolon',
  "'": 'Quote',
  '"': 'Quote',
  ',': 'Comma',
  '<': 'Comma',
  '.': 'Period',
  '>': 'Period',
  '/': 'Slash',
  '?': 'Slash',
  '≈': 'x', // Fixes the display issue of Alt+x on macOS
}

/**
 * Captures keyboard shortcuts and validates them
 * Only allows combinations with modifiers (Ctrl/Cmd, Alt, Shift) for letter/number keys
 * Function keys (F1-F12) can be used without modifiers
 */
const captureShortcut = (event, shortcutKey) => {
  event.preventDefault()
  event.stopPropagation()

  currentShortcutKey.value = shortcutKey

  const { key, code, ctrlKey, metaKey, altKey, shiftKey } = event

  if (['Control', 'Alt', 'Shift', 'Meta'].includes(key)) {
    return
  }

  // Check if it's a function key (F1-F12)
  const isFunctionKey = /^F(1[0-2]|[1-9])$/i.test(key)

  // Check if any modifier keys are pressed
  const hasModifier = ctrlKey || metaKey || altKey || shiftKey

  // Get the string of the key combination
  let shortcut = []
  if (ctrlKey || metaKey) shortcut.push('CommandOrControl')
  if (altKey) shortcut.push('Alt')
  if (shiftKey) shortcut.push('Shift')

  // Process the main key
  let mainKey
  if (KEY_DISPLAY_MAP[key]) {
    mainKey = KEY_DISPLAY_MAP[key]
  } else if (code.startsWith('Key')) {
    // For letter keys, use uppercase form
    mainKey = code.slice(3).toUpperCase()
  } else if (code.startsWith('Digit')) {
    // For number keys, use the number itself
    mainKey = code.slice(5)
  } else {
    // For other keys, use the original key name
    mainKey = key
  }

  // Validate the shortcut combination
  if (!isFunctionKey && !hasModifier) {
    showMessage(t('settings.general.shortcutNeedsModifier'), 'warning')
    return
  }

  const shortcutString = shortcut.join('+') + (shortcut.length > 0 ? '+' : '') + mainKey
  setSetting(shortcutKey, shortcutString)
}

/**
 * Clears the current shortcut
 */
const clearShortcut = shortcutKey => {
  setSetting(shortcutKey, '')
}
/**
 * Handles the change of history messages
 * @param {number} value - The value of history messages
 */
const onHistoryMessagesChange = value => {
  setSetting('historyMessages', value || 0)
}
/**
 * Handles the change of show menu button
 * @param {boolean} value - The value of show menu button
 */
const onShowMenuButtonChange = value => {
  setSetting('showMenuButton', value || false)
}

/**
 * Handles the change of proxy type
 * @param {string} value - The value of proxy type
 */
const onProxyTypeChange = value => {
  setSetting('proxyType', value || 'none')
}

/**
 * Handles the change of proxy server
 */
const onProxyServerChange = () => {
  setSetting('proxyServer', settings.value.proxyServer || '')
}

const onProxyUsernameChange = () => {
  setSetting('proxyUsername', settings.value.proxyUsername || '')
}

const onProxyPasswordChange = () => {
  setSetting('proxyPassword', settings.value.proxyPassword || '')
}

const onWordSelectionToolbarChange = value => {
  settingStore.setTextMonitor(value || false).catch(err => {
    showMessage(err, 'error')
  })
}

/**
 * Handles the change of auto start
 * @param {boolean} value - The value of auto start
 */
const onAutoStartChange = async value => {
  try {
    if (value) {
      await enable()
    } else {
      await disable()
    }
    // Only update the setting if the autostart operation succeeded
    setSetting('autoStart', value)
  } catch (error) {
    console.error('Failed to change autostart setting:', error)
    // Revert the switch state
    settings.value.autoStart = !value
    showMessage(t('settings.general.autoStartChangeFailed', { error: error.toString() }), 'error')
  }
}

/**
 * Handles the change of auto update
 * @param {boolean} value - The value of auto update
 */
const onAutoUpdateChange = value => {
  setSetting('autoUpdate', value || false)
}

// =================================================
// Backup
// =================================================
const onBackupDirChange = () => {
  setSetting('backupDir', settings.value.backupDir || '')
  getAllBackups()
}

const selectBackupDir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      filters: [{ name: 'Directory', extensions: ['*'] }],
      defaultPath: await documentDir(),
    })

    if (selected) {
      settings.value.backupDir = selected
    } else {
      settings.value.backupDir = ''
    }
  } catch (error) {
    settings.value.backupDir = ''
    showMessage(error.toString(), 'error')
  } finally {
    setSetting('backupDir', settings.value.backupDir)
    getAllBackups()
  }
}

const isBackingUp = ref(false)

const startBackup = async () => {
  if (isBackingUp.value) return

  // Show loading state
  const loadingInstance = ElLoading.service({
    text: t('settings.general.backingUp'),
    background: 'var(--cs-bg-color-opacity)',
  })
  try {
    isBackingUp.value = true

    await invoke('backup_setting', { backupDir: settings.value.backupDir })
    getAllBackups()
    showMessage(t('settings.general.backupSuccess'), 'success')
  } catch (error) {
    showMessage(error.toString(), 'error')
  } finally {
    isBackingUp.value = false
    // Hide loading state
    loadingInstance?.close()
  }
}

const onRestore = value => {
  ElMessageBox.confirm(
    t('settings.general.restoreConfirm'),
    t('settings.general.restoreConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
    }
  )
    .then(() => {
      invoke('restore_setting', { backupDir: value })
        .then(() => {
          settingStore.reloadConfig().then(() => {
            sendSyncState('model', 'all')
            sendSyncState('skill', 'all')
            sendSyncState('setting_changed', 'all')
            showMessage(t('settings.general.restoreSuccess'), 'success')
          })
        })
        .catch(error => {
          showMessage(error.toString(), 'error')
        })
    })
    .catch(() => {
      restoreDir.value = ''
    })
}

const getAllBackups = () => {
  invoke('get_all_backups', { backupDir: settings.value.backupDir }).then(dirs => {
    if (!dirs || !Array.isArray(dirs)) {
      backups.value = []
    } else {
      backups.value = []
      dirs.forEach(b => {
        backups.value.push({
          label: b.split('/').pop(),
          value: b,
        })
      })
    }
  })
}
</script>

<style lang="scss">
.auto-width-select {
  width: auto;
  min-width: 150px;
  max-width: 400px;

  .el-input {
    width: 100%;
  }

  .el-input__wrapper {
    width: 100%;
  }

  .el-input__inner {
    width: 100%;
  }
}

.el-loading-mask.is-fullscreen {
  border-radius: var(--cs-border-radius-md) !important;
}
</style>
