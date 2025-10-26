<template>
  <div class="todo-list-container" v-if="items.length > 0">
    <div class="title" :class="{ collapsed: !todoListShow }" @click="todoListShow = !todoListShow">
      {{ t('workflow.todoList.title') }}
    </div>
    <ul v-show="todoListShow">
      <li v-for="(item, index) in items" :key="item.id || index" :class="item.status">
        <cs :name="item.status" />
        {{ item.title }}
      </li>
    </ul>
  </div>
</template>

<script setup>
import { useI18n } from 'vue-i18n'
import { ref } from 'vue'

const { t } = useI18n()

const todoListShow = ref(true)

const props = defineProps({
  items: {
    type: Array,
    default: () => []
  }
})
</script>

<style lang="scss" scoped>
.todo-list-container {
  padding: var(--cs-space-sm) var(--cs-space-md);

  .title {
    font-size: var(--cs-font-size-md);
    font-weight: bold;
    color: var(--cs-text-color-primary);
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    margin-bottom: var(--cs-space);

    &::after {
      font-family: 'chatspeed';
      content: '\e642';
      display: block;
      font-size: var(--cs-font-size-xs);
      transform: rotate(180deg);
      transition: transform 0.3s ease;
    }

    &.collapsed {
      &::after {
        transform: rotate(0deg);
      }
    }
  }
}
ul {
  list-style: none;
  padding: 0;
  margin: 0;

  li {
    margin-bottom: var(--cs-space-xs);
    font-size: var(--cs-font-size);
    color: var(--cs-text-color-secondary);

    &.running {
      color: var(--cs-warning-color);
    }

    &.completed {
      color: var(--cs-text-color-primary);

      .cs {
        color: var(--cs-success-color);
      }
    }

    .completed {
      text-decoration: line-through;
      color: var(--cs-text-color-secondary);
    }
  }
}
</style>
