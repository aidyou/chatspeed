<template>
  <div class="todo-list-container" v-if="items.length > 0">
    <div class="header-info" :class="{ collapsed: !todoListShow }" @click="todoListShow = !todoListShow">
      <div class="current-task">
        <span class="dot">●</span>
        <span class="task-text">{{ currentTask?.subject || currentTask?.title || t('workflow.todoList.title') }}</span>
      </div>
      <div class="next-task" v-if="nextTask">
        <span class="elbow">⎿</span>
        <span class="next-label">{{ t('workflow.todoList.next') }}:</span>
        <span class="task-text">{{ nextTask.subject || nextTask.title }}</span>
      </div>
    </div>
    <ul v-show="todoListShow" class="full-list">
      <li v-for="(item, index) in items" :key="item.id || index" :class="item.status">
        <cs :name="getStatusIcon(item.status)" :class="{ 'cs-spin': item.status === 'in_progress' }" />
        <span :class="{ 'text-completed': item.status === 'completed' || item.status === 'failed' || item.status === 'data_missing' }">
          {{ item.subject || item.title }}
        </span>
      </li>
    </ul>
  </div>
</template>

<script setup>
import { useI18n } from 'vue-i18n'
import { ref, computed } from 'vue'

const { t } = useI18n()

const todoListShow = ref(false)

const props = defineProps({
  items: {
    type: Array,
    default: () => []
  }
})

const currentTask = computed(() => {
  const inProgress = props.items.find(item => item.status === 'in_progress')
  if (inProgress) return inProgress
  return props.items.find(item => item.status !== 'completed') || props.items[0]
})

const nextTask = computed(() => {
  if (!currentTask.value) return null
  const currentIndex = props.items.findIndex(item => item === currentTask.value)
  return props.items.slice(currentIndex + 1).find(item => item.status === 'pending')
})

const getStatusIcon = (status) => {
  switch (status) {
    case 'completed': return 'check'
    case 'in_progress': return 'loading'
    case 'failed': return 'error'
    case 'data_missing': return 'warning'
    default: return 'pending'
  }
}
</script>

<style lang="scss" scoped>
.todo-list-container {
  padding: var(--cs-space-xs) var(--cs-space-md);
  border-left: 2px solid var(--el-color-primary-light-7);
  background-color: var(--el-color-primary-light-9);
  border-radius: 4px;
  margin: 10px 0;

  .header-info {
    cursor: pointer;
    user-select: none;
    padding: 4px 0;

    .current-task {
      display: flex;
      align-items: center;
      gap: 8px;
      font-weight: 600;
      color: var(--cs-text-color-primary);
      font-size: var(--cs-font-size-sm);

      .dot {
        color: var(--el-color-primary);
      }
    }

    .next-task {
      display: flex;
      align-items: center;
      gap: 4px;
      margin-left: 4px;
      margin-top: 2px;
      font-size: var(--cs-font-size-xs);
      color: var(--cs-text-color-secondary);

      .elbow {
        font-family: monospace;
        margin-right: 4px;
      }
      
      .next-label {
        font-weight: bold;
        margin-right: 4px;
      }
    }

    &::after {
      content: '';
      position: absolute;
      right: var(--cs-space-md);
      top: 50%;
      // Use a custom icon or simple chevron
    }
  }
}

.full-list {
  list-style: none;
  padding: 10px 0 5px 12px;
  margin: 0;
  border-top: 1px solid var(--el-color-primary-light-8);
  margin-top: 8px;

  li {
    margin-bottom: 6px;
    font-size: var(--cs-font-size-xs);
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--cs-text-color-secondary);

    &.in_progress {
      color: var(--el-color-primary);
      font-weight: 500;
    }

    &.completed {
      .cs {
        color: var(--el-color-success);
      }
    }

    &.failed {
      color: var(--el-color-danger);
      .cs {
        color: var(--el-color-danger);
      }
    }

    &.data_missing {
      color: var(--el-color-warning);
      .cs {
        color: var(--el-color-warning);
      }
    }

    .text-completed {
      text-decoration: line-through;
      opacity: 0.6;
    }
  }
}
</style>
