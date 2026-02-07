<template>
  <i class="cs" :class="[iconClass, isActive ? 'active' : '', classColor]" :style="mergeStyle" />
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import Icon from './type.js'

const props = defineProps({
  name: {
    type: String,
    default: '',
  },
  size: {
    type: [String, Number],
    default: 'normal',
  },
  color: {
    type: String,
  },
  iconStyle: {
    type: Object,
    default: () => ({}),
  },
  active: {
    type: [Boolean, String],
    default: false,
  },
})

const IconRef = ref(Icon)
const classColor = ref('')
const localIconStyle = ref({ ...props.iconStyle }) // Create a local copy.

watch(
  () => props.color,
  newColor => {
    const colorDict = {
      primary: 'primary',
      'primary-deep': 'primary-deep',
      secondary: 'secondary',
      black: 'color',
      gray: 'gray',
      light: 'light',
      red: 'red',
    }
    classColor.value = newColor && colorDict[newColor] ? `color-${colorDict[newColor]}` : ''
    if (newColor && !colorDict[newColor]) {
      localIconStyle.value['color'] = newColor
    }
  },
  { immediate: true }
)

const mergeStyle = computed(() => {
  let icStyle = { ...localIconStyle.value }
  if (props.size) {
    let fontSize = '14px'
    const sizeDict = {
      xxs: '8px',
      xs: '10px',
      sm: '12px',
      normal: '14px',
      md: '24px',
      lg: '48px',
      '1x': '1em',
      '2x': '2em',
      '3x': '3em',
      '4x': '4em',
      '5x': '5em',
    }
    if (typeof props.size === 'number') {
      fontSize = props.size + 'px'
    } else if (sizeDict[props.size]) {
      fontSize = sizeDict[props.size]
    } else {
      fontSize = props.size
    }
    icStyle['font-size'] = fontSize
  }
  // Explicitly set font-family in inline style to ensure it takes precedence
  // This is critical for Windows where font loading can be flaky
  return {
    fontFamily: 'chatspeed !important',
    ...icStyle,
  }
})

const isActive = computed(() => String(props.active) !== 'false')

const iconClass = computed(() => {
  return props.name ? `cs-${props.name}` : ''
  // return !IconRef.value || !props.name || typeof IconRef.value[props.name] === 'undefined'
  //   ? ''
  //   : `cs-${IconRef.value[props.name]}`
})
</script>

<style lang="scss">
.cs {
  font-family: 'chatspeed' !important;
  text-decoration: none;
  text-align: center;
  font-style: normal;
  display: inline-block;
  outline: none !important;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

.color-black {
  color: var(--cs-text-color-primary);
}

.color-secondary {
  color: var(--cs-text-color-secondary);
}

.color-primary {
  color: var(--cs-color-primary);
}

.color-primary-deep {
  color: var(--cs-color-primary-dark);
}

.color-red {
  color: var(--cs-error-color);
}

.color-gray {
  color: var(--cs-text-color-tertiary);
}

.clolr-light {
  color: var(--cs-text-color-placeholder);
}

.active {
  color: var(--cs-color-primary) !important;
}

.cs-md {
  font-size: 1.5em;
}

.cs-2x,
.cs-lg {
  font-size: 2em;
}

.cs-3x,
.cs-xl {
  font-size: 3em;
}

.cs-4x,
.cs-xxl {
  font-size: 4em;
}
</style>
