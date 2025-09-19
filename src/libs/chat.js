import i18n from '@/i18n/index.js'
import { invoke } from '@tauri-apps/api/core'
import he from 'he'
import { marked } from 'marked'

// Regular expressions
const CODE_BLOCK_REGEX = /```([^\n]*)\n([\s\S]*?)```/g
const REFERENCE_REGEX = /\[([0-9,\s]+)\]\(@ref\)/g
const REFERENCE_LINK_ALTERNATIVE_REGEX = /\[\^([0-9]+)\^]/g
const REFERENCE_LINK_ALTERNATIVE_2_REGEX = /\[\[([0-9]+)\]\]/g
const REFERENCE_BLOCK_REGEX = /`\[\^[0-9]+\]`/g
const REFERENCE_CITATION_REGEX = /\[citation:(\d+)\]/g
const THINK_REGEX = /<think(\s+class="([^"]*)")?>([\s\S]+?)<\/think>/ // just deal the first think tag
const LINE_BREAK_REGEX = /([^\n])\n(?!\n)/g
// const BLOCK_CODE_REGEX = /\n*```([a-zA-Z\#]+\s+)?([\s\S]+?)```\n*/g;
const THINK_CONTENT_REGEX = /<think>[\s\S]+?<\/think>/
const PLACEHOLDER_RESTORE_REGEX = /___(?:CODE|MATH|BLOCK_MATH|THINK)_\d+___/g

const MATH_BLOCK_REGEX = /\$\$([\s\S]+?)\$\$/g
const CHINESE_CHARS_REGEX = /[\u4e00-\u9fa5]/
const CHINESE_CHARS_GROUP_REGEX = /([\u4e00-\u9fa5]+)/g

import { getLanguageByCode } from '@/i18n/langUtils'
import { useSettingStore } from '@/stores/setting'

const settingStore = useSettingStore()

/**
 * Preprocess chat messages before sending to AI
 * Used in both chat window and tool pages:
 * - Chat window: Handles regular chat with optional skills
 * - Tool pages: Usually uses skills but can also handle regular chat
 *
 * @param {string} inputMessage - User's input message
 * @param {Array<{role: string, content: string, metadata: Object<{toolCalls: Array<{name: string, args: Object}>}>}>} historyMessages - Previous conversation messages
 * @param {string} quoteMessage - Previously quoted AI response
 * @param {Object} [skill] - Optional skill configuration
 * @param {string} [skill.prompt] - Skill prompt template
 * @param {Object} [skill.metadata] - Skill metadata
 * @param {string} [skill.metadata.type] - Skill type (e.g. 'translation')
 * @param {boolean} [skill.metadata.useSystemRole] - Whether to use system role
 * @param {Object} [metadata] - Additional processing parameters
 * @param {string} [metadata.sourceLang] - Source language for translation
 * @param {string} [metadata.targetLang] - Target language for translation
 * @returns {Array<{role: string, content: string}>} Processed messages ready for AI
 */
export const chatPreProcess = async (inputMessage, historyMessages, skill, metadata = {}) => {
  const messages = []
  const skillType = skill?.metadata?.type
  const useSystemRole = skill?.metadata?.useSystemRole
  const prompt = skill?.prompt?.trim() || ''
  let processedPrompt = ''

  // Handle translation skill separately
  if (skillType === 'translation') {
    if (!prompt) {
      throw new Error(i18n.global.t('chat.translationSkillPromptEmpty'))
    }
    processedPrompt = await processTranslationPrompt(
      prompt,
      metadata?.sourceLang,
      metadata?.targetLang,
      inputMessage // 翻译技能不使用引用消息
    )
    messages.push({ role: 'user', content: processedPrompt })
  } else {
    if (useSystemRole) {
      // Handle regular skills
      if (prompt) {
        messages.push({ role: 'system', content: prompt })
      }
      messages.push({ role: 'user', content: inputMessage })
    } else {
      // Combine prompt and user message based on whether prompt contains {content}
      let finalContent = ''
      if (prompt.includes('{content}')) {
        finalContent = prompt.replace(/\{content\}/g, inputMessage)
      } else {
        finalContent = prompt ? `${prompt}\n\n${inputMessage}` : inputMessage
      }
      messages.push({ role: 'user', content: finalContent })
    }
  }

  // Add history messages to the messages array
  const history = buildHistoryMessages(historyMessages)

  // Handle system role messages
  if (useSystemRole && messages[0]?.role === 'system') {
    const systemMessage = messages[0]
    const userMessages = messages.slice(1)
    return [systemMessage, ...history, ...userMessages]
  }

  return [...history, ...messages]
}

/**
 * Converts history messages array to the format expected by AI
 * Performs deep copy to avoid modifying original messages
 * Handles tool calls by splitting them into separate assistant and tool messages
 *
 * @param {Array<{role: string, content: string, metadata: Object}>} historyMessages - Array of previous conversation messages
 * @returns {Array<{role: string, content: string, tool_calls?: Array, tool_call_id?: string}>} Formatted history messages
 */
function buildHistoryMessages(historyMessages) {
  const acc = historyMessages.reduceRight(
    (acc, message) => {
      // If the contextCleared marker is encountered, stop collecting
      if (message.metadata?.contextCleared) {
        acc.stop = true
        return acc
      }

      // If collection has already stopped, return directly
      if (acc.stop) {
        return acc
      }

      // Skip if current message has same role as the last message added to the accumulator.
      // This ensures the roles are always alternating.
      // Note: With tool calls, we may have sequences like: user -> assistant -> tool -> user
      // So we need to check the original role, not the potentially added tool messages
      if (acc.messages.length > 0) {
        const lastOriginalRole = acc.lastOriginalRole || acc.messages[0].role
        if (message.role === lastOriginalRole) {
          return acc
        }
      }

      if (message.role === 'assistant') {
        // Handle assistant messages with potential tool calls
        const cleanContent = message.content.replace(/<think>[\s\S]*?<\/think>/g, '')
        const toolCalls = message.metadata?.toolCall

        if (toolCalls && Array.isArray(toolCalls) && toolCalls.length > 0) {
          // Only include tool calls if this is the most recent assistant message with tools
          // This prevents context overflow from accumulating too many historical tool results
          const shouldIncludeToolResults = !acc.hasIncludedToolResults

          if (shouldIncludeToolResults) {
            // Mark that we've included tool results to prevent including older ones
            acc.hasIncludedToolResults = true

            // Add tool result messages (in reverse order since we're using unshift)
            for (let i = toolCalls.length - 1; i >= 0; i--) {
              const toolCall = toolCalls[i]
              acc.messages.unshift({
                role: 'tool',
                content:
                  typeof toolCall.result === 'string'
                    ? toolCall.result
                    : JSON.stringify(toolCall.result),
                tool_call_id: toolCall.id
              })
            }

            // Add assistant message with tool_calls since we are including the results
            acc.messages.unshift({
              role: 'assistant',
              content: cleanContent,
              tool_calls: toolCalls.map(toolCall => ({
                id: toolCall.id,
                type: toolCall.type || 'function',
                function: {
                  name: toolCall.function.name,
                  arguments: toolCall.function.arguments
                }
              }))
            })
          } else {
            // If not including tool results, add a regular assistant message without tool_calls
            acc.messages.unshift({
              role: 'assistant',
              content: cleanContent
            })
          }
        } else {
          // Regular assistant message without tool calls
          acc.messages.unshift({
            role: 'assistant',
            content: cleanContent
          })
        }
      } else {
        // User message - keep as is
        acc.messages.unshift({
          role: message.role,
          content: message.content
        })
      }

      // Track the last original role for alternating check
      acc.lastOriginalRole = message.role
      return acc
    },
    { messages: [], stop: false, lastOriginalRole: null, hasIncludedToolResults: false }
  )

  const processedMessages = acc.messages

  // Ensure the first message is from the user
  if (processedMessages.length > 0 && processedMessages[0].role === 'assistant') {
    processedMessages.shift()
  }

  return processedMessages
}

/**
 * Combines user input with quoted message if present
 * Adds a transition prompt between quote and new input
 *
 * @param {string} inputMessage - User's new input message
 * @param {string} quoteMessage - Previously quoted AI response
 * @returns {string} Combined message with quote and input
 */
export function buildUserMessage(inputMessage, quoteMessage) {
  if (!quoteMessage) {
    return inputMessage || ''
  }
  return `<quoted-response>\n${quoteMessage}\n</quoted-response>\n\n<system-reminder>User quoted your response. Please respond considering the quoted content.</system-reminder>\n\n${inputMessage}`
}

/**
 * Processes the translation prompt by replacing placeholders with actual language values.
 *
 * The function performs the following steps:
 * 1. If the source language code is not provided, it attempts to detect the language from the input message.
 * 2. If detection is successful, it assigns the detected language and code to the respective variables.
 * 3. If the source language code is provided, it retrieves the corresponding language name from the language dictionary.
 * 4. It determines the target language code using the `getTranslationTargetLang` function.
 * 5. The prompt is then updated by replacing the placeholders for the source language, target language, and content with their actual values.
 * 6. Finally, it logs the source and target languages, as well as the final prompt, before returning the processed prompt.
 *
 * @param {string} prompt - The initial prompt containing placeholders for language and content.
 * @param {string} sourceLangCode - The language code of the source language. If not provided, it will be detected.
 * @param {string} targetLangCode - The language code of the target language. If not provided, it will be determined based on user settings.
 * @param {string} inputMessage - The input message to be translated.
 * @returns {Promise<string>} - A promise that resolves to the processed prompt with placeholders replaced by actual values.
 */
export const processTranslationPrompt = async (
  prompt,
  sourceLangCode,
  targetLangCode,
  inputMessage
) => {
  let sourceLang = ''
  let targetLang = ''

  if (!sourceLangCode) {
    // detect from language
    try {
      const result = await invoke('detect_language', { text: inputMessage })
      sourceLang = result.lang
      sourceLangCode = result.code
      console.log('Detected source language:', sourceLang, 'text:', inputMessage)
    } catch (error) {
      console.error('Error detecting language:', error)
      return '' // Return empty string on error
    }
  } else {
    sourceLang = getLanguageByCode(sourceLangCode) || 'chinese'
  }

  targetLangCode = getTranslationTargetLang(sourceLangCode, targetLangCode)
  targetLang = getLanguageByCode(targetLangCode) || 'english'

  prompt = prompt
    .replace(/\{fromLang\}/g, sourceLang)
    .replace(/\{toLang\}/g, targetLang)
    .replace(/\{content\}/g, inputMessage)
  console.log('From language:', sourceLang, 'To language:', targetLang)
  console.log('Final prompt:', prompt)
  return prompt
}

/**
 * Determines appropriate target language for translation based on source and user settings
 *
 * @param {string} sourceLang - Source language code (without region)
 * @param {string} targetLang - Optional user-specified target language code
 * @returns {string} Resolved target language code
 */
export const getTranslationTargetLang = (sourceLang, targetLang) => {
  // If user explicitly set target language, use it
  if (targetLang) {
    return targetLang
  }

  const primaryLang = settingStore.settings.primaryLanguage // e.g. "zh-Hans"
  const secondaryLang = settingStore.settings.secondaryLanguage // e.g. "en"

  // Extract base language code without region
  const primaryBaseLang = primaryLang.split('-')[0] // e.g. "zh"
  const secondaryBaseLang = secondaryLang.split('-')[0] // e.g. "en"

  // If source matches primary language base code, use secondary language
  if (sourceLang === primaryBaseLang) {
    return secondaryLang || 'en' // Fallback to en if no secondary language
  }

  // If source matches secondary language base code, use primary language
  if (sourceLang === secondaryBaseLang) {
    return primaryLang
  }

  // For other source languages, prefer primary language
  return primaryLang
}

/**
 * Converts special characters to HTML entities
 * Only escapes < and > when they are likely part of HTML tags
 * @param {string} text - The text to be escaped
 * @returns {string} - Escaped text with HTML entities
 */
export const htmlspecialchars = text => {
  // const map = {
  //   '&': '&amp;',
  //   '"': '&quot;',
  //   "'": '&#039;'
  // }
  // return text
  //   .replace(/[&"']/g, m => map[m])
  //   .replace(/<([a-zA-Z][^>\n]*?)>/g, (_match, p1) => `&lt;${p1}&gt;`)
  //   .replace(/<\/[a-zA-Z]+>/g, (match) => `&lt;/${match.slice(2, -1)}&gt;`)
  return he.encode(text, { '&': false })
}

/**
 * modify parseMarkdown function
 */
export const parseMarkdown = (content, reference, toolCalls) => {
  content = content ? content.trim() : ''
  if (!content) return ''

  // remove reminder
  content = content.replace(/<system-reminder>[\s\S]+?<\/system-reminder>/gi, '')

  // let refs = ''
  // format refs [1,2,3](@ref) -> [[1]][[2]][[3]]
  content = content.replace(REFERENCE_REGEX, (_match, numbers) => {
    return numbers
      .split(',')
      .map(num => `[[${num.trim()}]]`)
      .join('')
  })
  // format refs [^1^] -> [[1]]
  content = content.replace(REFERENCE_LINK_ALTERNATIVE_REGEX, (_match, number) => {
    return `[[${number.trim()}]]`
  })
  // format refs [[1]] -> [[1]] (normalization)
  content = content.replace(REFERENCE_LINK_ALTERNATIVE_2_REGEX, (_match, number) => {
    return `[[${number.trim()}]]`
  })
  // format refs [citation:1] -> [[1]]
  content = content.replace(REFERENCE_CITATION_REGEX, (_match, number) => {
    return `[[${number.trim()}]]`
  })

  // Text like `[1]` needs to be replaced first; otherwise, the subsequent reference parsing will be converted to a code block by mk.
  const refBlocks = new Map()
  let refCounter = 0

  // Replace regular reference text with placeholders
  content = content.replace(REFERENCE_BLOCK_REGEX, match => {
    const id = `___REF_${refCounter++}___`
    refBlocks.set(id, match)
    return id
  })

  if (Array.isArray(reference) && reference.length > 0) {
    reference.forEach(item => {
      content = content.replace(
        new RegExp(`\\[\\[${item.id}\\]\\]`, 'g'),
        `<a href="${item.url}" class="reference-link l" title="${item.title.replace(/"/g, "'").trim()}">${item.id}</a>`
      )
    })
  } else {
    //remove all reference blocks
    content = content.replace(/\[\[\d+\]\]/g, '')
  }

  // Replace placeholders back to original reference text
  refBlocks.forEach((value, key) => {
    content = content.replace(key, value)
  })

  // Handle reasoning process similar to deepseek r1
  if (content.startsWith('<think')) {
    if (content.indexOf('</think>') === -1) {
      content = `<div class="chat-think">
          <div class="chat-think-title expanded"><span>${i18n.global.t('chat.reasoning')}</span></div>
          <div class="think-content">${content.replace('<think>', '')}</div>
        </div>\n`
    }
  }
  // remove <think>\s+</think>
  if (/<think>\s*<\/think>/.test(content)) {
    content = content.replace(/<think>\s*<\/think>/, '')
  }

  content = content.replace(THINK_REGEX, (_match, _classAttr, className, content) => {
    const translationKey = className?.includes('thinking')
      ? 'chat.reasoning'
      : 'chat.reasoningProcess'
    return `<div class="chat-think ${className || ''}"><div class="chat-think-title expanded"><span>${i18n.global.t(translationKey)}</span></div><div class="think-content">${content}</div></div>`
  })

  // Handle line breaks to ensure correct Markdown line break behavior:
  // 1. Preserve line breaks within code blocks
  // 2. Add two spaces at the end of single line breaks in normal text (non-code blocks)
  // 3. Retain consecutive line breaks (empty lines) for creating new paragraphs

  // Use Map to store placeholders for better performance and cleaner restoration
  const blocks = new Map()
  let counter = 0

  // Create placeholder for special content (code blocks, math formulas)
  const createPlaceholder = (content, prefix) => {
    const id = `___${prefix}_${counter++}___`
    blocks.set(id, content)
    return id
  }

  // Protect special content by replacing them with placeholders
  content = content.replace(CODE_BLOCK_REGEX, match => createPlaceholder(match, 'CODE'))

  // Add two spaces at the end of non-empty lines for soft line breaks, but retain consecutive line breaks for paragraph separation
  content = content.replace(LINE_BREAK_REGEX, '$1  \n')

  // Replace all <think>...</think> blocks with placeholders
  const thinkBlocks = new Map()
  let thinkCounter = 0
  content = content.replace(THINK_CONTENT_REGEX, match => {
    const id = `___THINK_${thinkCounter++}___`
    thinkBlocks.set(id, match)
    return id
  })

  // Process math formulas before markdown parsing
  if (content.includes('$$')) {
    // Process block math formulas
    content = content.replace(MATH_BLOCK_REGEX, (_match, formula) => {
      // Handle Chinese characters
      if (CHINESE_CHARS_REGEX.test(formula)) {
        formula = formula.replace(CHINESE_CHARS_GROUP_REGEX, '\\text{$1}')
      }
      return `<div class="katex katex-block" data-formula="${encodeURIComponent(formula)}"></div>`
    })

    // 在 markdown 文档解析单`$`包裹的共识非常容易识别错误，所以还是不要支持行内公式好。
    //
    // Parsing single `$` wrapped formulas in markdown documents is very prone to recognition errors, so inline formulas are not supported.
    //
    // Process inline math formulas
    // content = content.replace(MATH_INLINE_REGEX, (_match, formula) => {
    //   // Handle Chinese characters
    //   if (CHINESE_CHARS_REGEX.test(formula)) {
    //     formula = formula.replace(CHINESE_CHARS_GROUP_REGEX, '\\text{$1}')
    //   }
    //   return `<span class="katex katex-inline" data-formula="${encodeURIComponent(formula)}"></span>`
    // })
  }

  // Restore all protected content in a single pass
  content = content.replace(
    PLACEHOLDER_RESTORE_REGEX,
    match => blocks.get(match) || thinkBlocks.get(match)
  )

  // Replace strings wrapped with ``` to ```\n$1\n``` and trim leading and trailing spaces from $1
  // content = content.replace(BLOCK_CODE_REGEX, (_match, p1, p2) => {
  //   return `\n\`\`\`${p1?.trim() || 'txt'}\n${p2?.trim() || ''}\n\`\`\`\n`
  // })

  if (toolCalls) {
    content = createToolCallHtml(content,toolCalls)
  }

  const renderer = new marked.Renderer()

  renderer.code = ev => {
    let lang = ev.lang?.toLowerCase() || ''
    switch (lang) {
      case 'sqlite':
      case 'mysql':
      case 'pgsql':
      case 'postgres':
        lang = 'sql'
        break
      case 'log':
        lang = 'text'
        break
      case 'vue':
        lang = 'html'
        break
    }
    if (lang === 'mermaid') {
      return `<div class="svg-container mermaid" data-content="${encodeURIComponent(ev.text)}"><div class="generating-svg"><i class="cs cs-loading cs-spin"></i>${i18n.global.t('chat.generatingDiagram')}</div></div>`
    } else if (lang === 'mindmap' || lang === 'markmap') {
      return `<div class="svg-container markmap" data-content="${encodeURIComponent(ev.text)}"><div class="generating-svg"><i class="cs cs-loading cs-spin"></i>${i18n.global.t('chat.generatingMindmap')}</div></div>`
    }
    return `<pre><code class="language-${lang}">${htmlspecialchars(ev.text)}</code></pre>`
  }

  return marked(content, { renderer })
}

/**
 * Handle chat message from backend stream
 * This function extracts the common logic from Index.vue and Assistant.vue handleChatMessage
 *
 * @param {Object} payload - The payload from chat_stream event
 * @param {Object} chatStateRef - The chat state ref object (reactive ref)
 * @param {Object} refs - Object containing reactive references
 * @param {Function} refs.currentAssistantMessage - Ref for current assistant message
 * @param {Function} refs.chatErrorMessage - Ref for chat error message
 * @param {Function} refs.isChatting - Ref for chatting state
 * @param {Function} onComplete - Optional callback when message is completed
 * @returns {boolean} - Returns true if message is done, false otherwise
 */
export const handleChatMessage = (payload, chatStateRef, refs, onComplete) => {
  let isDone = false
  const chatState = chatStateRef.value
  chatState.isReasoning = payload?.type === 'reasoning'

  switch (payload?.type) {
    case 'step':
      refs.currentAssistantMessage.value = payload?.chunk || ''
      return false
    case 'reference':
      if (payload?.chunk) {
        console.log('reference', payload?.chunk)
        try {
          if (typeof payload?.chunk === 'string') {
            const parsedChunk = JSON.parse(payload?.chunk || '[]')
            if (Array.isArray(parsedChunk)) {
              chatState.reference.push(...parsedChunk)
            } else {
              console.error('Expected an array but got:', typeof parsedChunk)
            }
          } else {
            if (payload.chunk) {
              chatState.reference.push(...payload.chunk)
            }
          }
        } catch (e) {
          console.error('error on parse reference:', e)
          console.log('chunk', payload?.chunk)
        }
      }
      break
    case 'reasoning':
      chatState.reasoning += payload?.chunk || ''
      break
    case 'error':
      refs.chatErrorMessage.value = payload?.chunk || ''
      isDone = true
      break
    case 'finished':
      isDone = true
      chatState.message += payload?.chunk || ''
      break
    case 'text':
      chatState.message += payload?.chunk || ''
      // handle deepseek-r1 reasoning flag `<think></think>`
      if (chatState.message.startsWith('<think>') && chatState.message.includes('</think>')) {
        const messages = chatState.message.split('</think>')
        chatState.reasoning = messages[0].replace('<think>', '').trim()
        chatState.message = messages[1].trim()
      }
      break

    case 'toolCalls':
      chatState.message += '\n<!--[ToolCalls]-->\n'
      break

    case 'toolResults':
      if (typeof payload?.chunk === 'string') {
        const parsedChunk = JSON.parse(payload?.chunk || '[]')
        if (Array.isArray(parsedChunk)) {
          chatState.toolCall.push(...parsedChunk)
        } else {
          console.error('Expected an array but got:', typeof parsedChunk)
        }
      } else {
        if (payload?.chunk) {
          chatState.toolCall.push(...payload.chunk)
        }
      }
      break

    case 'log':
      chatState.log.push(payload?.chunk || '')
      break
    case 'plan':
      if (payload?.chunk) {
        try {
          const plan = JSON.parse(payload?.chunk || '[]')
          chatState.plan = Array.isArray(plan) ? [...plan] : []
        } catch (e) {
          console.log('error on parse plan:', e)
          console.log('chunk', payload?.chunk)
        }
      }
      break
    default:
      console.warn('Unknown message type:', payload?.type, payload?.chunk)
      break
  }

  // Update current assistant message
  refs.currentAssistantMessage.value = chatState.message || ''

  // Handle completion
  if (isDone) {
    refs.isChatting.value = false
    if (onComplete && typeof onComplete === 'function') {
      onComplete(payload, chatState)
    }
  }

  return isDone
}

const createToolCallHtml = (
		content,
		toolCalls,
	)=> {
  toolCalls.forEach(call => {
    const functionName = call?.function?.name
    if (functionName) {
      let tools = '<div class="chat-tool-calls">'
      if (functionName.includes('__MCP__')) {
        const names = functionName.split('__MCP__')
        if (names.length === 2) {
          tools += `<div class="tool-name"><span>${i18n.global.t('chat.mcpCall')} ${htmlspecialchars(names[0])}::${htmlspecialchars(names[1])}</span></div>`
        } else {
          tools += `<div class="tool-name"><span>${i18n.global.t('chat.toolCall')} ${htmlspecialchars(functionName)}</span></div>`
        }
      } else {
        tools += `<div class="tool-name"><span>${i18n.global.t('chat.toolCall')} ${htmlspecialchars(functionName)}</span></div>`
      }
      const result =
        typeof call.result === 'string' ? call.result : JSON.stringify(call.result, null, 2)
      // Escape HTML in arguments and result to prevent XSS and highlight.js warnings
      const escapedArguments = htmlspecialchars(call.function.arguments || '')
      // const escapedResult = htmlspecialchars(result || '')

      tools += `<div class="tool-codes" style="display:none;">
      <div class="tool-code"><h3>📝 ${i18n.global.t('chat.toolArgs')}</h3>
         <pre><code class="language-json">${escapedArguments}</code></pre>
        </div>
        <div class="tool-code"><h3>🎯 ${i18n.global.t('chat.toolResult')}</h3>
          <code data-result="${encodeURIComponent(result)}" class="tool-results"></code>
        </div>
        </div>
      </div>`

      content = content.replace('<!--[ToolCalls]-->', tools)
    }
  })
  return content
}
