<template>
  <div class="item" :class="{ active: active }">
    <div class="icon">
      <cs :name="skill.icon" v-if="skill.icon" />
      <avatar
        :text="skill.name"
        :size="24"
        bgColor="var(--cs-bg-color)"
        textColor="var(--cs-text-color-secondary)"
        v-else />
    </div>
    <div class="name">{{ skill.name }}</div>
    <div class="desc">{{ skill?.metadata?.description || '' }}</div>
  </div>
</template>
<script setup>
import avatar from '@/components/common/avatar.vue'
const props = defineProps({
  skill: {
    type: Object,
    required: true
  },
  active: {
    type: Boolean,
    default: false
  }
})
</script>
<style lang="scss" scoped>
.item {
  display: flex;
  align-items: center;
  flex-direction: row;
  gap: var(--cs-space-sm);
  padding: var(--cs-space-xs) var(--cs-space-xxs);
  border-radius: var(--cs-border-radius);
  color: var(--cs-text-color-secondary);

  .icon {
    width: 24px;
    height: 24px;
    border-radius: var(--cs-border-radius-round);
    box-sizing: border-box;
    background-color: var(--cs-bg-color);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .name {
    font-size: var(--cs-font-size);
  }
  img,
  icon,
  name {
    flex-shrink: 0;
  }
  .desc {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--cs-font-size-sm);
    color: var(--cs-text-color-secondary);
  }

  &.active {
    .name {
      color: var(--cs-color-primary);
    }

    .desc {
      color: var(--cs-color-primary-light);
    }
  }
}
</style>
