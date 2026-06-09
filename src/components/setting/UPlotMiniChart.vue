<template>
  <div ref="rootRef" class="uplot-mini-chart">
    <div ref="chartRef" class="uplot-mini-chart__canvas"></div>
  </div>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'

const props = defineProps({
  labels: {
    type: Array,
    default: () => []
  },
  series: {
    type: Array,
    default: () => []
  },
  bands: {
    type: Array,
    default: () => []
  },
  height: {
    type: Number,
    default: 320
  },
  xAxisLabel: {
    type: String,
    default: ''
  },
  scaleConfigs: {
    type: Object,
    default: () => ({
      y: {}
    })
  }
})

const rootRef = ref(null)
const chartRef = ref(null)
const plotRef = ref(null)
const resizeObserverRef = ref(null)

const nativeSeries = computed(() =>
  props.series.filter(item => item.type !== 'stackedBar' && item.type !== 'groupedBar')
)

const stackedBarSeries = computed(() => props.series.filter(item => item.type === 'stackedBar'))

const groupedBarSeries = computed(() => props.series.filter(item => item.type === 'groupedBar'))

const chartData = computed(() => {
  const xValues = props.labels.map((_, index) => index)
  // Include all series data for legend display
  const seriesData = props.series.map(item => item.values || [])
  return [xValues, ...seriesData]
})

const activeScaleNames = computed(() => {
  const names = new Set(['y'])
  for (const item of props.series) {
    if (item.scale) names.add(item.scale)
  }
  return Array.from(names)
})

const buildAxisFormatter = scaleName => {
  const config = props.scaleConfigs?.[scaleName] || {}
  if (typeof config.formatter === 'function') {
    return (_u, values) => values.map(value => config.formatter(value))
  }
  return undefined
}

const getScaleSeriesValues = scaleName =>
  props.series
    .filter(item => (item.scale || 'y') === scaleName)
    .flatMap(item => item.values || [])
    .map(value => Number(value))
    .filter(value => Number.isFinite(value))

const estimateAxisSize = scaleName => {
  const config = props.scaleConfigs?.[scaleName] || {}
  if (config.size) return config.size

  const defaultSize = config.side === 'right' ? 52 : 56
  if (typeof config.formatter !== 'function') return defaultSize

  const values = getScaleSeriesValues(scaleName)
  if (!values.length) return defaultSize

  const minValue = Math.min(...values, 0)
  const maxValue = Math.max(...values, 0)
  const samples = [minValue, minValue / 2, 0, maxValue / 2, maxValue]
  const maxLabelLength = samples
    .map(value => `${config.formatter(value)}`)
    .reduce((max, label) => Math.max(max, label.length), 0)

  return Math.max(defaultSize, Math.min(120, maxLabelLength * 8 + 20))
}

const getXAxisSplits = () => {
  const xValues = chartData.value[0] || []
  const total = xValues.length
  if (total <= 1) return xValues

  const maxTicks = Math.max(2, props.scaleConfigs?.x?.maxTicks || 8)
  if (total <= maxTicks) return xValues

  const step = Math.ceil((total - 1) / (maxTicks - 1))
  const splits = []
  for (let index = 0; index < total; index += step) {
    splits.push(xValues[index])
  }

  const lastValue = xValues[total - 1]
  if (splits[splits.length - 1] !== lastValue) {
    splits.push(lastValue)
  }

  return splits
}

const buildOptions = width => {
  const labels = props.labels
  const scales = {
    x: {
      time: false,
      ...(props.scaleConfigs?.x || {})
    }
  }

  for (const scaleName of activeScaleNames.value) {
    const config = props.scaleConfigs?.[scaleName] || {}
    scales[scaleName] = {
      auto: config.range == null,
      range: config.range
    }
  }

  const axes = [
    {
      stroke: 'rgba(148, 163, 184, 0.6)',
      grid: {
        stroke: 'rgba(148, 163, 184, 0.18)',
        width: 1
      },
      splits: () => getXAxisSplits(),
      values: (_u, values) =>
        values.map(value => {
          const index = Math.round(value)
          return labels[index] || ''
        }),
      size: 44
    }
  ]

  for (const scaleName of activeScaleNames.value) {
    const config = props.scaleConfigs?.[scaleName] || {}
    axes.push({
      scale: scaleName,
      side: config.side === 'right' ? 1 : 3,
      stroke: 'rgba(148, 163, 184, 0.6)',
      grid:
        scaleName === 'y'
          ? {
              stroke: 'rgba(148, 163, 184, 0.18)',
              width: 1
            }
          : { show: false },
      values: buildAxisFormatter(scaleName),
      size: estimateAxisSize(scaleName)
    })
  }

  const series = [
    {
      label: props.xAxisLabel,
      value: (_u, idx) => {
        if (idx == null) return '--'
        return props.labels[idx] || '--'
      }
    },
    ...props.series.map((item, index) => {
      const isBarType = item.type === 'groupedBar' || item.type === 'stackedBar'
      return {
        label: item.label,
        stroke: item.strokeColor || item.color,
        width: isBarType ? 0 : item.width || 2,
        fill: isBarType ? null : item.fill || null,
        scale: item.scale || 'y',
        paths:
          item.type === 'spline'
            ? uPlot.paths.spline()
            : item.type === 'bars'
              ? uPlot.paths.bars({
                  gap: item.barGap ?? 4,
                  radius: item.barRadius ?? [6, 0],
                  size: item.barSize ?? [0.72, 80, 12]
                })
              : undefined,
        points: {
          show: isBarType ? false : item.type === 'bars' ? false : item.showPoints !== false,
          size: item.pointSize || 5,
          width: 0,
          fill: item.strokeColor || item.color,
          stroke: item.strokeColor || item.color
        },
        value: (_u, value) => {
          if (value == null) return '--'
          if (typeof item.valueFormatter === 'function') return item.valueFormatter(value)
          return `${value}`
        },
        alpha: item.alpha ?? (isBarType ? 0 : 1)
      }
    })
  ]

  return {
    width,
    height: props.height,
    legend: {
      show: true,
      markers: {
        show: true,
        width: 0,
        dash: 'solid',
        fill: (u, seriesIdx) => {
          return (
            props.series[seriesIdx - 1]?.color || props.series[seriesIdx - 1]?.strokeColor || '#000'
          )
        },
        stroke: (u, seriesIdx) => {
          return (
            props.series[seriesIdx - 1]?.strokeColor || props.series[seriesIdx - 1]?.color || '#000'
          )
        }
      }
    },
    cursor: {
      drag: {
        setScale: false
      },
      focus: {
        prox: 0
      }
    },
    scales,
    axes,
    series,
    bands: props.bands.map(item => ({
      series: [item.from + 1, item.to + 1],
      fill: item.fill,
      dir: item.dir ?? -1
    })),
    hooks: {
      drawClear: [
        u => {
          if (!stackedBarSeries.value.length) return

          const ctx = u.ctx
          const xValues = chartData.value[0] || []
          if (!xValues.length) return

          const scaleKey = stackedBarSeries.value[0]?.scale || 'y'
          const barSpan =
            xValues.length > 1
              ? Math.abs(u.valToPos(xValues[1], 'x', true) - u.valToPos(xValues[0], 'x', true))
              : u.bbox.width
          const barWidth = Math.max(12, Math.min(52, barSpan * 0.64))
          const radius = Math.min(6, barWidth / 4)

          ctx.save()

          for (let index = 0; index < xValues.length; index += 1) {
            const center = u.valToPos(xValues[index], 'x', true)
            const left = Math.round(center - barWidth / 2)
            let baseValue = 0

            stackedBarSeries.value.forEach((item, seriesIndex) => {
              const rawValue = Number(item.values?.[index] || 0)
              const topValue = baseValue + rawValue
              const yBase = u.valToPos(baseValue, scaleKey, true)
              const yTop = u.valToPos(topValue, scaleKey, true)
              const rectTop = Math.min(yBase, yTop)
              const rectHeight = Math.max(0, Math.abs(yBase - yTop))

              if (rectHeight <= 0) {
                baseValue = topValue
                return
              }

              const isTopSegment =
                seriesIndex === stackedBarSeries.value.length - 1 ||
                !stackedBarSeries.value[seriesIndex + 1].values?.[index]

              ctx.beginPath()
              if (isTopSegment) {
                ctx.moveTo(left, yBase)
                ctx.lineTo(left, rectTop + radius)
                ctx.quadraticCurveTo(left, rectTop, left + radius, rectTop)
                ctx.lineTo(left + barWidth - radius, rectTop)
                ctx.quadraticCurveTo(left + barWidth, rectTop, left + barWidth, rectTop + radius)
                ctx.lineTo(left + barWidth, yBase)
              } else {
                ctx.rect(left, rectTop, barWidth, rectHeight)
              }
              ctx.closePath()
              ctx.fillStyle = item.color
              ctx.fill()

              baseValue = topValue
            })
          }

          ctx.restore()
        }
      ],
      draw: [
        u => {
          if (!groupedBarSeries.value.length) return

          const ctx = u.ctx
          const xValues = chartData.value[0] || []
          if (!xValues.length) return

          const scaleKey = groupedBarSeries.value[0]?.scale || 'y'
          const groupSpan =
            xValues.length > 1
              ? Math.abs(u.valToPos(xValues[1], 'x', true) - u.valToPos(xValues[0], 'x', true))
              : u.bbox.width
          const groupWidth = Math.max(18, Math.min(64, groupSpan * 0.78))
          const barGap = Math.max(2, groupWidth * 0.08)
          const barWidth =
            (groupWidth - barGap * (groupedBarSeries.value.length - 1)) /
            groupedBarSeries.value.length
          const radius = Math.min(6, barWidth / 4)

          ctx.save()

          for (let index = 0; index < xValues.length; index += 1) {
            const center = u.valToPos(xValues[index], 'x', true)
            const groupLeft = center - groupWidth / 2

            groupedBarSeries.value.forEach((item, seriesIndex) => {
              const rawValue = Number(item.values?.[index] || 0)
              const yBase = u.valToPos(0, scaleKey, true)
              const yTop = u.valToPos(rawValue, scaleKey, true)
              const rectTop = Math.min(yBase, yTop)
              const rectHeight = Math.max(0, Math.abs(yBase - yTop))

              if (rectHeight <= 0) return

              const left = Math.round(groupLeft + seriesIndex * (barWidth + barGap))

              ctx.beginPath()
              ctx.moveTo(left, yBase)
              ctx.lineTo(left, rectTop + radius)
              ctx.quadraticCurveTo(left, rectTop, left + radius, rectTop)
              ctx.lineTo(left + barWidth - radius, rectTop)
              ctx.quadraticCurveTo(left + barWidth, rectTop, left + barWidth, rectTop + radius)
              ctx.lineTo(left + barWidth, yBase)
              ctx.closePath()
              ctx.fillStyle = item.color
              ctx.fill()
            })
          }

          ctx.restore()
        }
      ]
    }
  }
}

const destroyPlot = () => {
  if (plotRef.value) {
    plotRef.value.destroy()
    plotRef.value = null
  }
}

const renderPlot = async () => {
  await nextTick()
  if (!chartRef.value || !rootRef.value) return
  const width = Math.max(rootRef.value.clientWidth || 0, 320)
  destroyPlot()
  plotRef.value = new uPlot(buildOptions(width), chartData.value, chartRef.value)
}

watch(
  () => [props.labels, props.series, props.scaleConfigs, props.bands],
  () => {
    renderPlot()
  },
  { deep: true }
)

onMounted(() => {
  renderPlot()
  if (typeof ResizeObserver !== 'undefined' && rootRef.value) {
    resizeObserverRef.value = new ResizeObserver(() => {
      renderPlot()
    })
    resizeObserverRef.value.observe(rootRef.value)
  }
})

onBeforeUnmount(() => {
  resizeObserverRef.value?.disconnect()
  destroyPlot()
})
</script>

<style lang="scss" scoped>
.uplot-mini-chart {
  position: relative;
  width: 100%;
  min-height: 320px;

  &__canvas {
    width: 100%;
    height: 100%;
  }

  :deep(.uplot) {
    font-family: inherit;
    background: transparent;
  }

  :deep(.u-legend .u-head) {
    display: none;
  }

  :deep(.u-wrap) {
    background: transparent;
  }
}
</style>
