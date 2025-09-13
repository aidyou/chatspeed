<template>
  <div class="model-selector-trigger" @click.stop="toggleVisible"
    :style="{ width: triggerSize + 'px', height: triggerSize + 'px' }">
    <el-tooltip :content="triggerTooltip || `${currentModel.name} / ${currentModel?.defaultModel}`" :hide-after="0"
      :enterable="false" placement="top">
      <template v-if="useProviderAvatar">
        <img :src="currentModel?.providerLogo" v-if="currentModel?.providerLogo !== ''" class="provider-avatar"
          :style="{ width: triggerSize + 'px', height: triggerSize + 'px' }" />
        <avatar :text="currentModel?.name" :size="triggerSize" v-else />
      </template>
      <cs :name="triggerIcon" v-else />
    </el-tooltip>
  </div>

  <!-- Model selection panel -->
  <div class="select-group upperLayer" v-if="visible" @click.stop
    :class="{ 'position-top': position === 'top', 'position-bottom': position === 'bottom' }">
    <div class="selector arrow" :class="{ 'arrow-top': position === 'top', 'arrow-bottom': position === 'bottom' }">
      <div class="selector-content">
        <div class="item" v-for="model in modelProviders" @click.stop="handleModelSelect(model)" :key="model.id"
          :class="{ active: currentModel.id === model.id }">
          <div class="name">
            <img :src="model.providerLogo" v-if="model.providerLogo !== ''" class="provider-logo" />
            <avatar :text="model.name" size="16" v-else />
            <span>{{ model.name }}</span>
          </div>
          <div class="icon" v-if="currentModel.id === model.id">
            <cs name="check" class="active" />
          </div>
        </div>
      </div>
    </div>
    <div class="selector">
      <div class="selector-content">
        <template v-for="(models, group) in currentSubModels" :key="group">
          <div class="item group" @click.stop>
            <div class="name">
              {{ group }}
            </div>
          </div>
          <div class="item" v-for="(model, index) in models" @click.stop="handleSubModelSelect(model)" :key="index"
            :class="{ active: currentModel?.defaultModel === model.id }">
            <div class="name">
              <span>{{ model.name || model.id.split('/').pop() }}</span>
            </div>
            <div class="icon" v-if="currentModel?.defaultModel === model.id">
              <cs name="check" class="active" />
            </div>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, toRefs } from 'vue'
import { useI18n } from 'vue-i18n'
import { useModelStore } from '@/stores/model'

const { t } = useI18n()
const modelStore = useModelStore()

// Props
const props = defineProps({
  // Current model object. If not provided, use the default model from the store.
  modelValue: {
    type: Object,
    default: null
  },
  // Panel position: 'top' for popping down (Assistant.vue), 'bottom' for popping up (Index.vue).
  position: {
    type: String,
    default: 'bottom',
    validator: (value) => ['top', 'bottom'].includes(value)
  },
  // Trigger button icon.
  triggerIcon: {
    type: String,
    default: 'menu'
  },
  // Trigger button tooltip text.
  triggerTooltip: {
    type: String,
    default: ''
  },
  // Trigger button size.
  triggerSize: {
    type: [String, Number],
    default: 24
  },
  // Whether to use the provider's avatar as the trigger button.
  useProviderAvatar: {
    type: Boolean,
    default: false
  }
})

// Emits
const emit = defineEmits(['update:modelValue', 'model-select', 'sub-model-select', 'selection-complete'])

const visible = ref(false)

// computed
const modelProviders = computed(() => modelStore.getAvailableProviders)
const currentModel = computed(() => props.modelValue || modelStore.defaultModelProvider)
const currentSubModels = computed(() =>
  currentModel.value?.models?.reduce((groups, x) => {
    if (!x.group) {
      x.group = t('settings.model.ungrouped')
    }
    groups[x.group] = groups[x.group] || []
    groups[x.group].push(x)
    return groups
  }, {})
)

// Watch for model providers changes to ensure reactivity
watch(() => modelStore.providers, (newProviders, oldProviders) => {
  // Force reactivity update when providers change
  // This ensures the component updates when models are added/deleted from backend
  console.debug('ModelSelector: providers changed', newProviders?.length, oldProviders?.length)
}, { deep: true })

// Also watch the length to catch array changes
watch(() => modelStore.providers.length, (newLength, oldLength) => {
  console.debug('ModelSelector: providers length changed', newLength, oldLength)
})

// Watch for default model changes to ensure current selection is valid
watch(() => modelStore.defaultModelProvider, (newDefault) => {
  // Check if current model is still available
  if (!props.modelValue && newDefault) {
    console.debug('ModelSelector: default model updated', newDefault)
  }
}, { deep: true })

const toggleVisible = () => {
  visible.value = !visible.value
}

const handleModelSelect = (model) => {
  emit('model-select', model)
  if (!props.modelValue) {
    // If modelValue is not passed in, update the store
    modelStore.setDefaultModelProvider(model)
  } else {
    // If modelValue is passed in, update via v-model
    emit('update:modelValue', { ...model })
  }
}

const handleSubModelSelect = (model) => {
  const modelId = model.id
  emit('sub-model-select', model, modelId)

  if (!props.modelValue) {
    // If modelValue is not passed in, update the store (for Index.vue)
    currentModel.value.defaultModel = modelId
    modelStore.setDefaultModelProvider(currentModel.value)

    // Update database records
    modelStore.setModelProvider({
      ...currentModel.value,
      defaultModel: modelId,
      metadata: {
        ...currentModel.value.metadata,
        proxyType: currentModel.value?.metadata?.proxyType || 'bySetting',
        logo: currentModel.value?.metadata?.logo || ''
      }
    }).catch(error => {
      console.error(error)
    })
  } else {
    // If modelValue is passed in, update via v-model (for Assistant.vue)
    const updatedModel = { ...currentModel.value, defaultModel: modelId }
    emit('update:modelValue', updatedModel)
  }

  // Hide the panel after selection is complete
  visible.value = false
  emit('selection-complete', model, modelId)
}

// Hide the panel when clicking outside
const handleClickOutside = (event) => {
  if (visible.value && !event.target.closest('.model-selector-trigger') && !event.target.closest('.select-group')) {
    visible.value = false
  }
}

// Hide the panel when pressing the ESC key
const handleKeyDown = (event) => {
  if (event.key === 'Escape' && visible.value) {
    visible.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  document.addEventListener('keydown', handleKeyDown)
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
  document.removeEventListener('keydown', handleKeyDown)
})
</script>

<style lang="scss" scoped>
.model-selector-trigger {
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--cs-border-radius);
  position: relative;

  .provider-avatar {
    border-radius: var(--cs-border-radius-round);
    object-fit: cover;
  }
}

.select-group {
  position: absolute;
  left: 0;
  display: flex;
  flex-direction: row;
  gap: var(--cs-space-xxs);
  z-index: 1000;

  &.position-bottom {
    bottom: 55px;
    left: 10px;
  }

  &.position-top {
    top: 22px;
    left: -8px;
  }

  .selector {
    position: relative;
    display: flex;
    flex-direction: column;
    background-color: var(--cs-bg-color);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    box-shadow: 0 2px 12px 0 var(--cs-shadow-color);
    transform: translateZ(0);
    -webkit-transform: translateZ(0);

    .selector-content {
      max-width: 230px;
      min-width: 150px;
      max-height: 250px;
      overflow: auto;
      padding: var(--cs-space-xs);
    }

    &.arrow-bottom {
      &::before {
        content: '';
        position: absolute;
        bottom: -9px;
        left: calc(var(--cs-space-lg));
        border-width: 9px 9px 0;
        border-style: solid;
        border-color: var(--cs-border-color) transparent transparent transparent;
        pointer-events: none;
      }

      &::after {
        content: '';
        position: absolute;
        bottom: -8px;
        left: calc(var(--cs-space-lg) + 1px);
        border-width: 8px 8px 0;
        border-style: solid;
        border-color: var(--cs-bg-color) transparent transparent transparent;
        pointer-events: none;
      }
    }

    &.arrow-top {
      &::before {
        content: '';
        position: absolute;
        top: -9px;
        left: var(--cs-space-sm);
        border-width: 0 9px 9px;
        border-style: solid;
        border-color: transparent transparent var(--cs-border-color) transparent;
        pointer-events: none;
      }

      &::after {
        content: '';
        position: absolute;
        top: -8px;
        left: calc(var(--cs-space-sm) + 1px);
        border-width: 0 8px 8px;
        border-style: solid;
        border-color: transparent transparent var(--cs-bg-color) transparent;
        pointer-events: none;
      }
    }

    .item {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: var(--cs-space-xs) var(--cs-space-sm);
      border-radius: var(--cs-border-radius);
      cursor: pointer;

      &:hover {
        background-color: var(--cs-bg-color-deep);
      }

      &.active {
        color: var(--cs-color-primary);
      }

      .name {
        display: flex;
        align-items: center;
        gap: var(--cs-space-xs);
        max-width: calc(100% - 24px);

        span {
          white-space: nowrap;
          text-overflow: ellipsis;
          overflow: hidden;
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-primary);
        }

        .provider-logo {
          width: 16px;
          height: 16px;
          border-radius: 16px;
        }
      }

      .icon {
        flex-shrink: 0;
        display: flex;

        .cs {
          font-size: var(--cs-font-size) !important;
        }
      }

      &.group {
        border-bottom: 1px solid var(--cs-border-color);
        border-radius: 0;

        &:hover {
          background: none;
          cursor: default;
        }

        .name {
          font-size: var(--cs-font-size-sm);
          color: var(--cs-text-color-secondary);
        }
      }
    }
  }
}
</style>