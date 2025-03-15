import iconFont from '@/components/icon/type.js'

/**
 * Retrieves the logo key associated with a given model.
 *
 * This function converts the input model to lowercase and searches for
 * keys in the iconFont object that start with `ai-${model}`. If a matching
 * key is found, it returns the first one; otherwise, it returns 'ai-common'.
 *
 * @param {string} model - The model name to search for.
 * @returns {string} - The corresponding logo key or 'ai-common' if no match is found.
 */
export function getModelLogo(model) {
  model = model.toLowerCase();
  // Some service platforms may set the model as company/model name (e.g., openai/gpt-4o),
  // so the company name needs to be removed.
  if (model.indexOf('/') !== -1) {
    model = model.split('/').slice(-1)[0]
  }
  if (model === 'k1' || model === 'kimi' || model.startsWith('k1@') || model.startsWith('kimi@')) {
    return 'moonshot'
  }
  if (model === 'deep_seek' || model.startsWith('ds-') || model.startsWith('deep_seek')) {
    return 'deepseek'
  }
  if (model.startsWith('glm-')) {
    return 'chatglm'
  }
  // 对一些模型的名称做处理
  if (model.startsWith('meta-')) {
    return 'llama'
  }
  if (model.startsWith('hunyuan')) {
    return 'hunyuan'
  }
  if (model.startsWith('qwq:') || model.startsWith('qwq-')) {
    return 'qwen'
  }
  // get all key start with `ai-${model}`
  const keys = Object.keys(iconFont).filter(key => model.startsWith(key.replace('ai-', '')))
  // get the first key
  return keys.length > 0 ? keys[0].replace('ai-', '') : 'common'
}
