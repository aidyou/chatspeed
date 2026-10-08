<template>
  <div class="agent-skills">
    <div class="card">
      <div class="title">
        <span>{{ t('settings.agentSkills.title') }}</span>
        <div class="actions">
          <el-tooltip :content="t('settings.agentSkills.rescan')" placement="left" :hide-after="0">
            <span class="icon" :class="{ disabled: store.loading }" @click="rescan">
              <cs name="refresh" />
            </span>
          </el-tooltip>
          <el-tooltip :content="t('settings.agentSkills.doctor')" placement="left" :hide-after="0">
            <span class="icon" @click="runDoctor">
              <cs name="check-circle" />
            </span>
          </el-tooltip>
          <el-tooltip :content="t('settings.agentSkills.reconcile')" placement="left" :hide-after="0">
            <span
              class="icon"
              :class="{ disabled: store.reconciling }"
              @click="runReconcile"
            >
              <cs name="refresh" />
            </span>
          </el-tooltip>
        </div>
      </div>

      <p class="hint">{{ t('settings.agentSkills.hint') }}</p>

      <!-- source -->
      <div class="section">
        <div class="section-title">{{ t('settings.agentSkills.source') }}</div>
        <el-radio-group v-model="sourceKind" size="small">
          <el-radio-button value="local_directory">
            {{ t('settings.agentSkills.sourceLocal') }}
          </el-radio-button>
          <el-radio-button value="local_zip">
            {{ t('settings.agentSkills.sourceZip') }}
          </el-radio-button>
          <el-radio-button value="github">{{ t('settings.agentSkills.sourceGithub') }}</el-radio-button>
        </el-radio-group>

        <div class="fields">
          <el-input
            v-if="sourceKind === 'local_directory' || sourceKind === 'local_zip'"
            v-model="sourcePath"
            :placeholder="
              sourceKind === 'local_zip'
                ? t('settings.agentSkills.zipPlaceholder')
                : t('settings.agentSkills.directoryPlaceholder')
            "
          >
            <template #prepend>{{ t('settings.agentSkills.path') }}</template>
          </el-input>

          <template v-else>
            <el-input v-model="githubRepository" placeholder="owner/repository">
              <template #prepend>{{ t('settings.agentSkills.repository') }}</template>
            </el-input>
            <el-input v-model="githubPath" placeholder="skills/demo">
              <template #prepend>{{ t('settings.agentSkills.path') }}</template>
            </el-input>
            <el-input v-model="githubRef" placeholder="main">
              <template #prepend>{{ t('settings.agentSkills.ref') }}</template>
            </el-input>
          </template>

          <div class="buttons">
            <el-button size="small" :loading="store.checking" :disabled="!canCheck" @click="check">
              {{ t('settings.agentSkills.check') }}
            </el-button>
          </div>
        </div>

        <el-alert
          v-if="store.lastError"
          class="alert"
          type="error"
          :closable="false"
          :title="store.lastError"
        />

        <!-- check result -->
        <div v-if="report" class="report">
          <div class="report-head">
            <el-tag :type="verdictTagType" size="small" effect="dark">{{ report.verdict }}</el-tag>
            <span class="meta">{{ report.skill_name || '-' }}</span>
            <span class="meta">{{ report.checker_version }}</span>
            <span class="meta">{{
              t('settings.agentSkills.size', { files: report.file_count, bytes: report.total_bytes })
            }}</span>
          </div>

          <div class="permissions">
            <el-tag
              v-for="(allowed, name) in report.permissions"
              :key="name"
              size="small"
              :type="allowed === false ? 'info' : 'warning'"
              effect="plain"
            >
              {{ t(`settings.agentSkills.permission.${name}`) }}
            </el-tag>
          </div>

          <el-table
            v-if="report.findings?.length"
            :data="report.findings"
            size="small"
            class="findings skill-table"
          >
            <el-table-column prop="severity" :label="t('settings.agentSkills.severity')" width="110" />
            <el-table-column prop="rule" :label="t('settings.agentSkills.rule')" width="220" />
            <el-table-column prop="path" :label="t('settings.agentSkills.path')" width="220" />
            <el-table-column prop="detail" :label="t('settings.agentSkills.detail')" />
          </el-table>

          <!-- The same findings as data-driven cards: the wide table cannot fit a narrow
               container, so every column is shown as a labelled row instead of being cut. -->
          <ul v-if="report.findings?.length" class="findings skill-cards">
            <li v-for="(row, index) in report.findings" :key="index" class="skill-cards__item">
              <div class="skill-cards__row">
                <span class="skill-cards__label">{{ t('settings.agentSkills.severity') }}</span>
                <span class="skill-cards__value">{{ row.severity }}</span>
              </div>
              <div class="skill-cards__row">
                <span class="skill-cards__label">{{ t('settings.agentSkills.rule') }}</span>
                <span class="skill-cards__value">{{ row.rule }}</span>
              </div>
              <div class="skill-cards__row">
                <span class="skill-cards__label">{{ t('settings.agentSkills.path') }}</span>
                <span class="skill-cards__value">{{ row.path }}</span>
              </div>
              <div class="skill-cards__row">
                <span class="skill-cards__label">{{ t('settings.agentSkills.detail') }}</span>
                <span class="skill-cards__value">{{ row.detail }}</span>
              </div>
            </li>
          </ul>

          <!-- targets -->
          <div class="section-title">{{ t('settings.agentSkills.targets') }}</div>
          <el-checkbox-group v-model="selection">
            <el-tooltip
              v-for="target in store.targets"
              :key="target.id"
              :disabled="target.supported"
              :content="target.unsupported_reason || ''"
              placement="top"
            >
              <el-checkbox :value="target.id" :disabled="!target.supported">
                {{ target.id }}
                <span v-if="target.external" class="external">{{
                  t('settings.agentSkills.external')
                }}</span>
              </el-checkbox>
            </el-tooltip>
          </el-checkbox-group>

          <div class="buttons">
            <el-button
              type="primary"
              size="small"
              :loading="store.applying"
              :disabled="!canInstall"
              @click="install"
            >
              {{ t('settings.agentSkills.install') }}
            </el-button>
          </div>
        </div>
      </div>

      <!-- install outcomes -->
      <div v-if="outcomes.length" class="section">
        <div class="section-title">{{ t('settings.agentSkills.result') }}</div>
        <el-table :data="outcomes" size="small" class="skill-table">
          <el-table-column prop="target_id" :label="t('settings.agentSkills.target')" width="140" />
          <el-table-column :label="t('settings.agentSkills.status')" width="160">
            <template #default="{ row }">
              <el-tag size="small" :type="statusTagType(row.status)">{{ row.status }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="t('settings.agentSkills.detail')">
            <template #default="{ row }">{{ row.detail || row.install_path || '-' }}</template>
          </el-table-column>
        </el-table>

        <ul class="skill-cards">
          <li v-for="(row, index) in outcomes" :key="index" class="skill-cards__item">
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.target') }}</span>
              <span class="skill-cards__value">{{ row.target_id }}</span>
            </div>
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.status') }}</span>
              <span class="skill-cards__value">
                <el-tag size="small" :type="statusTagType(row.status)">{{ row.status }}</el-tag>
              </span>
            </div>
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.detail') }}</span>
              <span class="skill-cards__value">{{ row.detail || row.install_path || '-' }}</span>
            </div>
          </li>
        </ul>
      </div>

      <!-- installed skills -->
      <div class="section">
        <div class="section-title">{{ t('settings.agentSkills.installed') }}</div>
        <el-table v-loading="store.loading" :data="store.skills" size="small" class="skill-table" :empty-text="t('settings.agentSkills.empty')">
          <el-table-column prop="name" :label="t('settings.agentSkills.name')" width="180" />
          <el-table-column prop="source" :label="t('settings.agentSkills.origin')" width="120" />
          <el-table-column :label="t('settings.agentSkills.target')" width="140">
            <template #default="{ row }">{{ row.target_id || '-' }}</template>
          </el-table-column>
          <el-table-column :label="t('settings.agentSkills.state')" width="220">
            <template #default="{ row }">
              <el-tag v-if="row.protected" size="small" type="info" effect="plain">{{
                t('settings.agentSkills.protected')
              }}</el-tag>
              <el-tag v-if="row.managed" size="small" effect="plain">{{
                t('settings.agentSkills.managed')
              }}</el-tag>
              <el-tag v-if="row.drifted" size="small" type="warning" effect="plain">{{
                t('settings.agentSkills.drifted')
              }}</el-tag>
              <el-tag v-if="!row.present" size="small" type="danger" effect="plain">{{
                t('settings.agentSkills.missing')
              }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="t('settings.agentSkills.actions')" width="140">
            <template #default="{ row }">
              <el-button
                size="small"
                text
                type="danger"
                :disabled="!row.uninstallable"
                :loading="store.applying"
                @click="uninstall(row)"
              >
                {{ t('settings.agentSkills.uninstall') }}
              </el-button>
            </template>
          </el-table-column>
        </el-table>

        <ul v-loading="store.loading" class="skill-cards">
          <li v-for="(row, index) in store.skills" :key="`${row.target_id || ''}:${row.name}:${index}`" class="skill-cards__item">
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.name') }}</span>
              <span class="skill-cards__value">{{ row.name }}</span>
            </div>
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.origin') }}</span>
              <span class="skill-cards__value">{{ row.source }}</span>
            </div>
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.target') }}</span>
              <span class="skill-cards__value">{{ row.target_id || '-' }}</span>
            </div>
            <div class="skill-cards__row">
              <span class="skill-cards__label">{{ t('settings.agentSkills.state') }}</span>
              <span class="skill-cards__value skill-cards__tags">
                <el-tag v-if="row.protected" size="small" type="info" effect="plain">{{
                  t('settings.agentSkills.protected')
                }}</el-tag>
                <el-tag v-if="row.managed" size="small" effect="plain">{{
                  t('settings.agentSkills.managed')
                }}</el-tag>
                <el-tag v-if="row.drifted" size="small" type="warning" effect="plain">{{
                  t('settings.agentSkills.drifted')
                }}</el-tag>
                <el-tag v-if="!row.present" size="small" type="danger" effect="plain">{{
                  t('settings.agentSkills.missing')
                }}</el-tag>
              </span>
            </div>
            <div class="skill-cards__actions">
              <el-button
                size="small"
                text
                type="danger"
                :disabled="!row.uninstallable"
                :loading="store.applying"
                @click="uninstall(row)"
              >
                {{ t('settings.agentSkills.uninstall') }}
              </el-button>
            </div>
          </li>
          <li v-if="!store.loading && !store.skills?.length" class="skill-cards__empty">
            {{ t('settings.agentSkills.empty') }}
          </li>
        </ul>
      </div>

      <!-- doctor -->
      <div v-if="store.doctor" class="section">
        <div class="section-title">{{ t('settings.agentSkills.doctor') }}</div>
        <div class="doctor">
          <div class="doctor-row">
            journal: needs_reconcile={{ store.doctor.journal?.needs_reconcile?.length ?? 0 }},
            interrupted={{ store.doctor.journal?.interrupted?.length ?? 0 }}
          </div>
          <div class="doctor-row">
            skills: managed={{ store.doctor.skills?.managed ?? 0 }},
            discovered={{ store.doctor.skills?.discovered ?? 0 }},
            defined={{ store.doctor.skills?.defined ?? 0 }},
            drifted={{ store.doctor.skills?.drifted?.length ?? 0 }}
          </div>
          <div class="doctor-row">
            mcp: registered={{ store.doctor.mcp?.registered ?? 0 }},
            enabled={{ store.doctor.mcp?.desired_enabled ?? 0 }},
            drift={{ store.doctor.mcp?.drift?.length ?? 0 }}
          </div>
          <div class="doctor-row">
            {{ t('settings.agentSkills.findings') }}:
            {{ store.doctor.findings?.length ? store.doctor.findings.join(', ') : t('settings.agentSkills.none') }}
          </div>
        </div>
      </div>

      <!-- reconcile result -->
      <div v-if="store.lastReconcile" class="section">
        <div class="section-title">{{ t('settings.agentSkills.reconcile') }}</div>
        <div class="doctor">
          <div class="doctor-row">
            {{ t('settings.agentSkills.reconcileSummary', {
              quarantines: store.lastReconcile.quarantines_finalized?.length ?? 0,
              installs: store.lastReconcile.installs_recovered?.length ?? 0,
              mcp: store.lastReconcile.mcp_effects_recovered?.length ?? 0,
              staging: store.lastReconcile.staging_residue_removed ?? 0
            }) }}
          </div>
          <div class="doctor-row">
            {{ t('settings.agentSkills.stillNeedsReconcile') }}:
            {{ store.lastReconcile.still_needs_reconcile?.length
              ? store.lastReconcile.still_needs_reconcile.join(', ')
              : t('settings.agentSkills.none') }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { ElMessage } from 'element-plus';

import { mutationOutcomes, verdictAllowsInstall } from '@/libs/capability.js';
import { useCapabilityStore } from '@/stores/capability';

const { t } = useI18n();
const store = useCapabilityStore();

const sourceKind = ref('local_directory');
const sourcePath = ref('');
const githubRepository = ref('');
const githubPath = ref('');
const githubRef = ref('main');
const selection = ref([]);

const report = computed(() => store.checkReport);
const repositoryParts = computed(() => githubRepository.value.trim().split('/').filter(Boolean));

const canCheck = computed(() => {
  if (store.checking) return false;
  if (sourceKind.value === 'github') {
    return repositoryParts.value.length === 2 && githubPath.value.trim() !== '';
  }
  return sourcePath.value.trim() !== '';
});

const canInstall = computed(
  () => verdictAllowsInstall(report.value) && selection.value.length > 0 && !store.applying
);

const outcomes = computed(() => mutationOutcomes(store.lastMutation));

/** The strict source document the backend accepts; empty fields are omitted. */
const sourceDocument = computed(() => {
  if (sourceKind.value === 'github') {
    return {
      kind: 'github',
      owner: repositoryParts.value[0],
      repo: repositoryParts.value[1],
      path: githubPath.value.trim() || undefined,
      git_ref: githubRef.value.trim() || undefined
    };
  }
  return { kind: sourceKind.value, path: sourcePath.value.trim() };
});

// A changed source must not keep an old verdict around: the check is the gate.
watch([sourceKind, sourcePath, githubRepository, githubPath, githubRef], () => {
  store.resetCheck();
});

const verdictTagType = computed(() => {
  switch (report.value?.verdict) {
    case 'pass':
      return 'success';
    case 'blocked':
      return 'danger';
    default:
      return 'warning';
  }
});

function statusTagType(status) {
  switch (status) {
    case 'installed':
    case 'removed':
    case 'finalized':
      return 'success';
    case 'skipped_existing':
    case 'already_installed':
    case 'not_found':
      return 'info';
    case 'unsupported':
    case 'blocked':
    case 'refused':
    case 'failed':
      return 'warning';
    default:
      return '';
  }
}

async function rescan() {
  try {
    await store.loadInventory();
  } catch (error) {
    ElMessage.error(error.message);
  }
}

async function runDoctor() {
  try {
    await store.loadDoctor();
  } catch (error) {
    ElMessage.error(error.message);
  }
}

async function runReconcile() {
  try {
    await store.reconcile();
    ElMessage.success(t('settings.agentSkills.reconcileDone'));
  } catch (error) {
    ElMessage.error(error.message);
  }
}

async function check() {
  try {
    await store.checkSource(sourceDocument.value);
  } catch (error) {
    ElMessage.error(error.message);
  }
}

async function install() {
  try {
    await store.install({
      source: sourceDocument.value,
      targets: [...selection.value]
    });
    ElMessage.success(t('settings.agentSkills.installDone'));
  } catch (error) {
    // A blocked verdict and an unsupported target are legitimate results, so
    // the message is shown but the page keeps the per-target outcome list.
    ElMessage.error(error.message);
  }
}

async function uninstall(row) {
  try {
    await store.uninstall({
      skillName: row.name,
      targets: row.target_id ? [row.target_id] : []
    });
    ElMessage.success(t('settings.agentSkills.uninstallDone'));
  } catch (error) {
    ElMessage.error(error.message);
  }
}

onMounted(async () => {
  await rescan();
  // Selected by default: the ChatSpeed directory only, never an external tool.
  selection.value = [...store.defaultSelection];
});
</script>

<style lang="scss" scoped>
.agent-skills {
  // The page adapts to the width it is actually given, which is the dock width, not the
  // window viewport: a named inline-size container is what the `@container` rules below read.
  container-name: agent-skills;
  container-type: inline-size;
  display: flex;
  flex-direction: column;
  gap: var(--cs-space);

  .card {
    background: var(--cs-bg-color);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    padding: var(--cs-space);
    min-width: 0;
  }

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--cs-space-sm);
    font-weight: 600;
    margin-bottom: var(--cs-space);
    min-width: 0;

    .actions {
      display: flex;
      gap: var(--cs-space-sm);
    }

    .icon {
      cursor: pointer;
      color: var(--cs-text-color-secondary);

      &.disabled {
        cursor: not-allowed;
        opacity: 0.5;
      }
    }
  }

  .hint {
    margin: 0 0 var(--cs-space);
    color: var(--cs-text-color-secondary);
    font-size: 12px;
  }

  .section {
    margin-bottom: var(--cs-space);
    min-width: 0;

    .section-title {
      font-weight: 600;
      margin-bottom: var(--cs-space-sm);
    }
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-sm);
    margin-top: var(--cs-space-sm);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--cs-space-sm);
  }

  .alert {
    margin-top: var(--cs-space-sm);
  }

  .report {
    margin-top: var(--cs-space);
  }

  .report-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--cs-space-sm);
    margin-bottom: var(--cs-space-sm);

    .meta {
      color: var(--cs-text-color-secondary);
      font-size: 12px;
      min-width: 0;
      overflow-wrap: anywhere;
    }
  }

  .permissions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--cs-space-xs);
    margin-bottom: var(--cs-space-sm);
  }

  .findings {
    margin-bottom: var(--cs-space-sm);
  }

  .external {
    color: var(--cs-text-color-secondary);
    font-size: 11px;
    margin-left: var(--cs-space-xs);
  }

  .doctor {
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-xs);

    .doctor-row {
      overflow-wrap: anywhere;
    }
  }

  /*
   * Responsive projection of the three wide tables. Above the breakpoint every table keeps
   * its desktop Element Plus layout; below it the table is replaced by a data-complete card
   * list, so a narrow dock shows the same columns instead of a clipped or scrolling table.
   */
  .skill-cards {
    display: none;
    flex-direction: column;
    gap: var(--cs-space-sm);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .skill-cards__item {
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-xs);
    min-width: 0;
    padding: var(--cs-space-sm);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius);
    background: var(--cs-bg-color-deep);
  }

  .skill-cards__row {
    display: flex;
    align-items: flex-start;
    gap: var(--cs-space-sm);
    min-width: 0;
  }

  .skill-cards__label {
    flex: 0 0 auto;
    width: 6em;
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-sm);
  }

  .skill-cards__value {
    flex: 1 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .skill-cards__tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--cs-space-xs);
  }

  .skill-cards__actions {
    display: flex;
    justify-content: flex-end;
  }

  .skill-cards__empty {
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-sm);
  }

  /*
   * A 600px dock cannot fit the ~800px installed table, so the compact breakpoint starts
   * below the widest table instead of at a viewport size. The narrow breakpoint then stacks
   * the header and the form controls so nothing pushes the page sideways.
   */
  @container agent-skills (max-width: 860px) {
    .skill-table {
      display: none;
    }

    .skill-cards {
      display: flex;
    }
  }

  @container agent-skills (max-width: 480px) {
    .title {
      align-items: flex-start;
      flex-direction: column;
    }

    .report-head {
      align-items: flex-start;
      flex-direction: column;
      gap: var(--cs-space-xs);
    }

    .skill-cards__row {
      flex-direction: column;
      gap: var(--cs-space-xxs);
    }

    .skill-cards__label {
      width: auto;
      font-weight: 600;
    }

    .buttons :deep(.el-button) {
      flex: 1 1 auto;
    }

    :deep(.el-radio-group) {
      display: flex;
      flex-wrap: wrap;
      max-width: 100%;
    }

    :deep(.el-checkbox-group) {
      display: flex;
      flex-direction: column;
      align-items: flex-start;
      gap: var(--cs-space-xs);
    }

    :deep(.el-checkbox) {
      height: auto;
      margin-right: 0;
      white-space: normal;
    }
  }
}
</style>
