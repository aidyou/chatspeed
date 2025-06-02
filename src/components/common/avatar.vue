<script setup>
import { computed } from 'vue'

const props = defineProps({
  text: {
    type: String,
    required: true
  },
  size: {
    type: [Number, String],
    default: 40
  },
  bgColor: {
    type: String,
    default: ''
  },
  textColor: {
    type: String,
    default: ''
  }
})

const baseColors = ['#10897C', '#FB452E', '#5BA006', '#1399EF', '#643EFF', '#EA950F', '#E901F2']

// Get display text (first letter or first CJK character)
const displayText = computed(() => {
  if (!props.text) return ''

  // Check if character is CJK
  const firstChar = props.text[0]
  const isCJK = /[\u4e00-\u9fff]/.test(firstChar)

  return isCJK ? firstChar : firstChar.toUpperCase()
})

// Generate stable color based on text hash
const backgroundColor = computed(() => {
  if (props.bgColor) {
    return props.bgColor
  }

  // Simple string hash function
  const hash = str => {
    let hash = 0
    for (let i = 0; i < str.length; i++) {
      hash = str.charCodeAt(i) + ((hash << 5) - hash)
    }
    return hash
  }

  // Use text hash to select base color
  const textHash = hash(props.text)
  const baseColor = baseColors[Math.abs(textHash) % baseColors.length]

  // Generate similar color (slightly adjust HSL values)
  const color = baseColor.substring(1)
  const rgb = parseInt(color, 16)
  const r = (rgb >> 16) & 0xff
  const g = (rgb >> 8) & 0xff
  const b = (rgb >> 0) & 0xff

  // Use hash for stable adjustment
  const adjust = ((textHash % 20) - 10) / 100
  const adjustedR = Math.min(255, Math.max(0, r + r * adjust))
  const adjustedG = Math.min(255, Math.max(0, g + g * adjust))
  const adjustedB = Math.min(255, Math.max(0, b + b * adjust))

  return `#${Math.round(adjustedR).toString(16).padStart(2, '0')}${Math.round(adjustedG)
    .toString(16)
    .padStart(2, '0')}${Math.round(adjustedB).toString(16).padStart(2, '0')}`
})

// Calculate styles
const style = computed(() => {
  let sizeStr =
    typeof props.size === 'number' || (typeof props.size === 'string' && /^\d+$/.test(props.size))
      ? `${props.size}px`
      : props.size
  return {
    width: sizeStr,
    height: sizeStr,
    borderRadius: sizeStr,
    backgroundColor: props.bgColor || backgroundColor.value,
    color: props.textColor || 'white',
    fontSize: `calc(${typeof props.size === 'number' ? props.size : parseInt(props.size)}px * 0.5)`
  }
})
</script>

<template>
  <div class="avatar" :style="style">
    {{ displayText }}
  </div>
</template>

<style scoped>
.avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  color: v-bind('textColor || "white"');
  font-weight: bold;
  user-select: none;
  flex-shrink: 0;
}
</style>
