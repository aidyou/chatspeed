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
            <el-tag size="small" :type="scheme.disabled ? 'info' : 'success'">
              {{ scheme.disabled ? $t('settings.sandbox.disabled') : $t('settings.sandbox.enabled') }}
            </el-tag>
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

        <section class="editor-section">
          <div class="editor-section__header">
            <h3>{{ $t('settings.sandbox.profiles') }}</h3>
            <el-button size="small" @click="openProfileEditor()">{{ $t('settings.sandbox.addProfile') }}</el-button>
          </div>
          <div v-if="draft.config.profiles.length" class="profile-list">
            <div v-for="profile in draft.config.profiles" :key="profile._draftKey || profile.id" class="profile-list__row">
              <div>
                <strong>{{ profile.name }}</strong>
                <small>{{ profile.image }} · {{ profile.capabilities.join(', ') || $t('settings.sandbox.capabilities') }}</small>
              </div>
              <div class="profile-list__actions">
                <el-tag size="small">{{ profile.priority }}</el-tag>
                <span class="icon" @click="openProfileEditor(profile)"><cs name="edit" size="16px" color="secondary" /></span>
                <span class="icon" @click="removeProfile(profile)"><cs name="trash" size="16px" color="secondary" /></span>
              </div>
            </div>
          </div>
          <div v-else class="profile-list__empty">{{ $t('settings.sandbox.emptyProfiles') }}</div>
        </section>

        <section class="editor-section">
          <div class="editor-section__header">
            <h3>{{ $t('settings.sandbox.hostRules') }}</h3>
            <el-button size="small" @click="openHostRuleEditor()">{{ $t('settings.sandbox.addHostRule') }}</el-button>
          </div>
          <div v-if="draft.config.hostRules.length" class="profile-list">
            <div v-for="rule in draft.config.hostRules" :key="rule._draftKey || rule.id" class="profile-list__row">
              <div>
                <strong>{{ rule.name }}</strong>
                <small>{{ rule.capabilitiesAll.join(', ') || rule.invocationTagsAny.join(', ') || $t('settings.sandbox.hostRules') }}</small>
              </div>
              <div class="profile-list__actions">
                <el-tag size="small">{{ rule.priority }}</el-tag>
                <span class="icon" @click="openHostRuleEditor(rule)"><cs name="edit" size="16px" color="secondary" /></span>
                <span class="icon" @click="removeHostRule(rule)"><cs name="trash" size="16px" color="secondary" /></span>
              </div>
            </div>
          </div>
          <div v-else class="profile-list__empty">{{ $t('settings.sandbox.emptyHostRules') }}</div>
        </section>
      </el-form>
      <template #footer>
        <span v-if="healthSummary" class="health-summary">{{ healthSummary }}</span>
        <el-button :loading="checking" @click="checkHealth">{{ $t('settings.sandbox.healthCheck') }}</el-button>
        <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="saving" @click="save">{{ $t('common.save') }}</el-button>
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
        <el-form-item :label="$t('settings.sandbox.capabilities')">
          <el-select v-model="profileDraft.capabilities" multiple filterable style="width: 100%" @change="applyCapabilityPresets">
            <el-option v-for="capability in Object.keys(CAPABILITY_PRESETS)" :key="capability" :label="$t(`settings.sandbox.capability${capability}`)" :value="capability" />
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
        <el-form-item :label="$t('settings.sandbox.ruleId')">
          <el-input v-model="hostRuleDraft.id" readonly />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.priority')">
          <el-input-number v-model="hostRuleDraft.priority" :min="-1000" :max="1000" />
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.capabilities')">
          <el-select v-model="hostRuleDraft.capabilitiesAll" multiple filterable style="width: 100%">
            <el-option v-for="capability in Object.keys(CAPABILITY_PRESETS)" :key="capability" :label="$t(`settings.sandbox.capability${capability}`)" :value="capability" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('settings.sandbox.invocationTags')">
          <el-select v-model="hostRuleDraft.invocationTagsAny" multiple filterable allow-create default-first-option style="width: 100%" />
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
const healthSummary = ref('')
const runtimeLoading = ref(false)
const runtimeStatus = ref(null)
const profileDialogVisible = ref(false)
const profileEditing = ref(false)
const profileDraft = ref(null)
const hostRuleDialogVisible = ref(false)
const hostRuleEditing = ref(false)
const hostRuleDraft = ref(null)
const draft = ref({ id: '', name: '', description: '', disabled: false, config: null })

const defaultConfig = () => ({ runtimePreference: 'auto', profiles: [], hostRules: [] })
const newId = prefix => `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`
const resetDraft = () => {
  draft.value = { id: '', name: '', description: '', disabled: false, config: defaultConfig() }
  healthSummary.value = ''
}

const normalizeConfig = config => ({
  ...defaultConfig(),
  ...(config || {}),
  profiles: [...(config?.profiles || [])],
  hostRules: [...(config?.hostRules || [])]
})

const CAPABILITY_PRESETS = {
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

const defaultProfile = () => ({
  id: '',
  _draftKey: newId('profile'),
  name: '',
  enabled: true,
  priority: 0,
  capabilities: [],
  commandPatterns: [],
  runtimePreference: 'auto',
  image: '',
  imageSizeBytes: null,
  network: { mode: 'none', allowlist: [] },
  resources: { cpus: 1, memoryMb: 256, timeoutMs: 120000 },
  workspaceAccess: 'read_write'
})

const cloneProfile = profile => ({
  ...profile,
  capabilities: [...(profile.capabilities || [])],
  commandPatterns: [...(profile.commandPatterns || [])],
  network: { ...profile.network, allowlist: [...(profile.network?.allowlist || [])] },
  resources: { ...profile.resources }
})

const cloneSchemeConfig = config => ({
  ...config,
  profiles: config.profiles.map(profile => {
    const { _draftKey, ...persistedProfile } = cloneProfile(profile)
    return persistedProfile
  }),
  hostRules: config.hostRules.map(rule => {
    const { _draftKey, ...persistedRule } = rule
    return {
      ...persistedRule,
      capabilitiesAll: [...(rule.capabilitiesAll || [])],
      invocationTagsAny: [...(rule.invocationTagsAny || [])],
      commandPatterns: [...(rule.commandPatterns || [])]
    }
  })
})

const normalizeProfile = profile => ({
  ...defaultProfile(),
  ...(profile || {}),
  capabilities: [...(profile?.capabilities || [])],
  commandPatterns: [...(profile?.commandPatterns || [])],
  network: { mode: 'none', allowlist: [], ...(profile?.network || {}) },
  resources: { cpus: 1, memoryMb: 256, timeoutMs: 120000, ...(profile?.resources || {}) }
})

const defaultHostRule = () => ({
  id: newId('host'),
  _draftKey: newId('host-rule'),
  name: '',
  enabled: true,
  priority: 0,
  capabilitiesAll: [],
  invocationTagsAny: [],
  commandPatterns: []
})

const normalizeHostRule = rule => ({
  ...defaultHostRule(),
  ...(rule || {}),
  capabilitiesAll: [...(rule?.capabilitiesAll || [])],
  invocationTagsAny: [...(rule?.invocationTagsAny || [])],
  commandPatterns: [...(rule?.commandPatterns || [])]
})

const AVAILABLE_RUNTIME_STATES = new Set(['ready', 'ready_missing_image'])
const runtimeKeys = preference => preference === 'auto' ? ['msb', 'docker'] : [preference]
const availableImages = computed(() => {
  const images = new Set()
  for (const runtime of runtimeKeys(profileDraft.value?.runtimePreference || 'auto')) {
    const status = runtimeStatus.value?.[runtime]
    if (!AVAILABLE_RUNTIME_STATES.has(status?.state)) continue
    for (const image of status.images || []) images.add(image)
  }
  return [...images].sort()
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
  const sizes = runtimeKeys(profile.runtimePreference)
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

const normalizeCapabilitySelection = (capabilities, commandPatterns) => {
  const selected = [...new Set(capabilities || [])]
  if (selected.includes('common')) {
    return { capabilities: ['common'], commandPatterns: ['.*'] }
  }

  const patterns = new Set((commandPatterns || []).filter(pattern => pattern !== '.*'))
  for (const capability of selected) {
    for (const pattern of CAPABILITY_PRESETS[capability] || []) patterns.add(pattern)
  }
  return { capabilities: selected, commandPatterns: [...patterns] }
}

const applyCapabilityPresets = capabilities => {
  const selected = [...new Set(capabilities)]
  const existingPatterns = profileDraft.value.commandPatterns || []
  const hadCommonPattern = existingPatterns.includes('.*')

  if (selected.includes('common') && (selected.length === 1 || !hadCommonPattern)) {
    Object.assign(profileDraft.value, normalizeCapabilitySelection(['common'], []))
    return
  }

  Object.assign(
    profileDraft.value,
    normalizeCapabilitySelection(selected.filter(capability => capability !== 'common'), existingPatterns)
  )
}

const saveProfile = () => {
  const value = profileDraft.value
  if (!value.name.trim() || !value.image) {
    showMessage(t('settings.sandbox.profileRequired'), 'error')
    return
  }
  Object.assign(value, normalizeCapabilitySelection(value.capabilities, value.commandPatterns))
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
  if (!value.capabilitiesAll.length && !value.invocationTagsAny.length && !value.commandPatterns.length) {
    showMessage(t('settings.sandbox.ruleCriteriaRequired'), 'error')
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
  dialogVisible.value = true
}

const validateDraft = () => {
  if (!draft.value.name.trim()) return t('settings.sandbox.nameRequired')
  for (const profile of draft.value.config.profiles) {
    if (!profile.name.trim() || !profile.image.trim()) return t('settings.sandbox.profileRequired')
  }
  for (const rule of draft.value.config.hostRules) {
    if (!rule.id.trim() || !rule.name.trim()) return t('settings.sandbox.ruleRequired')
    if (!rule.capabilitiesAll.length && !rule.invocationTagsAny.length && !rule.commandPatterns.length) {
      return t('settings.sandbox.ruleCriteriaRequired')
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
  .editor-section__header { display: flex; align-items: center; justify-content: space-between; margin-bottom: var(--cs-space-sm); }
  .editor-section__header h3 { margin: 0; font-size: var(--cs-font-size); }
  .editor-card { margin-bottom: var(--cs-space-sm); }
  .editor-card__actions { display: flex; justify-content: flex-end; }
  .health-summary { color: var(--cs-text-color-secondary); margin-right: auto; }

  .profile-list { display: grid; gap: var(--cs-space-xs); }
  .profile-list__row { justify-content: space-between; gap: var(--cs-space); min-height: 52px; padding: var(--cs-space-xs) var(--cs-space-sm); border: 1px solid var(--cs-border-color); border-radius: var(--cs-border-radius); background: var(--cs-bg-color-light); }
  .profile-list__actions { gap: var(--cs-space-sm); flex-shrink: 0; }
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
