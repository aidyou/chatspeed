export const DEFAULT_EXECUTION_STYLE = 'preset:default'

export const EXECUTION_STYLE_PRESETS = Object.freeze([
  {
    value: 'preset:executor',
    labelKey: 'settings.agent.personalityExecutor',
    descriptionKey: 'settings.agent.personalityExecutorDescription'
  },
  {
    value: 'preset:companion',
    labelKey: 'settings.agent.personalityCompanion',
    descriptionKey: 'settings.agent.personalityCompanionDescription'
  },
  {
    value: 'preset:expert',
    labelKey: 'settings.agent.personalityExpert',
    descriptionKey: 'settings.agent.personalityExpertDescription'
  },
  {
    value: 'preset:researcher',
    labelKey: 'settings.agent.personalityResearcher',
    descriptionKey: 'settings.agent.personalityResearcherDescription'
  },
  {
    value: 'preset:coach',
    labelKey: 'settings.agent.personalityCoach',
    descriptionKey: 'settings.agent.personalityCoachDescription'
  },
  {
    value: 'preset:reviewer',
    labelKey: 'settings.agent.personalityReviewer',
    descriptionKey: 'settings.agent.personalityReviewerDescription'
  }
])

export const EXECUTION_STYLE_PRESET_VALUES = new Set(
  [DEFAULT_EXECUTION_STYLE, ...EXECUTION_STYLE_PRESETS].map(preset =>
    typeof preset === 'string' ? preset : preset.value
  )
)

export const normalizeExecutionStyle = value => String(value || '').trim()

export const getAgentCustomExecutionStyle = agent => {
  const style = normalizeExecutionStyle(agent?.personality)
  return style && !style.startsWith('preset:') ? style : ''
}

export const isExecutionStyleAvailable = (style, agent) => {
  const normalized = normalizeExecutionStyle(style)
  return (
    !normalized ||
    EXECUTION_STYLE_PRESET_VALUES.has(normalized) ||
    normalized === getAgentCustomExecutionStyle(agent)
  )
}

export const resolveExecutionStylePreference = (style, agent) => {
  const normalized = normalizeExecutionStyle(style)
  if (!normalized) return DEFAULT_EXECUTION_STYLE
  return isExecutionStyleAvailable(normalized, agent) ? normalized : DEFAULT_EXECUTION_STYLE
}

export const getExecutionStyleOptions = agent => {
  const customStyle = getAgentCustomExecutionStyle(agent)
  return [
    {
      value: DEFAULT_EXECUTION_STYLE,
      labelKey: 'settings.agent.personalityDefault',
      descriptionKey: 'settings.agent.personalityDefaultDescription'
    },
    ...EXECUTION_STYLE_PRESETS,
    ...(customStyle
      ? [
          {
            value: customStyle,
            labelKey: 'settings.agent.personalityCustom',
            descriptionKey: 'settings.agent.personalityCustomHint'
          }
        ]
      : [])
  ]
}
