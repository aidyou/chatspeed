<template>
  <section class="sandbox-schemes">
    <div class="card">
      <div class="title">
        <span>{{ $t('settings.sandbox.title') }}</span>
        <el-tooltip :content="$t('settings.sandbox.add')" placement="left" :enterable="false" :hide-after="0">
          <span class="icon" @click="openCreate"><cs name="add" /></span>
        </el-tooltip>
      </div>
      <div v-if="schemeStore.schemes.length" v-loading="schemeStore.loading" class="list">
        <div v-for="scheme in schemeStore.schemes" :key="scheme.id" class="item">
          <div class="label">
            <div class="scheme-marker"><cs name="setting" size="16px" color="secondary" /></div>
            <div class="label-text">
              {{ scheme.name }}
              <small>{{ schemeSummary(scheme) }}</small>
            </div>
          </div>
          <div class="value">
            <el-switch
              :model-value="!scheme.disabled"
              :loading="togglingSchemeId === scheme.id"
              :aria-label="$t('settings.sandbox.enabled')"
              @change="enabled => toggleSchemeEnabled(scheme, enabled)" />
            <el-tooltip :content="$t('settings.sandbox.edit')" placement="top" :enterable="false" :hide-after="0">
              <span class="icon" @click="openEdit(scheme)"><cs name="edit" size="16px" color="secondary" /></span>
            </el-tooltip>
            <el-tooltip :content="$t('common.delete')" placement="top" :enterable="false" :hide-after="0">
              <span class="icon" @click="remove(scheme)"><cs name="trash" size="16px" color="secondary" /></span>
            </el-tooltip>
          </div>
        </div>
      </div>
      <div v-else-if="!schemeStore.loading" class="list">
        <div class="empty-state">
          {{ $t('settings.sandbox.empty') }}
          <el-button type="primary" size="small" @click="openCreate"><cs name="add" />{{ $t('settings.sandbox.add') }}</el-button>
        </div>
      </div>
    </div>

    <el-dialog
      v-model="dialogVisible"
      :title="editing ? $t('settings.sandbox.edit') : $t('settings.sandbox.add')"
      width="650px"
      :close-on-click-modal="false"
      :close-on-press-escape="false">
      <el-form label-width="140px">
        <el-form-item :label="$t('settings.sandbox.name')" required>
          <el-input v-model="draft.name" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.schemeDescription')">
          <el-input v-model="draft.description" type="textarea" :rows="2" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.disabled')">
          <el-switch v-model="draft.disabled" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.runtimePreference')">
          <el-select v-model="draft.config.runtimePreference">
            <el-option value="auto" :label="$t('settings.sandbox.runtimeAuto')" />
            <el-option value="msb" label="MSB" />
            <el-option value="docker" label="Docker" />
          </el-select>
        </el-form-item>

        <section class="editor-section editor-section--rules">
          <div class="rule-tabs__header">
            <el-tabs v-model="activeRuleTab" class="rule-tabs">
              <el-tab-pane :label="$t('settings.sandbox.profiles')" name="profiles" />
              <el-tab-pane :label="$t('settings.sandbox.hostRules')" name="hostRules" />
            </el-tabs>
            <el-button v-if="activeRuleTab === 'profiles'" size="small" @click="openProfileEditor()">
              {{ $t('settings.sandbox.addProfile') }}
            </el-button>
            <el-button v-else size="small" @click="openHostRuleEditor()">
              {{ $t('settings.sandbox.addHostRule') }}
            </el-button>
          </div>

          <div v-if="activeRuleTab === 'profiles'">
            <el-table
              v-if="draft.config.profiles.length"
              :data="draft.config.profiles"
              size="small"
              max-height="320"
              class="rule-table">
              <el-table-column prop="name" :label="$t('settings.sandbox.profileName')" min-width="150" />
              <el-table-column prop="priority" :label="$t('settings.sandbox.priority')" width="100" />
              <el-table-column prop="image" :label="$t('settings.sandbox.image')" width="180" show-overflow-tooltip />
              <el-table-column
                v-if="hasProfileWorkspaceAccess"
                :label="$t('settings.sandbox.workspaceAccess')"
                width="120">
                <template #default="{ row }">
                  {{ workspaceAccessLabel(row.workspaceAccess) }}
                </template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.sandbox.management')"
                width="112"
                align="right"
                fixed="right">
                <template #default="{ row }">
                  <div class="rule-table__actions">
                    <el-tooltip :content="$t('settings.sandbox.editProfile')" placement="top" :enterable="false" :hide-after="0">
                      <span class="icon" @click="openProfileEditor(row)"><cs name="edit" size="16px" color="secondary" /></span>
                    </el-tooltip>
                    <el-tooltip :content="$t('common.delete')" placement="top" :enterable="false" :hide-after="0">
                      <span class="icon" @click="removeProfile(row)"><cs name="trash" size="16px" color="secondary" /></span>
                    </el-tooltip>
                  </div>
                </template>
              </el-table-column>
            </el-table>
            <div v-else class="profile-list__empty">{{ $t('settings.sandbox.emptyProfiles') }}</div>
          </div>

          <div v-else>
            <el-table
              v-if="draft.config.hostRules.length"
              :data="draft.config.hostRules"
              size="small"
              max-height="320"
              class="rule-table">
              <el-table-column prop="name" :label="$t('settings.sandbox.ruleName')" min-width="160" />
              <el-table-column prop="priority" :label="$t('settings.sandbox.priority')" width="100" />
              <el-table-column :label="$t('settings.sandbox.commandPatterns')" min-width="180" show-overflow-tooltip>
                <template #default="{ row }">
                  {{ (row.commandPatterns || []).join(', ') }}
                </template>
              </el-table-column>
              <el-table-column
                :label="$t('settings.sandbox.management')"
                width="112"
                align="right"
                fixed="right">
                <template #default="{ row }">
                  <div class="rule-table__actions">
                    <el-tooltip :content="$t('settings.sandbox.editHostRule')" placement="top" :enterable="false" :hide-after="0">
                      <span class="icon" @click="openHostRuleEditor(row)"><cs name="edit" size="16px" color="secondary" /></span>
                    </el-tooltip>
                    <el-tooltip :content="$t('common.delete')" placement="top" :enterable="false" :hide-after="0">
                      <span class="icon" @click="removeHostRule(row)"><cs name="trash" size="16px" color="secondary" /></span>
                    </el-tooltip>
                  </div>
                </template>
              </el-table-column>
            </el-table>
            <div v-else class="profile-list__empty">{{ $t('settings.sandbox.emptyHostRules') }}</div>
          </div>
        </section>
      </el-form>
      <template #footer>
        <div class="sandbox-dialog__footer">
          <div class="sandbox-dialog__actions">
            <el-button :loading="checking" @click="checkHealth">{{ $t('settings.sandbox.healthCheck') }}</el-button>
            <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
            <el-button type="primary" :loading="saving" @click="save">{{ $t('common.save') }}</el-button>
          </div>
          <span v-if="healthSummary" class="health-summary">{{ healthSummary }}</span>
        </div>
      </template>
    </el-dialog>

    <el-dialog
      v-model="profileDialogVisible"
      width="640px"
      :title="profileEditing ? $t('settings.sandbox.editProfile') : $t('settings.sandbox.addProfile')"
      :close-on-click-modal="false"
      :close-on-press-escape="false"
      :show-close="false"
      destroy-on-close>
      <el-form label-width="130px">
        <el-form-item :label="$t('settings.sandbox.profileName')" required>
          <el-input v-model="profileDraft.name" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.priority')">
          <el-input-number v-model="profileDraft.priority" :min="-1000" :max="1000" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.runtimePreference')">
          <el-select v-model="profileDraft.runtimePreference" style="width: 100%" @change="syncProfileImage">
            <el-option value="auto" :label="$t('settings.sandbox.runtimeAuto')" />
            <el-option value="msb" label="MSB" />
            <el-option value="docker" label="Docker" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.image')" required>
          <el-select v-model="profileDraft.image" style="width: 100%" :loading="runtimeLoading" @change="syncProfileImage">
            <el-option v-for="image in availableImages" :key="image" :label="image" :value="image" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.instanceName')">
          <div class="instance-name-field">
            <el-select
              v-model="profileDraft.instanceName"
              filterable
              allow-create
              default-first-option
              clearable
              style="width: 100%"
              :loading="runtimeLoading">
              <el-option v-for="instance in availableInstances" :key="instance" :label="instance" :value="instance" />
            </el-select>
            <small>{{ $t('settings.sandbox.instanceNameTip') }}</small>
          </div>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.commandPresets')">
          <el-select v-model="profileDraft.commandPresets" multiple filterable style="width: 100%" @change="applyProfileCommandPresets">
            <el-option v-for="preset in Object.keys(COMMAND_PRESETS)" :key="preset" :label="$t(`settings.sandbox.preset${preset}`)" :value="preset" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.commandPatterns')">
          <el-select v-model="profileDraft.commandPatterns" multiple filterable allow-create default-first-option style="width: 100%" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.network')">
          <el-select v-model="profileDraft.network.mode" style="width: 100%">
            <el-option value="none" :label="$t('settings.sandbox.networkNone')" />
            <el-option value="public" :label="$t('settings.sandbox.networkPublic')" />
            <el-option value="host" :label="$t('settings.sandbox.networkHost')" />
            <el-option value="allowlist" :label="$t('settings.sandbox.networkAllowlist')" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="profileDraft.network.mode === 'allowlist'" :label="$t('settings.sandbox.networkAllowlistHosts')">
          <el-select v-model="profileDraft.network.allowlist" multiple filterable allow-create default-first-option style="width: 100%" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.workspaceAccess')">
          <el-select v-model="profileDraft.workspaceAccess" style="width: 100%">
            <el-option value="read_write" :label="$t('settings.sandbox.workspaceReadWrite')" />
            <el-option value="read_only" :label="$t('settings.sandbox.workspaceReadOnly')" />
          </el-select>
        </el-form-item>
        <div class="profile-resources">
          <el-form-item :label="$t('settings.sandbox.cpus')"><el-input-number v-model="profileDraft.resources.cpus" :min="1" :max="16" /></el-form-item>
          <el-form-item :label="$t('settings.sandbox.memoryMb')"><el-input-number v-model="profileDraft.resources.memoryMb" :min="64" :step="64" /></el-form-item>
          <el-form-item :label="$t('settings.sandbox.timeoutMs')"><el-input-number v-model="profileDraft.resources.timeoutMs" :min="1000" :step="1000" /></el-form-item>
        </div>
        <el-form-item :label="$t('settings.sandbox.enabled')">
          <el-switch v-model="profileDraft.enabled" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="profileDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="saveProfile">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog
      v-model="hostRuleDialogVisible"
      width="640px"
      :title="hostRuleEditing ? $t('settings.sandbox.editHostRule') : $t('settings.sandbox.addHostRule')"
      :close-on-click-modal="false"
      :close-on-press-escape="false"
      :show-close="false"
      destroy-on-close>
      <el-form label-width="150px">
        <el-form-item :label="$t('settings.sandbox.ruleName')" required>
          <el-input v-model="hostRuleDraft.name" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.priority')">
          <el-input-number v-model="hostRuleDraft.priority" :min="-1000" :max="1000" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.commandPresets')">
          <el-select v-model="hostRuleDraft.commandPresets" multiple filterable style="width: 100%" @change="applyHostRuleCommandPresets">
            <el-option v-for="preset in HOST_COMMAND_PRESETS" :key="preset" :label="$t(`settings.sandbox.preset${preset}`)" :value="preset" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.commandPatterns')">
          <el-select v-model="hostRuleDraft.commandPatterns" multiple filterable allow-create default-first-option style="width: 100%" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.enabled')">
          <el-switch v-model="hostRuleDraft.enabled" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="hostRuleDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="saveHostRule">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessageBox } from 'element-plus'
import { invoke } from '@tauri-apps/api/core'
import { showMessage } from '@/libs/util'
import { useSandboxSchemeStore } from '@/stores/sandbox_scheme'

const { t } = useI18n()
const schemeStore = useSandboxSchemeStore()
const dialogVisible = ref(false)
const editing = ref(false)
const saving = ref(false)
const checking = ref(false)
const togglingSchemeId = ref('')
const healthSummary = ref('')
const runtimeLoading = ref(false)
const runtimeStatus = ref(null)
const profileDialogVisible = ref(false)
const profileEditing = ref(false)
const profileDraft = ref(null)
const hostRuleDialogVisible = ref(false)
const hostRuleEditing = ref(false)
const hostRuleDraft = ref(null)
const activeRuleTab = ref('profiles')
const draft = ref({ id: '', name: '', description: '', disabled: false, config: null })

const defaultConfig = () => ({ runtimePreference: 'auto', profiles: [], hostRules: [] })
const newId = prefix => `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`
const hasProfileWorkspaceAccess = computed(() =>
  (draft.value.config?.profiles || []).some(profile => profile.workspaceAccess)
)
const workspaceAccessLabel = workspaceAccess => {
  if (workspaceAccess === 'read_write') return t('settings.sandbox.workspaceReadWrite')
  if (workspaceAccess === 'read_only') return t('settings.sandbox.workspaceReadOnly')
  return ''
}
const resetRuleListState = () => {
  activeRuleTab.value = 'profiles'
}
const resetDraft = () => {
  draft.value = { id: '', name: '', description: '', disabled: false, config: defaultConfig() }
  healthSummary.value = ''
  resetRuleListState()
}

const normalizeConfig = config => ({
  ...defaultConfig(),
  ...(config || {}),
  profiles: [...(config?.profiles || [])],
  hostRules: [...(config?.hostRules || [])]
})

const COMMAND_PRESETS = {
  common: ['.*'],
  bash: ['^bash(?:\\s|$)', '^sh(?:\\s|$)', '^zsh(?:\\s|$)'],
  bun: ['^bun(?:\\s|$)'],
  ccpp: ['^cc(?:\\s|$)', '^c\\+\\+(?:\\s|$)', '^clang(?:\\s|$)', '^clang\\+\\+(?:\\s|$)', '^cmake(?:\\s|$)', '^make(?:\\s|$)', '^ninja(?:\\s|$)', '^gcc(?:\\s|$)', '^g\\+\\+(?:\\s|$)'],
  dart: ['^dart(?:\\s|$)', '^flutter(?:\\s|$)'],
  deno: ['^deno(?:\\s|$)'],
  dotnet: ['^dotnet(?:\\s|$)', '^nuget(?:\\s|$)', '^msbuild(?:\\s|$)'],
  elixir: ['^elixir(?:\\s|$)', '^iex(?:\\s|$)', '^mix(?:\\s|$)'],
  erlang: ['^erl(?:\\s|$)', '^erlc(?:\\s|$)', '^rebar3(?:\\s|$)'],
  git: ['^git(?:\\s|$)'],
  go: ['^go(?:\\s|$)'],
  java: ['^java(?:\\s|$)', '^javac(?:\\s|$)', '^mvn(?:\\s|$)', '^mvnw(?:\\s|$)', '^gradle(?:\\s|$)', '^gradlew(?:\\s|$)'],
  kotlin: ['^kotlin(?:\\s|$)', '^kotlinc(?:\\s|$)'],
  lua: ['^lua(?:\\s|$)', '^luajit(?:\\s|$)', '^luarocks(?:\\s|$)'],
  node: ['^node(?:\\s|$)', '^npm(?:\\s|$)', '^pnpm(?:\\s|$)', '^yarn(?:\\s|$)', '^npx(?:\\s|$)'],
  perl: ['^perl(?:\\s|$)', '^cpan(?:\\s|$)', '^prove(?:\\s|$)'],
  php: ['^php(?:\\s|$)', '^composer(?:\\s|$)'],
  python: ['^python(?:\\s|$)', '^python3(?:\\s|$)', '^pip(?:\\s|$)', '^pip3(?:\\s|$)'],
  r: ['^R(?:\\s|$)', '^Rscript(?:\\s|$)'],
  ruby: ['^ruby(?:\\s|$)', '^gem(?:\\s|$)', '^bundle(?:\\s|$)', '^bundler(?:\\s|$)', '^rake(?:\\s|$)', '^rails(?:\\s|$)'],
  rust: ['^cargo(?:\\s|$)', '^rustc(?:\\s|$)', '^rustup(?:\\s|$)', '^rustfmt(?:\\s|$)', '^rustdoc(?:\\s|$)', '^cargo-fmt(?:\\s|$)', '^cargo-clippy(?:\\s|$)', '^clippy-driver(?:\\s|$)', '^cargo-miri(?:\\s|$)', '^miri(?:\\s|$)', '^rust-analyzer(?:\\s|$)', '^rust-gdb(?:\\s|$)', '^rust-gdbgui(?:\\s|$)', '^rust-lldb(?:\\s|$)'],
  shell: ['^bash(?:\\s|$)', '^sh(?:\\s|$)', '^zsh(?:\\s|$)', '^echo(?:\\s|$)', '^printf(?:\\s|$)', '^pwd(?:\\s|$)', '^ls(?:\\s|$)', '^cat(?:\\s|$)', '^head(?:\\s|$)', '^tail(?:\\s|$)', '^wc(?:\\s|$)', '^grep(?:\\s|$)', '^rg(?:\\s|$)', '^find(?:\\s|$)', '^sed(?:\\s|$)', '^awk(?:\\s|$)', '^sort(?:\\s|$)', '^uniq(?:\\s|$)', '^cut(?:\\s|$)', '^tr(?:\\s|$)', '^cp(?:\\s|$)', '^mv(?:\\s|$)', '^rm(?:\\s|$)', '^mkdir(?:\\s|$)', '^touch(?:\\s|$)', '^chmod(?:\\s|$)', '^ln(?:\\s|$)', '^readlink(?:\\s|$)', '^realpath(?:\\s|$)', '^basename(?:\\s|$)', '^dirname(?:\\s|$)', '^env(?:\\s|$)', '^which(?:\\s|$)', '^date(?:\\s|$)', '^sleep(?:\\s|$)', '^test(?:\\s|$)', '^true(?:\\s|$)', '^false(?:\\s|$)', '^diff(?:\\s|$)', '^patch(?:\\s|$)', '^tar(?:\\s|$)', '^gzip(?:\\s|$)', '^gunzip(?:\\s|$)', '^zip(?:\\s|$)', '^unzip(?:\\s|$)', '^curl(?:\\s|$)', '^wget(?:\\s|$)'],
  swift: ['^swift(?:\\s|$)', '^swiftc(?:\\s|$)'],
  tauri: ['^(?:pnpm|npm|yarn|npx)(?:\\s+run)?\\s+tauri(?:\\s|$)', '^cargo\\s+tauri(?:\\s|$)', '^tauri(?:\\s|$)'],
  zig: ['^zig(?:\\s|$)']
}

const HOST_COMMAND_PRESETS = Object.keys(COMMAND_PRESETS).filter(preset => preset !== 'common')

const defaultProfile = () => ({
  id: '',
  _draftKey: newId('profile'),
  name: '',
  enabled: true,
  priority: 0,
  commandPresets: [],
  commandPatterns: [],
  runtimePreference: 'auto',
  image: '',
  instanceName: '',
  imageSizeBytes: null,
  network: { mode: 'none', allowlist: [] },
  resources: { cpus: 1, memoryMb: 256, timeoutMs: 120000 },
  workspaceAccess: 'read_write'
})

const cloneProfile = profile => ({
  ...profile,
  commandPresets: [...(profile.commandPresets || [])],
  commandPatterns: [...(profile.commandPatterns || [])],
  network: { ...profile.network, allowlist: [...(profile.network?.allowlist || [])] },
  resources: { ...profile.resources }
})

const cloneSchemeConfig = config => ({
  ...config,
  profiles: config.profiles.map(profile => {
    const { _draftKey, commandPresets: _commandPresets, instanceName, ...persistedProfile } = cloneProfile(profile)
    return {
      ...persistedProfile,
      ...(instanceName?.trim() ? { instanceName: instanceName.trim() } : {})
    }
  }),
  hostRules: config.hostRules.map(rule => {
    const { _draftKey, commandPresets: _commandPresets, ...persistedRule } = rule
    return {
      ...persistedRule,
      commandPatterns: [...(rule.commandPatterns || [])]
    }
  })
})

const normalizeProfile = profile => ({
  ...defaultProfile(),
  ...(profile || {}),
  commandPresets: [],
  commandPatterns: [...(profile?.commandPatterns || [])],
  network: { mode: 'none', allowlist: [], ...(profile?.network || {}) },
  resources: { cpus: 1, memoryMb: 256, timeoutMs: 120000, ...(profile?.resources || {}) }
})

const defaultHostRule = () => ({
  _draftKey: newId('host-rule'),
  name: '',
  enabled: true,
  priority: 0,
  commandPresets: [],
  commandPatterns: []
})

const normalizeHostRule = rule => ({
  ...defaultHostRule(),
  ...(rule || {}),
  commandPresets: [],
  commandPatterns: [...(rule?.commandPatterns || [])]
})

const AVAILABLE_RUNTIME_STATES = new Set(['ready', 'ready_missing_image'])
const runtimeKeys = preference => preference === 'auto' ? ['msb', 'docker'] : [preference]
const effectiveRuntimePreference = profile =>
  profile?.runtimePreference && profile.runtimePreference !== 'auto'
    ? profile.runtimePreference
    : (draft.value.config?.runtimePreference || 'auto')
const availableImages = computed(() => {
  const images = new Set()
  for (const runtime of runtimeKeys(effectiveRuntimePreference(profileDraft.value))) {
    const status = runtimeStatus.value?.[runtime]
    if (!AVAILABLE_RUNTIME_STATES.has(status?.state)) continue
    for (const image of status.images || []) images.add(image)
  }
  return [...images].sort()
})

const availableInstances = computed(() => {
  const instances = new Set()
  for (const runtime of runtimeKeys(effectiveRuntimePreference(profileDraft.value))) {
    const status = runtimeStatus.value?.[runtime]
    if (!AVAILABLE_RUNTIME_STATES.has(status?.state)) continue
    for (const instance of status.runningInstances || []) instances.add(instance)
  }
  return [...instances].sort()
})

const loadRuntimeStatus = async () => {
  runtimeLoading.value = true
  try {
    runtimeStatus.value = await invoke('get_sandbox_scheme_runtime_status', {
      config: draft.value.config
    })
  } catch (error) {
    runtimeStatus.value = null
    showMessage(String(error), 'error')
  } finally {
    runtimeLoading.value = false
  }
}

const imageSizeForProfile = profile => {
  const sizes = runtimeKeys(effectiveRuntimePreference(profile))
    .flatMap(runtime => {
      const size = runtimeStatus.value?.[runtime]?.imageSizes?.[profile.image]
      return Number.isSafeInteger(size) && size >= 0 ? [size] : []
    })
  return sizes.length ? Math.min(...sizes) : null
}

const syncProfileImage = () => {
  if (!availableImages.value.includes(profileDraft.value.image)) {
    profileDraft.value.image = availableImages.value[0] || ''
  }
  profileDraft.value.imageSizeBytes = imageSizeForProfile(profileDraft.value)
}

const openProfileEditor = async profile => {
  profileEditing.value = Boolean(profile)
  profileDraft.value = normalizeProfile(profile)
  await loadRuntimeStatus()
  syncProfileImage()
  profileDialogVisible.value = true
}

const addCommandPresetPatterns = (presets, commandPatterns) => {
  const patterns = new Set(commandPatterns || [])
  const selectedPresets = new Set(presets || [])
  for (const preset of selectedPresets) {
    for (const pattern of COMMAND_PRESETS[preset] || []) patterns.add(pattern)
  }
  return [...patterns]
}

const applyProfileCommandPresets = presets => {
  profileDraft.value.commandPatterns = addCommandPresetPatterns(
    presets,
    profileDraft.value.commandPatterns
  )
}

const applyHostRuleCommandPresets = presets => {
  hostRuleDraft.value.commandPatterns = addCommandPresetPatterns(
    presets,
    hostRuleDraft.value.commandPatterns
  )
}

const saveProfile = () => {
  const value = profileDraft.value
  if (!value.name.trim() || !value.image) {
    showMessage(t('settings.sandbox.profileRequired'), 'error')
    return
  }
  value.imageSizeBytes = imageSizeForProfile(value)
  const profiles = draft.value.config.profiles
  const existingIndex = profiles.findIndex(profile =>
    profile._draftKey === value._draftKey || (value.id && profile.id === value.id)
  )
  if (existingIndex === -1) profiles.push(cloneProfile(value))
  else profiles.splice(existingIndex, 1, cloneProfile(value))
  profileDialogVisible.value = false
}

const removeProfile = target => {
  draft.value.config.profiles = draft.value.config.profiles.filter(profile => profile !== target)
}

const schemeSummary = scheme => {
  const profiles = scheme.config?.profiles?.length || 0
  const rules = scheme.config?.hostRules?.length || 0
  return `${profiles} ${t('settings.sandbox.profiles')} · ${rules} ${t('settings.sandbox.hostRules')}`
}

const openHostRuleEditor = rule => {
  hostRuleEditing.value = Boolean(rule)
  hostRuleDraft.value = normalizeHostRule(rule)
  hostRuleDialogVisible.value = true
}

const saveHostRule = () => {
  const value = hostRuleDraft.value
  if (!value.name.trim()) {
    showMessage(t('settings.sandbox.ruleRequired'), 'error')
    return
  }

  if (!value.commandPatterns.length) {
    showMessage(t('settings.sandbox.ruleCriteriaRequired'), 'error')
    return
  }
  if (value.commandPatterns.some(isCatchAllPattern)) {
    showMessage(t('settings.sandbox.hostRuleCatchAllForbidden'), 'error')
    return
  }

  const rules = draft.value.config.hostRules
  const existingIndex = rules.findIndex(rule =>
    rule._draftKey === value._draftKey || (value.id && rule.id === value.id)
  )
  if (existingIndex === -1) rules.push(normalizeHostRule(value))
  else rules.splice(existingIndex, 1, normalizeHostRule(value))
  hostRuleDialogVisible.value = false
}

const removeHostRule = target => {
  draft.value.config.hostRules = draft.value.config.hostRules.filter(rule => rule !== target)
}

const openCreate = () => {
  editing.value = false
  resetDraft()
  dialogVisible.value = true
}

const openEdit = scheme => {
  editing.value = true
  draft.value = { ...scheme, config: normalizeConfig(scheme.config) }
  healthSummary.value = ''
  resetRuleListState()
  dialogVisible.value = true
}

const toggleSchemeEnabled = async (scheme, enabled) => {
  togglingSchemeId.value = scheme.id
  try {
    await invoke('update_sandbox_scheme', {
      scheme: {
        ...scheme,
        disabled: !enabled,
        config: cloneSchemeConfig(normalizeConfig(scheme.config))
      }
    })
    await schemeStore.fetchSchemes()
  } catch (error) {
    showMessage(String(error), 'error')
  } finally {
    togglingSchemeId.value = ''
  }
}

const isCatchAllPattern = pattern => ['.*', '^.*', '.*$', '^.*$'].includes(String(pattern || '').trim())

const validateDraft = () => {
  if (!draft.value.name.trim()) return t('settings.sandbox.nameRequired')
  for (const profile of draft.value.config.profiles) {
    if (!profile.name.trim() || !profile.image.trim()) return t('settings.sandbox.profileRequired')
    if (!profile.commandPatterns?.length) return t('settings.sandbox.profileCommandPatternsRequired')
  }
  for (const rule of draft.value.config.hostRules) {
    if (!rule.name.trim()) return t('settings.sandbox.ruleRequired')
    if (!rule.commandPatterns?.length) {
      return t('settings.sandbox.ruleCriteriaRequired')
    }
    if (rule.commandPatterns.some(isCatchAllPattern)) {
      return t('settings.sandbox.hostRuleCatchAllForbidden')
    }
  }
  return ''
}

const checkHealth = async () => {
  const error = validateDraft()
  if (error) return showMessage(error, 'error')
  checking.value = true
  try {
    const status = await invoke('get_sandbox_scheme_runtime_status', { config: draft.value.config })
    healthSummary.value = ['msb', 'docker']
      .map(runtime => `${runtime.toUpperCase()}: ${status[runtime]?.state || 'unknown'}`)
      .join(' · ')
  } catch (error) {
    showMessage(String(error), 'error')
  } finally {
    checking.value = false
  }
}

const save = async () => {
  const error = validateDraft()
  if (error) return showMessage(error, 'error')
  saving.value = true
  try {
    const config = cloneSchemeConfig(draft.value.config)
    const scheme = { ...draft.value, name: draft.value.name.trim(), config }
    if (editing.value) {
      await invoke('update_sandbox_scheme', { scheme })
    } else {
      await invoke('add_sandbox_scheme', { scheme })
    }
    await schemeStore.fetchSchemes()
    dialogVisible.value = false
    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    showMessage(String(error), 'error')
  } finally {
    saving.value = false
  }
}

const remove = async scheme => {
  try {
    await ElMessageBox.confirm(t('settings.sandbox.deleteConfirm', { name: scheme.name }), t('common.warning'), {
      type: 'warning'
    })
    await invoke('delete_sandbox_scheme', { id: scheme.id })
    await schemeStore.fetchSchemes()
    showMessage(t('common.deleteSuccess'), 'success')
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') showMessage(String(error), 'error')
  }
}

schemeStore.fetchSchemes()
</script>

<style scoped lang="scss">
.sandbox-schemes {
  max-width: 1080px;
  margin: 0 auto;

  .card {
    .title,
    .item,
    .label,
    .value,
    .profile-list__row,
    .profile-list__actions {
      display: flex;
      align-items: center;
    }

    .title {
      justify-content: space-between;
      padding: var(--cs-space-sm) var(--cs-space);
      font-weight: 600;
    }

    .list {
      border-top: 1px solid var(--cs-border-color);
    }

    .item {
      min-height: 58px;
      justify-content: space-between;
      gap: var(--cs-space);
      padding: 0 var(--cs-space);
      border-bottom: 1px solid var(--cs-border-color);
    }

    .label { min-width: 0; gap: var(--cs-space-sm); }
    .label-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .label-text small,
    .profile-list__row small { display: block; color: var(--cs-text-color-secondary); font-size: var(--cs-font-size-sm); }
    .value { flex-shrink: 0; gap: var(--cs-space-sm); }
    .scheme-marker { display: grid; place-items: center; width: 28px; height: 28px; border-radius: var(--cs-border-radius); background: var(--cs-bg-color-light); }
    .icon { cursor: pointer; }
    .empty-state { display: flex; align-items: center; justify-content: space-between; gap: var(--cs-space); padding: var(--cs-space-lg) var(--cs-space); color: var(--cs-text-color-secondary); }
  }

  .editor-section { margin-top: var(--cs-space-lg); }
  .editor-section--rules { min-width: 0; }
  .rule-tabs__header { display: flex; align-items: center; gap: var(--cs-space); margin-bottom: 5px; }
  .rule-tabs { flex: 1; min-width: 0; }
  .rule-tabs :deep(.el-tabs__header) { margin: 0; }
  .rule-tabs :deep(.el-tabs__content) { display: none; }
  .sandbox-dialog__footer { display: flex; align-items: center; justify-content: space-between; gap: var(--cs-space); }
  .sandbox-dialog__actions { display: flex; gap: var(--cs-space-sm); flex-shrink: 0; }
  .sandbox-dialog__actions :deep(.el-button + .el-button) { margin-left: 0; }
  .instance-name-field { width: 100%; }
  .instance-name-field small { display: block; margin-top: var(--cs-space-xs); color: var(--cs-text-color-secondary); line-height: 1.4; }
  .editor-card { margin-bottom: var(--cs-space-sm); }
  .editor-card__actions { display: flex; justify-content: flex-end; }
  .health-summary { color: var(--cs-text-color-secondary); margin-left: auto; }

  .rule-table { width: 100%; }
  .rule-table__actions { display: flex; justify-content: flex-end; gap: var(--cs-space-xs); }
  .rule-table__actions .icon { display: grid; place-items: center; width: 28px; height: 28px; border-radius: var(--cs-border-radius); }
  .rule-table__actions .icon:hover { background: var(--cs-bg-color-dark); }
  .profile-list__empty { padding: var(--cs-space); color: var(--cs-text-color-secondary); border: 1px dashed var(--cs-border-color); border-radius: var(--cs-border-radius); }

  .profile-resources { display: grid; grid-template-columns: 1fr; gap: var(--cs-space-sm); }
  .profile-resources :deep(.el-form-item) { margin-bottom: 0; }

  @media (max-width: 680px) {
    .profile-resources { grid-template-columns: 1fr; }
    .card .item { align-items: flex-start; padding: var(--cs-space-sm); }
    .card .value { gap: var(--cs-space-xs); }
  }
}
</style>
