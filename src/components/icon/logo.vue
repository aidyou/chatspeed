<template>
  <svg
    :class="`cs icon-logo icon-${name}`"
    :width="size"
    :height="size"
    :fill="color"
    aria-hidden="true">
    <use :href="`#cs-logo-${name}`" />
  </svg>
</template>

<script setup>
const props = defineProps({
  name: {
    type: String,
    required: true,
  },
  size: {
    type: [String, Number],
    default: '24',
  },
  color: {
    type: String,
    default: 'currentColor',
  },
})
</script>

<style scoped>
.cs {
  display: inline-block;
  vertical-align: middle;
  transition: transform 0.2s;
}

.icon-logo:hover {
  transform: scale(1.05);
}
</style>
