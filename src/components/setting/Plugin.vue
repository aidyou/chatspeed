<template>
  <div class="plugin">
    <div v-loading="pluginStore.loading" class="card">
      <div class="title">
        <span>{{ t('settings.plugin.title') }}</span>
        <el-tooltip :content="t('settings.plugin.refresh')" placement="left" :hide-after="0">
          <span class="icon" :class="{ disabled: pluginStore.loading }" @click="refresh">
            <cs name="refresh" />
          </span>
        </el-tooltip>
      </div>

      <p class="hint">{{ t('settings.plugin.hint') }}</p>

      <el-alert
        v-if="pluginStore.lastError"
        class="alert"
        type="error"
        :closable="false"
        :title="t('settings.plugin.operationFailed')" />

      <!-- Only the runtime's built-in inventory collection is rendered; the
           desktop never guesses a plugin the runtime did not report. -->
      <template v-if="pluginStore.builtinPlugins.length">
        <div class="summary">
          <span>{{ t('settings.plugin.builtinCount', { count: pluginStore.builtinPlugins.length }) }}</span>
          <span>{{ t('settings.plugin.verifiedUiCount', { count: pluginStore.verifiedUiPlugins.length }) }}</span>
        </div>

        <div v-for="plugin in pluginStore.builtinPlugins" :key="plugin.id" class="plugin-item">
          <div class="plugin-head">
            <span class="plugin-name">{{ plugin.id }}</span>
            <el-tag size="small" effect="plain">{{ t(`settings.plugin.kind.${plugin.kind}`) }}</el-tag>
            <el-tag size="small" :type="stateTagType(plugin.state)">
              {{ t(`settings.plugin.state.${plugin.state}`) }}
            </el-tag>
          </div>

          <div class="plugin-meta">
            <span>{{ t('settings.plugin.version') }}: {{ plugin.version || '-' }}</span>
            <span>{{ t('settings.plugin.capabilities') }}: {{ plugin.capabilities?.join(', ') || '-' }}</span>
            <span class="root">{{ plugin.root }}</span>
          </div>

          <div v-if="plugin.ui" class="plugin-ui">
            <el-tag size="small" :type="plugin.ui.verified ? 'success' : 'danger'" effect="plain">
              {{ plugin.ui.verified ? t('settings.plugin.uiVerified') : t('settings.plugin.uiUnverified') }}
            </el-tag>
            <span class="meta">{{ t('settings.plugin.uiEntry') }}: {{ plugin.ui.entry }}</span>
          </div>

          <div class="buttons">
            <el-button
              v-if="plugin.state === 'not_installed'"
              type="primary"
              size="small"
              :loading="pluginStore.applying"
              @click="install">
              {{ t('settings.plugin.install') }}
            </el-button>
            <el-button
              v-else-if="plugin.state === 'enabled'"
              size="small"
              :loading="pluginStore.applying"
              @click="disable">
              {{ t('settings.plugin.disable') }}
            </el-button>
            <el-button
              v-else
              type="primary"
              size="small"
              :loading="pluginStore.applying"
              @click="enable">
              {{ t('settings.plugin.enable') }}
            </el-button>
            <el-button
              v-if="plugin.state !== 'not_installed'"
              type="danger"
              plain
              size="small"
              :loading="pluginStore.applying"
              @click="uninstall">
              {{ t('settings.plugin.uninstall') }}
            </el-button>
          </div>
        </div>
      </template>

      <div v-else class="empty">{{ t('settings.plugin.empty') }}</div>
    </div>
  </div>
</template>

<script setup>
import { onMounted } from 'vue';
import { useI18n } from 'vue-i18n';

import { usePluginStore } from '@/stores/plugin';

const { t } = useI18n();
const pluginStore = usePluginStore();

function stateTagType(state) {
  switch (state) {
    case 'enabled':
      return 'success';
    case 'disabled':
      return 'warning';
    default:
      return 'info';
  }
}

// The store owns the message; the inline alert is the single place it is shown.
async function run(action) {
  try {
    await action();
  } catch {
    // Handled by the store's lastError alert.
  }
}

function refresh() {
  return run(() => pluginStore.loadInventory());
}

function install() {
  return run(() => pluginStore.install());
}

function enable() {
  return run(() => pluginStore.enable());
}

function disable() {
  return run(() => pluginStore.disable());
}

function uninstall() {
  return run(() => pluginStore.uninstall());
}

onMounted(refresh);
</script>

<style lang="scss" scoped>
.plugin {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space);

  .card {
    background: var(--cs-bg-color);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    padding: var(--cs-space);
  }

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-weight: 600;
    margin-bottom: var(--cs-space);

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

  .alert {
    margin-bottom: var(--cs-space);
  }

  .summary {
    display: flex;
    gap: var(--cs-space);
    margin-bottom: var(--cs-space-sm);
    color: var(--cs-text-color-secondary);
    font-size: 12px;
  }

  .plugin-item {
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius);
    padding: var(--cs-space-sm);
    margin-bottom: var(--cs-space-sm);
  }

  .plugin-head {
    display: flex;
    align-items: center;
    gap: var(--cs-space-sm);
    margin-bottom: var(--cs-space-xs);

    .plugin-name {
      font-weight: 600;
    }
  }

  .plugin-meta {
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-xs);
    margin-bottom: var(--cs-space-sm);
    color: var(--cs-text-color-secondary);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .plugin-ui {
    display: flex;
    align-items: center;
    gap: var(--cs-space-sm);
    margin-bottom: var(--cs-space-sm);

    .meta {
      color: var(--cs-text-color-secondary);
      font-size: 12px;
    }
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--cs-space-sm);
  }

  .empty {
    color: var(--cs-text-color-secondary);
    font-size: 12px;
  }
}
</style>