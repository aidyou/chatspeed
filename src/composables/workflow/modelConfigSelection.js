const isThinkingEnabled = thinking => String(thinking?.type || '').toLowerCase() === 'enabled'

export const thinkingLevelFromBudget = budget => {
  const normalized = Number(budget) || 0
  if (normalized > 4096) return 'max'
  if (normalized > 2048) return 'high'
  if (normalized > 1024) return 'medium'
  return 'low'
}

export const resolveActiveModelConfig = (models, planningMode) => {
  const active = planningMode ? models.plan : models.act
  return active?.model || !planningMode ? active : models.act
}

export const resolveDisplayModelConfig = (models, planningMode) => {
  const active = planningMode ? models?.plan : models?.act
  const fallback = planningMode ? models?.act : models?.plan
  return active?.model ? active : fallback
}

export const formatActiveModelName = ({ models, planningMode, modelStore }) => {
  const config = resolveDisplayModelConfig(models, planningMode)
  if (!config?.model) return ''

  const prefix = planningMode ? 'Plan/' : ''
  if (Number(config.id) === 0) {
    const [group, alias] = config.model.split('@')
    return alias ? `${prefix}${alias}(${group})` : `${prefix}${config.model}`
  }

  const provider = modelStore.getModelProviderById(config.id)
  const model = provider?.models?.find(item => item.id === config.model)
  return `${prefix}${model?.name || config.model}`
}

const getProxyTargetModel = (modelStore, settings, group, alias) => {
  const target = settings.chatCompletionProxy?.[group]?.[alias]?.[0]
  if (!target?.id || !target?.model) return null
  return modelStore.getModelProviderById(target.id)?.models?.find(model => model.id === target.model) || null
}

export const getModelConfigForOption = (option, currentConfig) => {
  const next = { ...currentConfig, id: option.id, model: option.model }
  const model = option.targetModel
  if (!model) return next

  next.functionCall = model.functionCall ?? false
  next.thinking = model.thinking || { type: 'disabled' }
  if (model.temperature !== undefined && model.temperature !== null) next.temperature = model.temperature
  if (model.contextSize !== undefined && model.contextSize !== null) next.contextSize = model.contextSize
  if (model.maxTokens !== undefined && model.maxTokens !== null) next.maxTokens = model.maxTokens
  return next
}

export const buildCurrentModelOptions = ({ currentConfig, modelStore, settings }) => {
  const isProxyModel = Number(currentConfig.id) === 0 && currentConfig.model.includes('@')

  if (isProxyModel) {
    const [groupName] = currentConfig.model.split('@')
    const aliases = settings.chatCompletionProxy?.[groupName] || {}
    return [
      {
        key: `proxy-${groupName}`,
        label: groupName,
        models: Object.keys(aliases).map(alias => {
          const targetModel = getProxyTargetModel(modelStore, settings, groupName, alias)
          const model = `${groupName}@${alias}`
          return {
            key: `proxy-${groupName}-${alias}`,
            id: 0,
            model,
            name: alias,
            targetModel,
            selected: currentConfig.model === model,
            supportsThinking:
              Boolean(targetModel?.reasoning) ||
              (currentConfig.model === model && isThinkingEnabled(currentConfig.thinking)),
            thinkingLevel: thinkingLevelFromBudget(currentConfig.thinking?.budgetTokens)
          }
        })
      }
    ]
  }

  const provider = modelStore.getModelProviderById(currentConfig.id)
  if (!provider) return []

  return [
    {
      key: `provider-${provider.id}`,
      label: provider.name,
      models: (provider.models || []).map(model => ({
        key: `provider-${provider.id}-${model.id}`,
        id: provider.id,
        model: model.id,
        name: model.name || model.id,
        targetModel: model,
        selected: currentConfig.model === model.id,
        supportsThinking:
          Boolean(model.reasoning) ||
          (currentConfig.model === model.id && isThinkingEnabled(currentConfig.thinking)),
        thinkingLevel: thinkingLevelFromBudget(currentConfig.thinking?.budgetTokens)
      }))
    }
  ]
}
