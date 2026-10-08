<template>
  <aside v-show="visible" class="docked-views" :style="{ width: `${width}px` }">
    <div class="docked-views__header">
      <!-- The tab strip only exists once a second tab can be told apart from the first: a
           single tab needs no strip, but the refresh action stays reachable either way. -->
      <div v-if="tabs.length > 1" class="docked-views__tabs" role="tablist">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          class="docked-views__tab"
          :class="{ active: tab.id === activeTabId }">
          <button
            class="docked-views__tab-label"
            type="button"
            role="tab"
            :aria-selected="tab.id === activeTabId"
            :title="tab.title"
            @click="$emit('select', tab.id)">
            {{ tab.title }}
          </button>
          <!-- The close control is a sibling button, never nested inside the tab button, so
               both stay independently reachable by pointer and keyboard. -->
          <button
            class="docked-views__tab-close"
            type="button"
            :aria-label="$t('workflow.plugin.close')"
            @click="$emit('close', tab.id)">
            <cs name="close" size="12px" />
          </button>
        </div>
      </div>
      <div class="docked-views__toolbar">
        <button
          class="docked-views__action"
          type="button"
          :aria-label="$t('common.refresh')"
          :title="$t('common.refresh')"
          @click="$emit('reload')">
          <cs name="refresh" size="16px" />
        </button>
      </div>
    </div>

    <!-- The measured rectangle the native webview covers. Trusted content is rendered as
         host-native Vue instead, so it never needs a native view. -->
    <div ref="surfaceEl" class="docked-views__surface">
      <AgentSkills v-if="trustedActive" :key="trustedKey" />
    </div>
  </aside>
</template>

<script setup>
import { computed, ref } from 'vue'

import AgentSkills from '@/components/setting/AgentSkills.vue'

/**
 * The shared right dock.
 *
 * The dock is plain Vue: it lays out the tab strip, the toolbar and a surface placeholder.
 * A native tab is a second native webview the carrier paints over the measured surface, so
 * the dock itself never renders a chat site or a plugin page. The one exception is a
 * trusted plugin (the built-in agent-skills bundle), whose management UI is a trusted Vue
 * component and is rendered here directly instead of being given a native view.
 */
const props = defineProps({
  /** Whether the dock column is on screen. Hiding it keeps every tab alive. */
  visible: { type: Boolean, default: false },
  /** Dock width in logical pixels, owned by the shared width store. */
  width: { type: Number, required: true },
  /** Resolved tabs: `{ id, title, kind }`. */
  tabs: { type: Array, default: () => [] },
  /** The visible tab id, empty when the dock has no active tab. */
  activeTabId: { type: String, default: '' },
  /** Changes when the trusted tab has to remount, which is how that tab reloads. */
  trustedKey: { type: Number, default: 0 }
})

defineEmits(['select', 'close', 'reload'])

const surfaceEl = ref(null)

const trustedActive = computed(
  () => props.tabs.find(tab => tab.id === props.activeTabId)?.kind === 'trusted'
)

/** The element the native view must cover. The view layer measures it after the DOM update. */
defineExpose({ getSurfaceElement: () => surfaceEl.value })
</script>

<style lang="scss" scoped>
.docked-views {
  position: fixed;
  top: var(--cs-titlebar-height);
  right: 0;
  bottom: 0;
  z-index: 4;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  background: var(--cs-bg-color);
  border-left: 1px solid var(--cs-border-color);
  box-sizing: border-box;

  &__header {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    min-width: 0;
    gap: var(--cs-space-xs);
    padding: 0 var(--cs-space-xs);
    border-bottom: 1px solid var(--cs-border-color);
  }

  &__tabs {
    display: flex;
    align-items: center;
    flex: 1 1 auto;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    gap: var(--cs-space-xs);
  }

  &__tab {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
    max-width: 12rem;
    border-radius: var(--cs-border-radius);

    &.active {
      background: var(--cs-hover-bg-color);
    }
  }

  &__tab-label {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    border: 0;
    background: transparent;
    color: var(--cs-text-color-secondary);
    cursor: pointer;
    padding: var(--cs-space-xs) var(--cs-space-sm);
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--cs-font-size-sm);

    .docked-views__tab.active & {
      color: var(--el-color-primary);
    }

    &:focus-visible {
      outline: 2px solid var(--el-color-primary);
      outline-offset: -2px;
    }
  }

  &__tab-close,
  &__action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: var(--cs-border-radius);
    background: transparent;
    color: var(--cs-text-color-secondary);
    cursor: pointer;

    &:hover {
      background: var(--cs-bg-color-deep);
      color: var(--cs-text-color-primary);
    }

    &:focus-visible {
      outline: 2px solid var(--el-color-primary);
      outline-offset: -2px;
    }
  }

  &__toolbar {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
    margin-left: auto;
    padding: var(--cs-space-xs) 0;
  }

  &__surface {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }
}
</style>