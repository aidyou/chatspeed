<template>
  <el-tabs v-model="activeTab" class="agent-management-tabs">
    <el-tab-pane :label="$t('settings.agent.title')" name="agents">
      <agent />
    </el-tab-pane>
    <el-tab-pane :label="$t('settings.sandbox.title')" name="sandbox">
      <sandbox />
    </el-tab-pane>
  </el-tabs>
</template>

<script setup>
import { ref } from 'vue'
import Agent from './Agent.vue'
import Sandbox from './Sandbox.vue'

const activeTab = ref('agents')
</script>

<style scoped lang="scss">
.agent-management-tabs {
  flex: 1;

  :deep(.el-tabs__content) {
    padding-top: var(--cs-space-sm);
  }
}
</style>
