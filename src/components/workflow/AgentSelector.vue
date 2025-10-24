<template>
  <div class="agent-selector" :class="{ disabled: disabled }">
    <el-dropdown trigger="click" :disabled="disabled">
      <div class="el-dropdown-link">
        <cs name="agent" size="var(--cs-font-size-lg)" />
        <span class="agent-name">{{ selectedAgent?.name || 'Select Agent' }}</span>
        <cs name="caret-down" size="var(--cs-font-size-sm)" />
      </div>
      <template #dropdown>
        <el-dropdown-menu>
          <el-dropdown-item v-for="agent in agents" :key="agent.id" @click="selectAgent(agent)">
            <div class="agent-item">
              <div class="agent-info">
                <avatar :text="agent.name" :size="24" />
                {{ agent.name }}
              </div>
              <cs name="check" class="active" v-if="selectedAgent?.id === agent.id" />
            </div>
          </el-dropdown-item>
        </el-dropdown-menu>
      </template>
    </el-dropdown>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { useAgentStore } from '@/stores/agent'

const agentStore = useAgentStore()
const agents = computed(() => agentStore.agents)
const selectedAgent = ref(agents.value[0])

const emit = defineEmits(['update:modelValue'])
const props = defineProps({
  disabled: {
    type: Boolean,
    default: false
  }
})

const selectAgent = agent => {
  if (props.disabled) return
  selectedAgent.value = agent
  emit('update:modelValue', agent)
}

// Set initial value
if (agents.value.length > 0) {
  selectAgent(agents.value[0])
}
</script>

<style lang="scss" scoped>
.agent-selector {
  display: flex;
  align-items: center;
  cursor: pointer;

  .el-dropdown-link {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: var(--cs-space-xxs);
    color: var(--cs-color-primary);
  }

  &.disabled {
    cursor: not-allowed;
    opacity: 0.8;

    .el-dropdown-link {
      color: var(--cs-text-color-secondary);
    }
  }
}
.agent-item {
  display: flex;
  align-items: center;
  flex-direction: row;
  justify-content: space-between;
  width: 100%;
  gap: var(--cs-space-sm);

  .agent-info {
    display: flex;
    flex-direction: row;
    justify-content: flex-start;
    align-items: center;
    gap: var(--cs-space-xs);
  }
}
</style>
