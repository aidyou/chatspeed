/**
 * @file This script is injected into a webview to perform scraping tasks.
 * It supports two modes:
 *  1. Schema-based scraping: Extracts data based on a provided JSON configuration.
 *  2. Generic scraping: Extracts the main content of a page as Markdown.
 */

// Flag to prevent multiple calls to sendScrapeResult
let hasSentResult = false

function sendScrapeResult(data) {
  // Prevent multiple calls
  if (hasSentResult) {
    logger.debug('sendScrapeResult called again, ignoring')
    return
  }

  hasSentResult = true
  const ev = `scrape_result${windowLabel ? `_${windowLabel}` : ''}`
  logger.debug(`send ${ev} , data: `, data?.success || data?.error)
  sendEvent(ev, data)
}

;(() => {
  const urls = [
    'https://www.bing.com/ck/',
    'https://www.so.com/link',
    'https://sogou.com/link',
    'https://www.sogou.com/link'
  ]
  const isRedirectUrl = () => {
    return urls.some(url => window.location.href.startsWith(url))
  }

  const isFinalHttp = () =>
    !!window.location.href &&
    (window.location.href.startsWith('http://') || window.location.href.startsWith('https://')) &&
    !isRedirectUrl()

  // Helper function to wait for the Tauri API to be available
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
      case 'attribute': {
        const attrValue = target.getAttribute(field.attribute)
        const urlAttributes = ['href', 'src', 'srcset', 'data-src', 'poster', 'action']
        if (urlAttributes.includes(field.attribute)) {
          return target[field.attribute] || attrValue // Use property first, fall back to attribute
        }
        return attrValue
      }
      case 'html':
        return target.innerHTML?.trim()
      case 'markdown': {
        const turndownService = new TurndownService()
        return turndownService.turndown(baseCleanForMarkdown(target))
      }
      case 'text':
        return formatText(target.innerText)
      default:
        return formatText(target.innerText)
    }
  }

  /**
   * Performs scraping based on a provided schema configuration.
   * @param {object} config - The scraper configuration.
   */
  async function scrapeWithSchema(config) {
    try {
      if (!config.selectors?.base_selector) {
        if (!hasSentResult) {
          return sendScrapeResult({
            error: 'selectors.base_selector is required'
          })
        }
        return
      }
      console.log('base_selector:', config.selectors.base_selector)

      if (config.config?.wait_for) {
        console.log('wait_for:', config.config.wait_for)
        try {
          await waitForElement(config.config.wait_for, config.config.wait_timeout || 10000)
        } catch (error) {
          if (!hasSentResult) {
            sendScrapeResult({
              error: `Wait for element failed: ${error.message}`
            })
          }
          return
        }
      }

      const results = []

      const maxTry = 3
      let tryCount = 0
      while (tryCount < maxTry) {
        const elements = document.querySelectorAll(config.selectors.base_selector)
        if (!elements.length) {
          tryCount++
          await new Promise(resolve => setTimeout(resolve, 1000 * 2 ** tryCount))
          continue
        }

        elements.forEach(element => {
          console.log('element:', element)
          const item = {}
          let hasSatisfy = true
          config.selectors.fields.forEach(field => {
            item[field.name] = extractField(element, field)
            if (field.required && !item[field.name]) {
              hasSatisfy = false
            }
          })
          if (hasSatisfy) {
            results.push(item)
          }
        })
        break
      }
      console.info('scrape with schema result:', results?.length)
      if (!hasSentResult) {
        // Empty search results are valid, not an error
        sendScrapeResult({ success: JSON.stringify(results) })
      }
    } catch (error) {
      logger.error('Schema scraping failed:', error)
      if (!hasSentResult) {
        sendScrapeResult({ error: `Schema scraping failed: ${error.message}` })
      }
    }
  }

  /**
   * Performs generic content extraction, converting the main body to Markdown.
   */
  async function scrapeGeneric(generic_content_rule = {}) {
    const maxTry = 3
    let lastError = 'Unknown error'

    for (let tryCount = 0; tryCount < maxTry; tryCount++) {
      try {
        const title = document.title || document.querySelector('h1, h2, h3')?.innerText || ''
        const content = document.body?.cloneNode(true)

        if (!content) {
          throw new Error('Content scraping failed, cannot find body child elements')
        }

        // Remove non-content elements
        const selectorToRemove =
          'script, style, noscript, form, iframe, frame, object, embed, video, audio, link, svg, canvas, meta, head, base, template, symbol, button, select, textarea, datalist, dialog, source, picture, track, map'
        content.querySelectorAll(selectorToRemove).forEach(el => {
          el.remove()
        })

        if (generic_content_rule.format === 'text') {
          content.querySelectorAll('nav, aside, header, footer').forEach(el => {
            el.remove()
          })

          const textContent = formatText(content?.innerText || '')
          if (textContent.length < 50) {
            throw new Error(`Scraped text content is too short (${textContent.length} chars).`)
          }
          if (!hasSentResult) {
            sendScrapeResult({
              success: JSON.stringify({
                title,
                content: textContent,
                url: window.location.href
              })
            })
          }
          return // Exit successfully
        }

        // remove image
        if (!generic_content_rule.keep_image) {
          content.querySelectorAll('img')?.forEach(el => {
            el.remove()
          })
        } else {
          content.querySelectorAll('img')?.forEach(el => {
            const src = el.src || el.getAttribute('src')
            if (!src || src.startsWith('data:') || src.startsWith('blob:')) {
              el.remove()
              return
            }
            const newImg = document.createElement('img')
            newImg.src = src
            newImg.alt = el.alt || ''
            el.replaceWith(newImg)
          })
        }

        if (generic_content_rule.keep_link) {
          // format link
          const links = content.querySelectorAll('a')
          links?.forEach(link => {
            const href = link.href
            if (href === '' || href.startsWith('#') || href.startsWith('javascript:')) {
              link.removeAttribute('href')
            } else {
              link.setAttribute('href', href)
            }
            link.innerText = link.innerText.trim().replace(/\n+/g, ' ')
          })
        } else {
          // Remove links
          const links = content.querySelectorAll('a')
          links?.forEach(link => {
            const textNode = document.createTextNode(link.innerText.replace(/\n+/g, ' ').trim())
            link.replaceWith(textNode)
          })

          content.querySelectorAll('nav, aside, header, footer').forEach(el => {
            el.remove()
          })
        }

        content.querySelectorAll(selectorToRemove).forEach(el => {
          el.remove()
        })

        const turndownService = new TurndownService({
          headingStyle: 'atx',
          hr: '---',
          bulletListMarker: '*',
          codeBlockStyle: 'fenced',
          emDelimiter: '_'
        })
        let markdown = turndownService.turndown(content)

        // Basic cleanup
        markdown = markdown
          .replace(/\\\[(.*?)\]/g, '[$1]') // Fix escaped brackets
          .replace(/\n{3,}/g, '\n\n') // Collapse multiple newlines
          .trim()

        // Check content length after processing
        if (markdown.length < 50) {
          throw new Error(`Scraped markdown content is too short (${markdown.length} chars).`)
        }

        const result = { title, content: markdown, url: window.location.href }
        if (!hasSentResult) {
          sendScrapeResult({ success: JSON.stringify(result) })
        }
        return // Exit successfully
      } catch (error) {
        lastError = error.message
        logger.error(`Scrape attempt ${tryCount + 1} failed: ${lastError}`)
        if (tryCount < maxTry - 1) {
          const delay = 1000 * 2 ** tryCount // 1s, 2s
          logger.debug(`Waiting ${delay}ms for next retry...`)
          await new Promise(resolve => setTimeout(resolve, delay))
        }
      }
    }

    // If all retries fail
    logger.error(`Generic scraping failed after ${maxTry} attempts.`)
    if (!hasSentResult) {
      sendScrapeResult({
        error: `Generic scraping failed after ${maxTry} attempts: ${lastError}`
      })
    }
  }

  /**
   * Main entry point for the scraping logic.
   * @param {object | null} config - The configuration object, or null for generic scraping.
   */
  window.performScrape = async (config, generic_content_rule) => {
    // Reset the flag at the start of each scrape operation
    hasSentResult = false
    logger.debug('performScrape called, config:', JSON.stringify(config))
    logger.debug('generic_content_rule:', JSON.stringify(generic_content_rule))

    try {
      // Wait for TurndownService to be available
      await new Promise((resolve, reject) => {
        const interval = setInterval(() => {
          if (typeof TurndownService !== 'undefined' && isFinalHttp()) {
            clearInterval(interval)
            logger.debug('TurndownService is ready, url:', window.location.href)
            resolve()
          }
        }, 100)

        setTimeout(() => {
          clearInterval(interval)
          reject(new Error('Timeout waiting for TurndownService'))
        }, 3000) // 5 second timeout
      })

      // Now that TurndownService is ready, proceed
      if (config) {
        await scrapeWithSchema(config)
      } else {
        await scrapeGeneric(generic_content_rule || {})
      }
    } catch (error) {
      // Uniformly catch any errors that may occur during the await process
      logger.error('performScrape failed:', error)
      // Only send result if we haven't already sent one
      if (!hasSentResult) {
        sendScrapeResult({ error: `performScrape failed: ${error.message}` })
      }
    }
  }
  ;(async () => {
    try {
      const event = await waitForTauriAPI()
      if (!event) {
        logger.warn('Tauri API unavailable, unable to send event')
        return
      }

      const maxTry = 3
      let tryCount = 0
      while (!isFinalHttp() && tryCount < maxTry) {
        tryCount++
        await new Promise(resolve => setTimeout(resolve, 1000 * 2 ** tryCount))
      }

      if (isFinalHttp()) {
        logger.info('emit page loaded, url:', window.location.href)
        event.emit(`page_loaded_${windowLabel}`)
      } else {
        logger.warn('Failed to get a valid URL after multiple retries.')
      }
    } catch (error) {
      logger.error('Error during page loaded event handling:', error)
    }
  })()
})()
