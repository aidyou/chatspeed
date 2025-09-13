<template>
  <div class="card">
    <div class="title">{{ $t('settings.general.generalSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">
          {{ $t('settings.general.language') }}
        </div>
        <div class="value">
          <el-select v-model="settings.interfaceLanguage" class="auto-width-select" placement="bottom"
            @change="onInterfaceLanguageChange">
            <el-option v-for="lang in softwareLanguages" :key="lang.code" :label="lang.name" :value="lang.code">
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
          <el-select v-model="settings.primaryLanguage" class="auto-width-select" placement="bottom"
            @change="onPrimaryLanguageChange">
            <el-option v-for="lang in availableLanguages" :key="lang.code" :label="lang.name" :value="lang.code">
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
          <el-select v-model="settings.secondaryLanguage" class="auto-width-select" placement="bottom"
            @change="onSecondaryLanguageChange">
            <el-option v-for="lang in availableLanguages" :key="lang.code" :label="lang.name" :value="lang.code">
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
          <el-select v-model="settings.theme" class="auto-width-select" placement="bottom" @change="onThemeChange">
            <el-option v-for="(label, theme) in themes" :key="theme" :label="label" :value="theme">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.codeLightTheme') }}</div>
        <div class="value">
          <el-select v-model="settings.codeLightTheme" class="auto-width-select" placement="bottom" filterable
            @change="onCodeLightThemeChange">
            <el-option v-for="theme in codeThemes.light" :key="theme" :label="theme" :value="theme">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.codeDarkTheme') }}</div>
        <div class="value">
          <el-select v-model="settings.codeDarkTheme" class="auto-width-select" placement="bottom" filterable
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
        <div class="value" style="width: 200px">
          <el-slider v-model="settings.historyMessages" :min="0" :max="50" @change="onHistoryMessagesChange" />
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.conversationTitleGenModel') }}
            <small class="tooltip">{{
              $t('settings.general.conversationTitleGenModelTooltip')
            }}</small>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-select v-model="settings.conversationTitleGenModel.id" class="auto-width-select" placement="bottom"
            @change="onConversationTitleGenModelIdChange">
            <el-option v-for="model in modelStore.providers" :key="model.id" :label="model.name" :value="model.id">
            </el-option>
          </el-select>
          <el-select v-model="settings.conversationTitleGenModel.model" class="auto-width-select" placement="bottom"
            @change="onConversationTitleGenModelModelChange">
            <el-option v-for="model in conversationTitleGenModelList" :key="model.id" :label="model.name || model.id"
              :value="model.id">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.sendMessageKey') }}
            <small class="tooltip">{{ $t('settings.general.sendMessageKeyTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 200px">
          <el-select v-model="settings.sendMessageKey" @change="onSendMessageKeyChange">
            <el-option v-for="key in ['Enter', 'Shift+Enter']" :key="key" :value="key">
              <span style="display:flex;align-items:center;gap:var(--cs-space-xs)">
                <cs :name="key === 'Enter' ? 'enter-square' : 'shift-enter-square'" size="24px" />
                {{ key }}
              </span>
            </el-option>
          </el-select>
        </div>
      </div>
      <!-- <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.websearchModel') }}
            <small class="tooltip">{{ $t('settings.general.websearchModelTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-select
            v-model="settings.websearchModel.id"
            class="auto-width-select"
            placement="bottom"
            @change="onWebsearchModelIdChange">
            <el-option
              v-for="model in modelStore.providers"
              :key="model.id"
              :label="model.name"
              :value="model.id">
            </el-option>
          </el-select>
          <el-select
            v-model="settings.websearchModel.model"
            class="auto-width-select"
            placement="bottom"
            @change="onWebsearchModelModelChange">
            <el-option
              v-for="model in websearchModelList"
              :key="model.id"
              :label="model.name || model.id"
              :value="model.id">
            </el-option>
          </el-select>
        </div>
      </div> -->
    </div>
  </div>

  <div class="card">
    <div class="title">{{ $t('settings.general.searchEngine') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.searchEngine') }}
            <small class="tooltip">{{ $t('settings.general.searchEngineTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 45%">
          <el-select v-model="settings.searchEngine" @change="onSearchEngineChange">
            <el-option v-for="engine in searchEngines" :key="engine" :label="engine" :value="engine" />
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.scraperDebugMode') }}
            <small class="tooltip">{{ $t('settings.general.scraperDebugModeTooltip') }}</small>
          </div>
        </div>
        <div class="value">
          <el-switch v-model="settings.scraperDebugMode" @change="onScraperDebugModeChange" />
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.scraperConcurrencyCount') }}
            <small class="tooltip">{{
              $t('settings.general.scraperConcurrencyCountTooltip')
            }}</small>
          </div>
        </div>
        <div class="value" style="width: 45%">
          <el-input v-model="settings.scraperConcurrencyCount" @input="scraperConcurrencyCountChange" type="number" />
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.search.google') }}
            <el-space>
              <small class="tooltip">{{ $t('settings.general.search.clickHere') }}</small>
              <a class="small info" href="javascript:"
                @click="openUrl('https://programmablesearchengine.google.com/controlpanel/all')">{{
                  $t('settings.general.search.apply') }}</a>
            </el-space>
          </div>
        </div>
        <div class="value" style="width: 400px">
          <el-input type="password" v-model="settings.googleApiKey" @input="onGoogleApiKeyChange"
            :placeholder="$t('settings.general.search.googleApiKey')" />
          <el-input v-model="settings.googleSearchId" @input="onGoogleSearchIdChange"
            :placeholder="$t('settings.general.search.googleSearchId')" />
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.search.serper') }}
            <el-space>
              <small class="tooltip">{{ $t('settings.general.search.clickHere') }}</small>
              <a class="small info" href="javascript:" @click="openUrl('https://serper.dev/api-keys')">{{
                $t('settings.general.search.apply') }}</a>
            </el-space>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-input type="password" v-model="settings.serperApiKey" @input="onSerperApiKeyChange"
            :placeholder="$t('settings.general.search.serperApiKey')" />
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.search.tavily') }}
            <el-space>
              <small class="tooltip">{{ $t('settings.general.search.clickHere') }}</small>
              <a class="small info" href="javascript:" @click="openUrl('https://app.tavily.com/home')">{{
                $t('settings.general.search.apply') }}</a>
            </el-space>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-input type="password" v-model="settings.tavilyApiKey" @input="onTavilyApiKeyChange"
            :placeholder="$t('settings.general.search.tavilyApiKey')" />
        </div>
      </div>
    </div>
  </div>

  <!-- workflow settings -->
  <!-- <div class="card">
    <div class="title">{{ $t('settings.general.workflowSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.workflow.reasoning') }}
            <small class="tooltip">{{ $t('settings.general.workflow.reasoningTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-select
            v-model="settings.workflowReasoningModel.id"
            class="auto-width-select"
            placement="bottom"
            @change="onWorkflowReasoningModelIdChange">
            <el-option
              v-for="model in modelStore.providers"
              :key="model.id"
              :label="model.name"
              :value="model.id">
            </el-option>
          </el-select>
          <el-select
            v-model="settings.workflowReasoningModel.model"
            class="auto-width-select"
            placement="bottom"
            @change="onWorkflowReasoningModelModelChange">
            <el-option
              v-for="model in workflowReasoningModelList"
              :key="model.id"
              :label="model.name || model.id"
              :value="model.id">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">
          <div class="label-text">
            {{ $t('settings.general.workflow.general') }}
            <small class="tooltip">{{ $t('settings.general.workflow.generalTooltip') }}</small>
          </div>
        </div>
        <div class="value" style="width: 300px">
          <el-select
            v-model="settings.workflowGeneralModel.id"
            class="auto-width-select"
            placement="bottom"
            @change="onWorkflowGeneralModelIdChange">
            <el-option
              v-for="model in modelStore.providers"
              :key="model.id"
              :label="model.name"
              :value="model.id">
            </el-option>
          </el-select>
          <el-select
            v-model="settings.workflowGeneralModel.model"
            class="auto-width-select"
            placement="bottom"
            @change="onWorkflowGeneralModelModelChange">
            <el-option
              v-for="model in workflowGeneralModelList"
              :key="model.id"
              :label="model.name || model.id"
              :value="model.id">
            </el-option>
          </el-select>
        </div>
      </div>
    </div>
  </div> -->

  <!-- network settings -->
  <div class="card">
    <div class="title">{{ $t('settings.general.networkSettings') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyType') }}</div>
        <div class="value">
          <el-select v-model="settings.proxyType" class="auto-width-select" placement="bottom"
            @change="onProxyTypeChange">
            <el-option v-for="(label, type) in proxyTypes" :key="type" :label="label" :value="type">
            </el-option>
          </el-select>
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyServer') }}</div>
        <div class="value">
          <el-input v-model="settings.proxyServer" @change="onProxyServerChange"
            :placeholder="$t('settings.general.proxyServerPlaceholder')" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyUsername') }}</div>
        <div class="value">
          <el-input v-model="settings.proxyUsername" @change="onProxyUsernameChange"
            :placeholder="$t('settings.general.proxyUsernamePlaceholder')" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.general.proxyPassword') }}</div>
        <div class="value">
          <el-input v-model="settings.proxyPassword" @change="onProxyPasswordChange"
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
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top" :hide-after="0"
            :enterable="false">
            <el-input v-model="settings.mainWindowVisibleShortcut" readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'mainWindowVisibleShortcut')" @focus="isCapturing = true"
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
        <div class="label">{{ $t('settings.general.noteWindowVisibleShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top" :hide-after="0"
            :enterable="false">
            <el-input v-model="settings.noteWindowVisibleShortcut" readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'noteWindowVisibleShortcut')" @focus="isCapturing = true"
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
      <div class="item">
        <div class="label">{{ $t('settings.general.assistantWindowVisibleShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top" :hide-after="0"
            :enterable="false">
            <el-input v-model="settings.assistantWindowVisibleShortcut" readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'assistantWindowVisibleShortcut')" @focus="isCapturing = true"
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
        <div class="label">{{ $t('settings.general.assistantWindowVisibleAndPasteShortcut') }}</div>
        <div class="value">
          <el-tooltip :content="$t('settings.general.pressKeysToSet')" placement="top" :hide-after="0"
            :enterable="false">
            <el-input v-model="settings.assistantWindowVisibleAndPasteShortcut" readonly
              :placeholder="$t('settings.general.pressKeysToSet')"
              @keydown.prevent="e => captureShortcut(e, 'assistantWindowVisibleAndPasteShortcut')"
              @focus="isCapturing = true" @blur="isCapturing = false">
              <template #append>
                <el-button @click="clearShortcut('assistantWindowVisibleAndPasteShortcut')">
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
          <el-input v-model="settings.backupDir" :readonly="true" :clearable="true" :placeholder="defaultBackupDir"
            @change="onBackupDirChange" @click="selectBackupDir" />
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
          <el-select v-model="restoreDir" class="auto-width-select" placement="top" filterable @change="onRestore">
            <el-option v-for="backup in backups" :key="backup.value" :label="backup.label" :value="backup.value">
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

import { appDataDir } from '@tauri-apps/api/path'
import { enable, disable } from '@tauri-apps/plugin-autostart'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

import {
  getAvailableLanguages,
  getSoftwareLanguages,
  mapBrowserLangToStandard
} from '@/i18n/langUtils'
import { showMessage, openUrl } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'

const { t } = useI18n()
import { useSettingStore } from '@/stores/setting'
import { useModelStore } from '@/stores/model'
const modelStore = useModelStore()
const settingStore = useSettingStore()
// import { useSkillStore } from '@/stores/skill'
// const skillStore = useSkillStore()

const { settings } = storeToRefs(settingStore)

const backups = ref([])
const restoreDir = ref('')
const searchEngines = computed(() => {
  const engines = ['bing', 'duckduckgo', 'brave', 'so', 'sogou']

  if (settings.value.googleApiKey && settings.value.googleSearchId) {
    engines.push('google')
  }
  if (settings.value.serperApiKey) {
    engines.push('serper')
  }
  if (settings.value.tavilyApiKey) {
    engines.push('tavily')
  }

  return engines
})

const defaultBackupDir = ref('')

import codeThemes from '@/config/highlight.js/themes.json'
const themes = computed(() => ({
  system: t('settings.general.systemTheme'),
  light: t('settings.general.lightTheme'),
  dark: t('settings.general.darkTheme')
}))

const conversationTitleGenModelList = computed(() => {
  if (settingStore.settings.conversationTitleGenModel.id) {
    return (
      modelStore.getModelProviderById(settingStore.settings.conversationTitleGenModel.id)?.models ||
      []
    )
  }
  return []
})

const websearchModelList = computed(() => {
  if (settingStore.settings.websearchModel.id) {
    return modelStore.getModelProviderById(settingStore.settings.websearchModel.id)?.models || []
  }
  return []
})

const proxyTypes = computed(() => ({
  none: t('settings.general.proxyTypes.none'),
  system: t('settings.general.proxyTypes.system'),
  http: t('settings.general.proxyTypes.http')
}))

// get all available languages
const availableLanguages = getAvailableLanguages()
const softwareLanguages = getSoftwareLanguages()

onMounted(async () => {
  defaultBackupDir.value = `${await appDataDir()}/backups`
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

/**
 * Handles the change of show menu button
 * @param {boolean} value - The value of show menu button
 */
const onShowMenuButtonChange = value => {
  setSetting('showMenuButton', value || false)
}

// =================================================
// shortcut settings
// =================================================

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
  '≈': 'x' // Fixes the display issue of Alt+x on macOS
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

// =================================================
// chat settings
// =================================================

/**
 * Handles the change of history messages
 * @param {number} value - The value of history messages
 */
const onHistoryMessagesChange = value => {
  setSetting('historyMessages', Number(value || 0))
}

/**
 * Handles the change of conversation title generation model id
 * @param {number} value - The value of conversation title generation model id
 */
const onConversationTitleGenModelIdChange = value => {
  settingStore.settings.conversationTitleGenModel = { id: value || 0, model: '' }
  setSetting('conversationTitleGenModel', settingStore.settings.conversationTitleGenModel)
}

/**
 * Handles the change of conversation title generation model
 * @param {string} value - The value of conversation title generation model
 */
const onConversationTitleGenModelModelChange = value => {
  settingStore.settings.conversationTitleGenModel.model = value || ''
  setSetting('conversationTitleGenModel', settingStore.settings.conversationTitleGenModel)
}

/**
 * Handles the change of send message key
 * @param {string} value - The value of send message key
 */
const onSendMessageKeyChange = value => {
  setSetting('sendMessageKey', value || 'Enter')
}

/**
 * Handles the change of web search model id
 * @param {number} value - The value of web search model id
 */
const onWebsearchModelIdChange = value => {
  settingStore.settings.websearchModel = { id: value || 0, model: '' }
  setSetting('websearchModel', settingStore.settings.websearchModel)
}

/**
 * Handles the change of web search model
 * @param {string} value - The value of web search model
 */
const onWebsearchModelModelChange = value => {
  settingStore.settings.websearchModel.model = value || ''
  setSetting('websearchModel', settingStore.settings.websearchModel)
}

const onSearchEngineChange = value => {
  setSetting('searchEngine', value || '')
}

const onScraperDebugModeChange = value => {
  setSetting('scraperDebugMode', value || false)
}

const scraperConcurrencyCountChange = value => {
  setSetting('scraperConcurrencyCount', Number(value || 0))
}

/**
 * Handles the change of chatspeed crawler
 * @param {string} value - The value of chatspeed crawler
 */
const onGoogleApiKeyChange = value => {
  setSetting('googleApiKey', value ? value.trim() : '' || '')
}

const onGoogleSearchIdChange = value => {
  setSetting('googleSearchId', value ? value.trim() : '' || '')
}

const onSerperApiKeyChange = value => {
  setSetting('serperApiKey', value ? value.trim() : '' || '')
}

const onTavilyApiKeyChange = value => {
  setSetting('tavilyApiKey', value ? value.trim() : '' || '')
}

// =================================================
// workflow settings
// =================================================

const onWorkflowReasoningModelIdChange = value => {
  settingStore.settings.workflowReasoningModel = { id: value || 0, model: '' }
  setSetting('workflowReasoningModel', settingStore.settings.workflowReasoningModel)
}

const workflowReasoningModelList = computed(() => {
  if (settingStore.settings.workflowReasoningModel.id) {
    return (
      modelStore.getModelProviderById(settingStore.settings.workflowReasoningModel.id)?.models || []
    )
  }
  return []
})

const onWorkflowReasoningModelModelChange = value => {
  settingStore.settings.workflowReasoningModel.model = value || ''
  setSetting('workflowReasoningModel', settingStore.settings.workflowReasoningModel)
}

const onWorkflowGeneralModelIdChange = value => {
  settingStore.settings.workflowGeneralModel = { id: value || 0, model: '' }
  setSetting('workflowGeneralModel', settingStore.settings.workflowGeneralModel)
}

const workflowGeneralModelList = computed(() => {
  if (settingStore.settings.workflowGeneralModel.id) {
    return (
      modelStore.getModelProviderById(settingStore.settings.workflowGeneralModel.id)?.models || []
    )
  }
  return []
})

const onWorkflowGeneralModelModelChange = value => {
  settingStore.settings.workflowGeneralModel.model = value || ''
  setSetting('workflowGeneralModel', settingStore.settings.workflowGeneralModel)
}

// =================================================
// network settings
// =================================================

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
  const server = settings.value.proxyServer || ''
  if (server && !server.startsWith('http://') && !server.startsWith('https://')) {
    showMessage(t('settings.general.proxyServerInvalid'), 'error')
    return
  }
  setSetting('proxyServer', server)
}

const onProxyUsernameChange = () => {
  setSetting('proxyUsername', settings.value.proxyUsername || '')
}

const onProxyPasswordChange = () => {
  setSetting('proxyPassword', settings.value.proxyPassword || '')
}

// =================================================
// Advanced settings
// =================================================

/**
 * Handles the change of auto update
 * @param {boolean} value - The value of auto update
 */
const onAutoUpdateChange = value => {
  setSetting('autoUpdate', value || false)
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

// =================================================
// Backup settings
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
      defaultPath: await appDataDir()
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
    background: 'var(--cs-bg-color-opacity)'
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
      cancelButtonText: t('common.cancel')
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
          value: b
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
