<template>
  <transition
    name="skill-list"
    @before-enter="onBeforeEnter"
    @enter="onEnter"
    @before-leave="onBeforeLeave"
    @leave="onLeave">
    <div class="skill-select-list" v-show="isVisible" ref="listRef">
      <div class="title">
        <span class="text">{{ t('settings.skill.selectSkill') }}</span>
        <span class="icons" @click="isVisible = false"><cs name="delete" /></span>
      </div>
      <div class="list">
        <SkillItem
          class="skill-item"
          v-for="(skill, index) in filteredSkills"
          :key="skill.id"
          :skill="skill"
          :class="{ active: selectedId === index }"
          @click="onSelected(index)" />
      </div>
    </div>
  </transition>
</template>
<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'

import { useSkillStore } from '@/stores/skill'
import SkillItem from './skillItem.vue'

const { t } = useI18n()
const skillStore = useSkillStore()
const { skills } = storeToRefs(skillStore)

const emit = defineEmits(['onSelected', 'visibleChanged'])
const props = defineProps({
  searchKw: {
    type: String,
    default: ''
  }
})

const selectedId = ref(0)
const isVisible = ref(false)
const listRef = ref(null)
const filteredSkills = computed(() => {
  if (!props.searchKw) return skills.value
  return skills.value.filter(skill =>
    skill.name.toLowerCase().includes(props.searchKw.toLowerCase())
  )
})

watch(
  () => isVisible.value,
  () => {
    emit('visibleChanged', isVisible.value)
  }
)

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
})

/**
 * handle keydown event
 * @param {KeyboardEvent} e
 */
const onKeydown = e => {
  // if skill list is not visible, do nothing
  if (!isVisible.value) return
  if (e.key === 'Escape') {
    e.preventDefault()
    e.stopPropagation()
    isVisible.value = false
  } else if (e.key === 'Enter') {
    e.preventDefault()
    e.stopPropagation()
    onSelected(selectedId.value)
  } else if (e.key === 'ArrowDown') {
    e.preventDefault()
    e.stopPropagation()
    selectedId.value++
    if (selectedId.value >= filteredSkills.value.length) {
      selectedId.value = 0
    }
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    e.stopPropagation()
    selectedId.value--
    if (selectedId.value < 0) {
      selectedId.value = filteredSkills.value.length - 1
    }
  }
}

/**
 * handle skill selected
 * @param {number} id selected skill id
 */
const onSelected = index => {
  selectedId.value = index
  const skill = filteredSkills.value[index]
  isVisible.value = false
  emit('onSelected', skill)
}

// =================================================
// Animation handlers
// =================================================

const onBeforeEnter = el => {
  el.style.opacity = '0'
}

const onEnter = el => {
  el.style.opacity = '1'
}

const onBeforeLeave = el => {
  el.style.opacity = '1'
}

const onLeave = el => {
  el.style.opacity = '0'
}

// =================================================
// expose
// =================================================
const show = () => {
  isVisible.value = true
}
const hide = () => {
  isVisible.value = false
}
defineExpose({
  show,
  hide,
  isVisible
})
</script>

<style lang="scss" scoped>
.skill-select-list {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-xs);
  width: 100%;
  background-color: var(--cs-input-bg-color);
  border-radius: var(--cs-border-radius-md);
  padding: var(--cs-space-sm);
  margin-bottom: var(--cs-space-xs);
  box-sizing: border-box;
  transition: height 0.3s ease, opacity 0.3s ease;
  overflow: hidden;

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--cs-space-xxs) var(--cs-space-sm);

    .text {
      font-size: var(--cs-font-size);
      font-weight: 500;
    }

    .icons {
      cursor: pointer;
      padding: var(--cs-space-xxs) var(--cs-space-xs);

      &:hover {
        background-color: var(--cs-bg-color-light);
        border-radius: var(--cs-border-radius-sm);
      }
    }
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--cs-space-xxs);
    max-height: 250px;
    overflow-y: auto;

    .skill-item {
      cursor: pointer;
      padding: var(--cs-space-xs) var(--cs-space-sm);

      &:hover,
      &.active {
        background-color: var(--cs-bg-color-deep);
        color: var(--cs-text-color-primary) !important;
      }
    }
  }
}

.skill-list-enter-active,
.skill-list-leave-active {
  transition: opacity 0.3s ease;
}

.skill-list-enter-from,
.skill-list-leave-to {
  opacity: 0;
}
</style>
