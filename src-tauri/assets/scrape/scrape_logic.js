/**
 * @file This script is injected into a webview to perform scraping tasks.
 * It supports two modes:
 *  1. Schema-based scraping: Extracts data based on a provided JSON configuration.
 *  2. Generic scraping: Extracts the main content of a page as Markdown.
 */

;(function () {
  // 等待Tauri API可用的辅助函数
  function waitForTauriAPI() {
    return new Promise(resolve => {
      if (window.__TAURI__?.event) {
        logger.debug('Tauri api available')

        resolve(window.__TAURI__.event)
      } else {
        const checkInterval = setInterval(() => {
          if (window.__TAURI__?.event) {
            logger.debug('Tauri api available')

            clearInterval(checkInterval)
            resolve(window.__TAURI__.event)
          }
        }, 50)

        setTimeout(() => {
          logger.warn('Tauri api not available after 5 secs.')
          clearInterval(checkInterval)
          resolve(null)
        }, 5000)
      }
    })
  }

  // 初始化事件发射器
  let eventEmitter = null

  /**
   * Waits for an element matching the selector to appear in the DOM.
   * @param {string} selector - The CSS selector to wait for.
   * @param {number} timeout - Maximum time to wait in milliseconds.
   * @returns {Promise<Element>} A promise that resolves with the element or rejects on timeout.
   */
  function waitForElement(selector, timeout = 10000) {
    return new Promise((resolve, reject) => {
      const element = document.querySelector(selector)
      if (element) {
        return resolve(element)
      }

      const observer = new MutationObserver(() => {
        const el = document.querySelector(selector)
        if (el) {
          observer.disconnect()
          resolve(el)
        }
      })

      observer.observe(document.body, {
        childList: true,
        subtree: true
      })

      setTimeout(() => {
        observer.disconnect()
        reject(new Error(`Timeout waiting for element: ${selector}`))
      }, timeout)
    })
  }

  /**
   * Extracts data from a single element based on a field configuration.
   * @param {Element} element - The parent element to search within.
   * @param {object} field - The field configuration object.
   * @returns {string|null} The extracted data.
   */
  function extractField(element, field) {
    const target = element.querySelector(field.selector)
    if (!target) return null

    switch (field.type) {
      case 'text':
        return target.innerText
      case 'attribute':
        return target.getAttribute(field.attribute)
      case 'html':
        return target.innerHTML
      case 'markdown':
        const turndownService = new TurndownService()
        return turndownService.turndown(target)
      default:
        return null
    }
  }

  /**
   * Performs scraping based on a provided schema configuration.
   * @param {object} config - The scraper configuration.
   */
  async function scrapeWithSchema(config) {
    try {
      await waitForElement(config.selectors.baseSelector, config.config.waitForTimeout || 10000)

      const results = []
      const elements = document.querySelectorAll(config.selectors.baseSelector)

      elements.forEach(element => {
        const item = {}
        config.selectors.fields.forEach(field => {
          item[field.name] = extractField(element, field)
        })
        results.push(item)
      })
      logger.info('scrape with schema result:', results)
      sendScrapeResult({ success: JSON.stringify(results) })
    } catch (error) {
      logger.error('Schema scraping failed:', error)
      sendScrapeResult({ error: `Schema scraping failed: ${error.message}` })
    }
  }

  /**
   * Performs generic content extraction, converting the main body to Markdown.
   */
  function scrapeGeneric() {
    try {
      const turndownService = new TurndownService({
        headingStyle: 'atx',
        hr: '---',
        bulletListMarker: '*',
        codeBlockStyle: 'fenced',
        emDelimiter: '_'
      })

      let title = document.title || document.querySelector('h1, h2, h3')?.innerText || ''

      const content = document.body?.cloneNode(true)

      // Remove non-content elements
      const elementsToRemove = content.querySelectorAll(
        'script, style, noscript, nav, footer, header, aside, form, iframe, frame, video, audio, link'
      )
      elementsToRemove.forEach(el => el.remove())

      let markdown = turndownService.turndown(content)

      // Basic cleanup
      markdown = markdown
        .replace(/\\\\[(.*?)\\\\]/g, '[$1]') // Fix escaped brackets
        .replace(/\n{3,}/g, '\n\n') // Collapse multiple newlines
        .trim()
      logger.info('scrape generic title:', title,", content:",markdown)
      const result = { title, content: markdown }
      sendScrapeResult({ success: JSON.stringify(result) })
    } catch (error) {
      logger.error('scrape generic failed:', error)
      sendScrapeResult({ error: `Generic scraping failed: ${error.message}` })
    }
  }

  /**
   * Main entry point for the scraping logic.
   * @param {object | null} config - The configuration object, or null for generic scraping.
   */
  window.performScrape = function (config) {
    logger.debug('performScrape called, config:', config)
    // The TurndownService might not be loaded immediately.
    // We wait for it to be available before proceeding.
    const checkTurndown = setInterval(() => {
      if (typeof TurndownService !== 'undefined') {
        clearInterval(checkTurndown)
        if (config) {
          scrapeWithSchema(config)
        } else {
          scrapeGeneric()
        }
      }
    }, 100)
  }

  // 初始化并通知后端
  waitForTauriAPI().then(event => {
    eventEmitter = event
    if (eventEmitter) {
      logger.info('emit page loaded')
      eventEmitter.emit('page_loaded')
    } else {
      logger.warn('Tauri API不可用，无法发送事件')
    }
  })
})()

// 全局变量，用于跟踪采集是否已完成
let scrapeCompleted = false

// 修改全局消息发送函数，添加采集完成标记
function sendScrapeResult(data) {
  // 如果采集已完成，设置标记
  if (data?.success) {
    scrapeCompleted = true
  }

  logger.debug('sendScrapeResult called, data: ', data?.success || data?.error)
  sendEvent("scrape_result",data)
}

// 添加全局错误处理
window.addEventListener('error', event => {
  logger.error('Global error:', event.error)
  // 只有在采集完成后才发送错误消息给Rust端
  if (scrapeCompleted) {
    sendScrapeResult({ error: 'Global error occurred' })
  }
  // 阻止错误继续传播
  event.preventDefault()
  event.stopPropagation()
  return true
})

window.addEventListener('unhandledrejection', event => {
  logger.error('Unhandled promise rejection:', event.reason)
  // 只有在采集完成后才发送错误消息给Rust端
  if (scrapeCompleted) {
    sendScrapeResult({ error: 'Unhandled promise rejection' })
  }
  // 阻止错误继续传播
  event.preventDefault()
  event.stopPropagation()
})
