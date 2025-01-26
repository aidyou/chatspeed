<template>
  <div class="item" :class="{ active: active }">
    <div class="img" v-if="skill.logo">
      <img :src="skill.logo" class="skill-logo" />
    </div>
    <div class="icon" v-else>
      <cs :name="skill.icon" />
    </div>
    <div class="name">{{ skill.name }}</div>
    <div class="desc">{{ skill?.metadata?.description || '' }}</div>
  </div>
</template>
<script setup>
const props = defineProps({
  skill: {
    type: Object,
    required: true,
  },
  active: {
    type: Boolean,
    default: false,
  },
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

  .img,
  .icon {
    width: 24px;
    height: 24px;
    border-radius: var(--cs-border-radius-round);
    box-sizing: border-box;
    background-color: var(--cs-bg-color);
    display: flex;
    align-items: center;
    justify-content: center;

    img {
      width: 100%;
      height: 100%;
      object-fit: cover;
      border-radius: var(--cs-border-radius-round);
    }
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
