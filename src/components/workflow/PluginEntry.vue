<template>
  <div class="plugin-entry">
    <el-dropdown trigger="click" placement="right" @command="onCommand">
      <button class="plugin-entry__button" :class="{ active: !!activePlugin }" type="button"
        :aria-label="$t('workflow.plugin.title')">
        <el-tooltip :content="$t('workflow.plugin.title')" placement="right" :hide-after="0" :enterable="false">
          <cs name="function" size="var(--cs-font-size-lg)" />
        </el-tooltip>
      </button>
      <template #dropdown>
        <el-dropdown-menu class="plugin-entry__menu">
          <el-dropdown-item v-if="plugins.length === 0" disabled>
            {{ $t('workflow.plugin.empty') }}
          </el-dropdown-item>
          <el-dropdown-item
            v-for="plugin in plugins"
            :key="plugin.id"
            :command="plugin.id"
            :class="{ active: plugin.id === activePluginId }">
            <cs name="function" size="16px" />
            <span>{{ plugin.id }}</span>
            <cs v-if="plugin.id === activePluginId" name="check" size="14px" />
          </el-dropdown-item>
          <el-dropdown-item v-if="activePluginId" divided :command="CLOSE_COMMAND">
            <cs name="close" size="16px" />
            <span>{{ $t('workflow.plugin.close') }}</span>
          </el-dropdown-item>
        </el-dropdown-menu>
      </template>
    </el-dropdown>
    <div v-if="activePlugin" class="plugin-entry__current">
      <el-tooltip :content="activePlugin.id" placement="right" :hide-after="0" :enterable="false">
        <button class="plugin-entry__current-button" type="button" :aria-label="activePlugin.id" @click="emit('toggle')">
          <cs name="function" size="16px" />
        </button>
      </el-tooltip>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  plugins: { type: Array, default: () => [] },
  activePluginId: { type: String, default: '' }
})

const emit = defineEmits(['select', 'close', 'toggle'])
const CLOSE_COMMAND = 'close'
const activePlugin = computed(() => props.plugins.find(plugin => plugin.id === props.activePluginId) || null)

const onCommand = command => {
  if (command === CLOSE_COMMAND) {
    emit('close')
    return
  }
  const plugin = props.plugins.find(item => item.id === command)
  if (plugin) emit('select', plugin)
}
</script>

<style lang="scss" scoped>
.plugin-entry {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
}

.plugin-entry__button,
.plugin-entry__current-button {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: 0;
  border-radius: var(--cs-border-radius);
  color: var(--cs-text-color-secondary);
  background: transparent;
  cursor: pointer;

  &:hover,
  &.active {
    color: var(--el-color-primary);
    background: var(--cs-hover-bg-color);
  }
}

.plugin-entry__current {
  width: 36px;
  height: 36px;
}

.plugin-entry__menu {
  .el-dropdown-menu__item {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);

    &.active { color: var(--el-color-primary); }
  }
}
</style>
