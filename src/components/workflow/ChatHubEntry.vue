<template>
  <div class="chat-hub-entry">
    <!-- The icon is the entry list and nothing else: the page is shown and hidden by the entry
         of the site that is currently docked, below this list. -->
    <el-dropdown
      trigger="click"
      placement="right"
      @command="onSelectCommand">
      <div class="chat-hub-entry__button" :class="{ active: !!activeHub }">
        <el-tooltip :content="$t('workflow.chatHub.title')" placement="right" :hide-after="0" :enterable="false">
          <cs name="connected" size="var(--cs-font-size-lg)" />
        </el-tooltip>
      </div>
      <template #dropdown>
        <el-dropdown-menu class="chat-hub-entry__menu">
          <el-dropdown-item v-if="hubs.length === 0" disabled>
            {{ $t('workflow.chatHub.empty') }}
          </el-dropdown-item>
          <el-dropdown-item
            v-for="hub in hubs"
            :key="hub.id"
            :command="hub.id"
            :class="{ active: hub.id === activeHubId }">
            <img
              v-if="logoOf(hub)"
              :src="logoOf(hub)"
              class="chat-hub-entry__logo"
              @error="markLogoBroken(hub)" />
            <avatar v-else :text="hub.name" :size="16" />
            <span class="chat-hub-entry__name">{{ hub.name }}</span>
          </el-dropdown-item>
          <!-- The page is docked next to the workflow UI, so this list is the only place
               that can offer an explicit close action. -->
          <el-dropdown-item
            v-if="activeHubId"
            divided
            :command="CLOSE_COMMAND"
            class="chat-hub-entry__close">
            <cs name="close" size="16px" color="secondary" />
            <span class="chat-hub-entry__name">{{ $t('workflow.chatHub.close') }}</span>
          </el-dropdown-item>
        </el-dropdown-menu>
      </template>
    </el-dropdown>

    <div v-if="activeHub" class="chat-hub-entry__current">
      <el-tooltip :content="activeHub.name" placement="right" :hide-after="0" :enterable="false">
        <div class="chat-hub-entry__current-surface" @click="emit('toggle')">
          <img
            v-if="logoOf(activeHub)"
            :src="logoOf(activeHub)"
            class="chat-hub-entry__logo"
            @error="markLogoBroken(activeHub)" />
          <avatar v-else :text="activeHub.name" :size="16" />
        </div>
      </el-tooltip>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue'

/**
 * Workflow sidebar entry for the ChatHub web chat sites.
 *
 * It is a pure view control: it renders the entry list and reports the selected
 * entry, and it never touches workflow state. Logos fall back to the shared
 * letter avatar when the entry has no logo or the remote image fails to load.
 */
const props = defineProps({
  hubs: {
    type: Array,
    default: () => []
  },
  activeHubId: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits(['select', 'close', 'toggle'])

/** Menu command that closes the docked page instead of selecting an entry. */
const CLOSE_COMMAND = 'close'

const brokenLogoIds = ref(new Set())

const activeHub = computed(() => props.hubs.find(hub => hub.id === props.activeHubId) || null)

const logoOf = hub => (hub.logo && !brokenLogoIds.value.has(hub.id) ? hub.logo : '')

const markLogoBroken = hub => {
  brokenLogoIds.value.add(hub.id)
}

const onSelectCommand = command => {
  if (command === CLOSE_COMMAND) {
    emit('close')
    return
  }

  const hub = props.hubs.find(item => item.id === command)
  if (hub) {
    emit('select', hub)
  }
}
</script>

<style lang="scss">
.chat-hub-entry {
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  align-items: center;
  // gap: var(--cs-space-xs);

  .chat-hub-entry__button {
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    width: 36px;
    height: 36px;
    border-radius: var(--cs-border-radius);
    color: var(--cs-text-color-secondary);
    cursor: pointer;
    transition:
      background-color 0.2s ease,
      color 0.2s ease;

    &:hover {
      background-color: var(--cs-hover-bg-color);
      color: var(--cs-text-color-primary);
    }

    &.active {
      color: var(--el-color-primary);
    }
  }

  // The icon of the site that is currently docked, and the toggle that hides or shows it.
  .chat-hub-entry__current {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    position: relative;
  }

  .chat-hub-entry__current-surface {
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    width: 100%;
    height: 100%;
    border-radius: var(--cs-border-radius);
    cursor: pointer;
    transition: background-color 0.2s ease;

    &:hover {
      background-color: var(--cs-hover-bg-color);
    }
  }
}

.chat-hub-entry__logo {
  width: 20px;
  height: 20px;
  margin-right: var(--cs-space-xs);
  border-radius: var(--cs-border-radius-round);
  object-fit: contain;
}

.chat-hub-entry__current .chat-hub-entry__logo {
  margin-right: 0;
}

.chat-hub-entry__menu {
  max-height: 60vh;
  overflow-y: auto;

  .chat-hub-entry__name {
    margin-left: var(--cs-space-xs);
  }

  .el-dropdown-menu__item.active {
    color: var(--el-color-primary);
  }
}
</style>
