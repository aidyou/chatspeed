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
  if (!model) {
    return "common";
  }
  model = model?.trim()?.toLowerCase();
  // Some service platforms may set the model as company/model name (e.g., openai/gpt-4o),
  // so the company name needs to be removed.
  if (model.indexOf('/') !== -1) {
    model = model.split('/').slice(-1)[0]
  }
  // Exact match dictionary
  const exactMatch = {
    'k1': 'moonshot',
    'k1.5': 'moonshot',
    'k2': 'moonshot',
    'kimi': 'moonshot',
    'deep_seek': 'deepseek'
  };

  // Prefix match dictionary
  const prefixMatch = {
    'k1@': 'moonshot',
    'k1.5@': 'moonshot',
    'k2@': 'moonshot',
    'kimi': 'moonshot',
    'ds-': 'deepseek',
    'deep_seek': 'deepseek',
    'glm-': 'chatglm',
    'meta-': 'llama',
    'hunyuan': 'hunyuan',
    'qwq:': 'qwen',
    'qwq-': 'qwen',
    'qwen': 'qwen',
    'openai': 'gpt',
  };

  // Check exact matches first
  if (exactMatch[model]) {
    return exactMatch[model];
  }

  // Then check prefix matches
  for (const [prefix, logo] of Object.entries(prefixMatch)) {
    if (model.startsWith(prefix)) {
      return logo;
    }
  }

  // get all key start with `ai-${model}`
  const keys = Object.keys(iconFont).filter(key => model.startsWith(key.replace('ai-', '')))
  // get the first key
  return keys.length > 0 ? keys[0].replace('ai-', '') : 'common'
}
