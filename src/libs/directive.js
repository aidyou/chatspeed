/**
 * This file contains custom Vue directives for enhancing code blocks and links within the application.
 *
 * The directives include:
 * 1. `highlight`: Automatically highlights code blocks using the Highlight.js library
 * 2. `link`: Opens URLs in the default browser
 * 3. `katex`: Renders LaTeX formulas within paragraphs
 * 4. `mermaid`: Renders Mermaid diagrams and Markmap mindmaps
 * 5. `think`: Toggles the visibility of think content
 */

import { invoke } from '@tauri-apps/api/core'
import hljs from 'highlight.js'
import katex from 'katex'
import i18n from '@/i18n'
import mermaid from 'mermaid'
import { Markmap } from 'markmap-view'
import { Transformer } from 'markmap-lib'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'

// =================================================
// Mermaid Diagram Processing
// =================================================

// Mermaid constants
const MERMAID_CONFIG = {
  startOnLoad: false,
  theme: 'default',
  securityLevel: 'loose',
  fontFamily: 'var(--cs-font-family)',
  fontSize: 14,
  flowchart: {
    htmlLabels: true,
    curve: 'linear',
  },
  suppressErrorRendering: true,
  logLevel: 5,
  deterministicIds: true,
  er: { useMaxWidth: false },
  sequence: { useMaxWidth: false },
  gantt: { useMaxWidth: false },
  journey: { useMaxWidth: false }
}

// Initialize mermaid configuration
mermaid.initialize(MERMAID_CONFIG)

/**
 * Processes and normalizes Mermaid diagram syntax
 * @param {string} content - Raw Mermaid diagram content
 * @returns {string} - Normalized Mermaid diagram content
 */
const processMermaidContent = (content) => {
  content = content.trim()

  // Ensure proper graph declaration
  if (!content.startsWith('graph') &&
    !content.startsWith('sequenceDiagram') &&
    !content.startsWith('classDiagram')) {
    content = 'graph TD\n' + content
  }

  // Normalize arrow syntax
  return content
    .replace(/---\s*(\w+)\s*-->/g, '-->|$1|')
    .replace(/--\s*(\w+)\s*-->/g, '-->|$1|')
    .replace(/(\w+)\s+-->\s*(\w+)/g, '$1-->$2')
    .replace(/(\w+)\s*-->\s*$/gm, '$1')
    .replace(/^\s*[\r\n]/gm, '')
    .replace(/\w+\s*-->\s*\|[^|]+\|\s*$/g, '')
}

// =================================================
// Markmap Processing
// =================================================

// Markmap layout configuration
const MARKMAP_LAYOUT_CONFIG = {
  duration: 300,
  nodeMinHeight: 16,
  paddingX: 8,
  maxWidth: 300,
  spacingHorizontal: 40,
  spacingVertical: 15,
  autoFit: true
}

// Base layout parameters for mindmap
const MINDMAP_LAYOUT_PARAMS = {
  minHeight: 200,    // Minimum height of the mindmap
  maxHeight: 800,    // Maximum height constraint
  nodeHeight: 20,    // Height of each node
  levelSpacing: 15,  // Vertical spacing between levels
  nodeSpacing: 10,   // Vertical spacing between nodes at same level
  padding: 10        // Padding at top and bottom
}

// Initialize transformers
const transformer = new Transformer()

/**
 * Calculates dimensions for a mindmap based on its node structure
 * @param {Object} root - Root node of the mindmap
 * @returns {Object} Object containing:
 *   - maxDepth: Maximum depth of the tree
 *   - maxWidth: Maximum width (number of nodes) at any level
 *   - totalNodes: Total number of nodes in the tree
 *   - levelWidths: Map of depth to number of nodes at that depth
 */
function calculateMapDimensions(root) {
  let maxDepth = 0
  let maxWidth = 0
  let levelWidths = new Map()

  // Recursively traverse the tree to calculate dimensions
  function traverse(node, depth = 0) {
    // Count nodes at each depth level
    levelWidths.set(depth, (levelWidths.get(depth) || 0) + 1)
    maxDepth = Math.max(maxDepth, depth)

    // Recursively process child nodes
    if (node.children?.length > 0) {
      node.children.forEach(child => traverse(child, depth + 1))
    }
  }

  traverse(root)
  // Find the maximum width across all levels
  levelWidths.forEach((width) => {
    maxWidth = Math.max(maxWidth, width)
  })

  return {
    maxDepth,
    maxWidth,
    totalNodes: Array.from(levelWidths.values()).reduce((a, b) => a + b, 0),
    levelWidths
  }
}

/**
 * Calculates optimal height for a mindmap based on its structure
 * @param {Object} root - Root node of the mindmap
 * @returns {number} Calculated optimal height in pixels, bounded between minHeight and maxHeight
 */
function calculateOptimalHeight(root) {
  const { maxDepth, levelWidths } = calculateMapDimensions(root)
  const {
    minHeight,
    maxHeight,
    nodeHeight,
    levelSpacing,
    nodeSpacing,
    padding
  } = MINDMAP_LAYOUT_PARAMS

  // Calculate maximum height needed for any single level
  let maxLevelHeight = 0
  levelWidths.forEach((nodesCount) => {
    // Height for each level = number of nodes * (node height + spacing between nodes)
    const levelHeight = nodesCount * (nodeHeight + nodeSpacing)
    maxLevelHeight = Math.max(maxLevelHeight, levelHeight)
  })

  // Calculate total content height
  // contentHeight = maximum level height + (spacing between levels * number of levels)
  const contentHeight = maxLevelHeight + (levelSpacing * maxDepth)
  // Add padding to top and bottom
  const totalHeight = contentHeight + (padding * 2)

  // Return height bounded between minHeight and maxHeight
  return Math.min(Math.max(minHeight, totalHeight), maxHeight)
}

// CSS variable mapping for Markmap styling with fallback values
const CSS_VAR_MAP = {
  '--markmap-max-width': { value: 'none', fallback: 'none' },
  '--markmap-bg-color': { value: '--cs-bg-color', fallback: '#ffffff' },
  '--markmap-a-color': { value: '--cs-color-primary', fallback: '#0097e6' },
  '--markmap-a-hover-color': { value: '--cs-color-primary-light', fallback: '#00a8ff' },
  '--markmap-code-bg': { value: 'transparent', fallback: '#f0f0f0' },
  '--markmap-code-color': { value: '--cs-text-color-primary', fallback: '#555555' },
  '--markmap-highlight-bg': { value: '#ffeaa7', fallback: '#ffeaa7' },
  '--markmap-table-border': { value: '1px solid currentColor', fallback: '1px solid currentColor' },
  '--markmap-font': { value: '300 16px/20px sans-serif', fallback: '300 16px/20px sans-serif' },
  '--markmap-circle-open-bg': { value: '--cs-bg-color', fallback: '#ffffff' },
  '--markmap-text-color': { value: '--cs-text-color-primary', fallback: '#333333' }
}

/**
 * Resolves CSS variables in the SVG content
 * @param {Element} svg - The SVG element
 */
function resolveCssVariables(svg) {
  // Get computed styles from root element
  const computedStyle = getComputedStyle(document.documentElement)
  console.log('computedStyle', computedStyle)

  // Resolve CSS variables with type-specific fallback values
  const resolvedVars = {}
  Object.entries(CSS_VAR_MAP).forEach(([key, { value, fallback }]) => {
    resolvedVars[key] = value.startsWith('--')
      ? computedStyle.getPropertyValue(value).trim() || fallback
      : value
  })

  // Process style tag
  const styleElement = svg.querySelector('style')
  if (styleElement) {
    let styleContent = styleElement.textContent

    // Remove existing markmap class styles
    styleContent = styleContent.replace(/.markmap\s*{[^}]*}/, '')

    // Add new styles with resolved variables
    const newStyles = `
      .markmap {
        font: ${resolvedVars['--markmap-font']};
        color: ${resolvedVars['--markmap-text-color']};
        background-color: ${resolvedVars['--markmap-bg-color']};
      }
      .markmap-link {
        fill: none;
      }
      .markmap-node > circle {
        cursor: pointer;
        fill: ${resolvedVars['--markmap-circle-open-bg']};
      }
      .markmap-foreign {
        display: inline-block;
      }
      .markmap-foreign p {
        margin: 0;
      }
      .markmap-foreign a {
        color: ${resolvedVars['--markmap-a-color']};
      }
      .markmap-foreign a:hover {
        color: ${resolvedVars['--markmap-a-hover-color']};
      }
      .markmap-foreign code {
        padding: 0 !important;
        margin: 0 !important;
        font-size: calc(1em - 2px);
        color: ${resolvedVars['--markmap-code-color']};
        background-color: transparent !important;
        border-radius: 2px;
      }
    `
    styleElement.textContent = styleContent + newStyles
  }

  // Ensure all text elements are visible
  svg.querySelectorAll('.markmap-foreign').forEach(el => {
    el.style.opacity = '1'
  })

  return svg
}

/**
 * Creates a download button for SVG diagrams
 * @param {HTMLElement} container - The container element containing the SVG
 * @param {string} type - The type of diagram ('mermaid' or 'markmap')
 * @returns {HTMLElement} The download button element
 */
function createDownloadButton(container, type) {
  const name = type === 'mermaid' ? i18n.global.t('common.diagram') : i18n.global.t('common.mindmap')
  const titleBar = document.createElement('div')
  titleBar.classList.add('code-title-bar')
  const title = document.createElement('span')
  title.innerText = name
  titleBar.appendChild(title)

  const btnContainer = document.createElement('div')
  btnContainer.classList.add('btn-container')

  // SVG download button
  const svgButton = document.createElement('i')
  svgButton.classList.add('cs', 'cs-download', 'diagram-download-btn')
  svgButton.innerText = i18n.global.t('common.downloadSvg')
  svgButton.onclick = async () => {
    try {
      const filePath = await save({
        filters: [{
          name: 'SVG Image',
          extensions: ['svg']
        }],
        defaultPath: `${name}-${new Date().toISOString().replace(/[-:T]/g, '').slice(0, 14)}.svg`
      })

      if (filePath) {
        const svg = container.querySelector('svg')
        const clonedSvg = svg.cloneNode(true)

        // Set SVG namespace attributes
        clonedSvg.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
        clonedSvg.setAttribute('xmlns:xlink', 'http://www.w3.org/1999/xlink')

        // Restore initial transform for markmap
        const initialTransform = svg.getAttribute('data-initial-transform')
        if (initialTransform) {
          const gElement = clonedSvg.querySelector('g')
          if (gElement) {
            gElement.setAttribute('transform', initialTransform)
          }
        }

        // Process CSS variables
        resolveCssVariables(clonedSvg)

        // Get dimensions from the original SVG
        const bbox = svg.getBoundingClientRect()
        clonedSvg.setAttribute('width', bbox.width)
        clonedSvg.setAttribute('height', bbox.height)

        const encoder = new TextEncoder()
        const data = encoder.encode(clonedSvg.outerHTML)
        await writeFile(filePath, data)
      }
    } catch (error) {
      console.error('Failed to save SVG:', error)
    }
  }

  // PNG download button
  // const pngButton = document.createElement('i')
  // pngButton.classList.add('cs', 'cs-image', 'diagram-download-btn')
  // pngButton.innerText = i18n.global.t('common.downloadPng')
  // pngButton.onclick = async () => {
  //   try {
  //     const svg = container.querySelector('svg')
  //     const isMarkmap = container.closest('.markmap') !== null

  //     // 获取 SVG 的尺寸和位置信息
  //     let width, height, transformMatrix
  //     if (isMarkmap) {
  //       // 对于 markmap，需要考虑 SVG 的变换
  //       const g = svg.querySelector('g')
  //       if (g) {
  //         // 保存当前变换
  //         const originalTransform = g.getAttribute('transform')

  //         // 临时移除变换以获取完整尺寸
  //         g.removeAttribute('transform')
  //         const bbox = g.getBBox()
  //         width = Math.ceil(bbox.width + bbox.x * 2)  // 添加边距
  //         height = Math.ceil(bbox.height + bbox.y * 2)

  //         // 恢复原始变换
  //         if (originalTransform) {
  //           g.setAttribute('transform', originalTransform)
  //         }
  //       } else {
  //         // 降级处理
  //         const bbox = svg.getBBox()
  //         width = Math.ceil(bbox.width)
  //         height = Math.ceil(bbox.height)
  //       }
  //     } else {
  //       // 对于 mermaid，使用之前的逻辑
  //       const viewBox = svg.getAttribute('viewBox')?.split(' ').map(Number)
  //       if (viewBox?.length === 4) {
  //         width = viewBox[2]
  //         height = viewBox[3]
  //       } else {
  //         const bbox = svg.getBBox()
  //         width = Math.ceil(bbox.width)
  //         height = Math.ceil(bbox.height)
  //       }
  //     }

  //     // Create temporary img element
  //     const img = new Image()
  //     // 克隆 SVG 并设置尺寸
  //     const clonedSvg = svg.cloneNode(true)
  //     clonedSvg.setAttribute('width', width)
  //     clonedSvg.setAttribute('height', height)

  //     if (isMarkmap) {
  //       // 重置 markmap 的变换，确保显示完整内容
  //       const g = clonedSvg.querySelector('g')
  //       if (g) {
  //         g.removeAttribute('transform')
  //         // 添加一个居中的变换
  //         const bbox = g.getBBox()
  //         const centerX = (width - bbox.width) / 2 - bbox.x
  //         const centerY = (height - bbox.height) / 2 - bbox.y
  //         g.setAttribute('transform', `translate(${centerX},${centerY})`)
  //       }
  //     }

  //     // 处理 CSS 变量
  //     resolveCssVariables(clonedSvg)

  //     const svgData = new XMLSerializer().serializeToString(clonedSvg)
  //     const svgDataUrl = 'data:image/svg+xml;base64,' + btoa(unescape(encodeURIComponent(svgData)))

  //     try {
  //       await new Promise((resolve, reject) => {
  //         img.onload = resolve
  //         img.onerror = reject
  //         img.src = svgDataUrl
  //       })

  //       // Create canvas with SVG's dimensions
  //       const canvas = document.createElement('canvas')
  //       canvas.width = width
  //       canvas.height = height
  //       const ctx = canvas.getContext('2d')

  //       // Set white background
  //       ctx.fillStyle = '#ffffff'
  //       ctx.fillRect(0, 0, canvas.width, canvas.height)

  //       // 绘制图像
  //       ctx.drawImage(img, 0, 0, width, height)

  //       // Convert canvas to blob and save
  //       const blob = await new Promise(resolve => {
  //         canvas.toBlob(resolve, 'image/png')
  //       })

  //       const response = await fetch('http://127.0.0.1:21914/save/png', {
  //         method: 'POST',
  //         headers: {
  //           'Content-Type': 'image/png'
  //         },
  //         body: blob
  //       })

  //       if (!response.ok) {
  //         const errorText = await response.text()
  //         console.error('Server response:', errorText)
  //         throw new Error(`Failed to save PNG: ${errorText}`)
  //       }

  //       const result = await response.text()
  //       console.log('Save result:', result)

  //     } finally {
  //       img.src = ''
  //     }
  //   } catch (error) {
  //     console.error('Failed to save PNG:', error)
  //   }
  // }
  // btnContainer.appendChild(pngButton)

  btnContainer.appendChild(svgButton)
  titleBar.appendChild(btnContainer)
  return titleBar
}

/**
 * Renders both Mermaid diagrams and Markmap mindmaps within a container
 * @param {HTMLElement} el - Container element
 */
async function renderMermaidDiagrams(el) {
  // Render Mermaid diagrams
  const mermaidDiagrams = el.querySelectorAll('.mermaid')
  for (const diagram of mermaidDiagrams) {
    if (diagram.querySelector('svg')) continue

    const content = decodeURIComponent(diagram.dataset.content || '').trim()
    if (!content) continue

    try {
      await mermaid.parse(content)

      const id = `mermaid-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
      const processedContent = processMermaidContent(content)
      const { svg } = await mermaid.render(id, processedContent)

      // Create container for SVG and download button
      const container = document.createElement('div')
      container.classList.add('diagram-container')

      // Add title bar first
      const downloadBtn = createDownloadButton(el, 'mermaid')
      container.appendChild(downloadBtn)

      // Add SVG after title bar
      const svgContainer = document.createElement('div')
      svgContainer.classList.add('diagram-svg-container')
      svgContainer.innerHTML = svg
      container.appendChild(svgContainer)

      diagram.innerHTML = ''
      diagram.appendChild(container)
    } catch (error) {
      console.error('Failed to render mermaid diagram:', error)
      diagram.innerHTML = `<pre class="mermaid-error"><div class="code-title-bar">${i18n.global.t('chat.diagramError')}</div><code>${content}</code></pre>`
    }
  }

  // Render Markmap mindmaps
  const markmapDiagrams = el.querySelectorAll('.markmap')
  for (const diagram of markmapDiagrams) {
    if (diagram.querySelector('svg')) continue

    const content = decodeURIComponent(diagram.dataset.content || '').trim()
    if (!content) continue

    try {
      const container = document.createElement('div')
      container.classList.add('diagram-container')

      const svgContainer = document.createElement('div')
      svgContainer.classList.add('diagram-svg-container')

      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
      svg.style.width = '100%'
      svgContainer.appendChild(svg)
      container.appendChild(svgContainer)

      diagram.innerHTML = ''
      diagram.appendChild(container)

      const { root } = transformer.transform(content)
      const height = calculateOptimalHeight(root)
      svg.style.height = `${height}px`

      // Create Markmap
      Markmap.create(svg, MARKMAP_LAYOUT_CONFIG, root)
      resolveCssVariables(svg)

      // Calculate wait time based on MARKMAP_LAYOUT_CONFIG.duration
      const ANIMATION_BUFFER = 200 // Additional buffer time
      const TRANSFORM_WAIT_TIME = MARKMAP_LAYOUT_CONFIG.duration + ANIMATION_BUFFER

      // Protection timeout to prevent infinite waiting
      let protectedTimer = setTimeout(() => {
        if (!svg.hasAttribute('data-initial-transform')) {
          const gElement = svg.querySelector('g')
          if (gElement) {
            const finalTransform = gElement.getAttribute('transform')
            if (finalTransform) {
              console.debug('Protected timeout: setting final transform', finalTransform)
              svg.setAttribute('data-initial-transform', finalTransform)
            }
          }
          observer?.disconnect()
        }
      }, 2000) // 2 seconds protection timeout

      // Create observer to monitor changes
      const observer = new MutationObserver((mutations) => {
        const gElement = svg.querySelector('g')
        if (gElement) {

          // Wait for animation completion before getting final transform value
          setTimeout(() => {
            const transform = gElement.getAttribute('transform')
            if (transform) {
              if (transform.includes('translate') && transform.includes('scale')) {
                console.debug('Animation completed: setting transform', transform)
                svg.setAttribute('data-initial-transform', transform)
              }
            }
          }, TRANSFORM_WAIT_TIME)

          observer.disconnect()
          if (protectedTimer) {
            clearTimeout(protectedTimer)
          }
        }
      })

      // Start observing the SVG element
      observer.observe(svg, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ['transform'],
        characterData: false
      })


      // Create download button after Markmap rendering
      const downloadBtn = createDownloadButton(el, 'markmap')
      container.insertBefore(downloadBtn, svgContainer)
    } catch (error) {
      console.error('Failed to render markmap:', error)
      diagram.innerHTML = `<pre class="mermaid-error"><div class="code-title-bar">${i18n.global.t('chat.mindmapError')}</div><code>${content}</code></pre>`
    }
  }
}

// =================================================
// Handle code block highlight
// =================================================

// Constants for copy button states
const COPY_BUTTON_CONFIG = {
  markdown: {
    icon: 'cs-copy',
    text: () => i18n.global.t('common.copyMarkdown'),
    successIcon: 'cs-check'
  },
  code: {
    icon: 'cs-copy',
    text: () => i18n.global.t('common.copyCode'),
    successIcon: 'cs-check'
  }
}

/**
 * Creates a copy button with specified options
 * @param {string} iconClass - Icon class for the button
 * @param {string} text - Button text
 * @param {Function} onClick - Click handler
 * @returns {HTMLElement} The created button element
 */
function createCopyButton(iconClass, text, onClick) {
  const button = document.createElement('i')
  button.classList.add('cs', iconClass, 'code-copy-btn')
  button.innerText = text
  button.onclick = onClick
  return button
}

/**
 * Handles the copy action and updates button state
 * @param {HTMLElement} button - The button element
 * @param {string} copyText - Text to copy
 * @param {string} btnTxt - Button text
 * @param {string} iconClass - Original icon class
 */
function handleCopy(button, btnTxt, copyText, iconClass) {
  navigator.clipboard.writeText(copyText).then(() => {
    button.classList.remove(iconClass)
    button.classList.add(COPY_BUTTON_CONFIG.code.successIcon)
    button.innerText = i18n.global.t('common.copied')
    setTimeout(() => {
      button.classList.remove(COPY_BUTTON_CONFIG.code.successIcon)
      button.classList.add(iconClass)
      button.innerText = btnTxt
    }, 3000)
  }).catch(err => {
    console.error('Could not copy text: ', err)
  })
}

/**
 * Creates a title bar for the code block with copy buttons
 * @param {Element} block - The code block element
 */
function createTitleBar(block) {
  const titleBar = document.createElement('div')
  titleBar.classList.add('code-title-bar')

  // Create language label
  const languageLabel = document.createElement('span')
  languageLabel.classList.add('code-language-label')
  const languageClass = block.getAttribute('class')?.split(' ')
    .find(cls => cls.startsWith('language-'))?.replace('language-', '') || ''
  languageLabel.innerText = languageClass

  titleBar.appendChild(languageLabel)

  // Create button container
  const copyBtnContainer = document.createElement('div')
  copyBtnContainer.classList.add('btn-container')

  // Create markdown copy button
  const codeCopyButton = createCopyButton(
    COPY_BUTTON_CONFIG.markdown.icon,
    COPY_BUTTON_CONFIG.markdown.text(),
    () => {
      const copyText = languageClass ?
        '```' + languageClass + '\n' + block.innerText.trim() + '\n```\n' :
        block.innerText.trim()
      handleCopy(
        codeCopyButton,
        COPY_BUTTON_CONFIG.markdown.text(),
        copyText,
        COPY_BUTTON_CONFIG.markdown.icon
      )
    }
  )
  copyBtnContainer.appendChild(codeCopyButton)

  // Create text copy button
  const textCopyButton = createCopyButton(
    COPY_BUTTON_CONFIG.code.icon,
    COPY_BUTTON_CONFIG.code.text(),
    () => {
      handleCopy(
        textCopyButton,
        COPY_BUTTON_CONFIG.code.text(),
        block.innerText.trim(),
        COPY_BUTTON_CONFIG.code.icon
      )
    }
  )
  copyBtnContainer.appendChild(textCopyButton)

  titleBar.appendChild(copyBtnContainer)

  // Set parent element style and insert title bar
  block.parentElement.style.position = 'relative'
  block.parentElement.insertBefore(titleBar, block)
}

// =================================================
// Register directives
// =================================================

// Directive configurations
const DIRECTIVE_CONFIG = {
  highlight: {
    name: 'highlight',
    handlers: {
      mounted: el => {
        el.querySelectorAll('pre code').forEach((block) => {
          hljs.highlightElement(block)
          if (!block.parentElement.querySelector('div')) {
            createTitleBar(block)
          }
        })
      },
      updated: el => {
        el.querySelectorAll('pre code').forEach((block) => {
          if (block?.attributes?.['data-highlighted']?.value === 'yes') {
            return
          }
          hljs.highlightElement(block)
          if (!block.parentElement.querySelector('div')) {
            createTitleBar(block)
          }
        })
      }
    }
  },
  link: {
    name: 'link',
    handlers: {
      mounted: el => {
        const handleClick = async (e) => {
          if (e.target.tagName === 'A') {
            e.preventDefault()
            e.stopPropagation()
            try {
              await invoke('open_url', { url: e.target.href })
            } catch (error) {
              console.error('Failed to open URL:', error)
            }
          }
        }
        el.addEventListener('click', handleClick)
        el._vLinkCleanup = () => el.removeEventListener('click', handleClick)
      },
      unmounted: el => {
        if (el._vLinkCleanup) {
          el._vLinkCleanup()
          delete el._vLinkCleanup
        }
      }
    }
  },
  katex: {
    name: 'katex',
    handlers: {
      mounted: el => {
        el.querySelectorAll('p').forEach((block) => {
          block.innerHTML = block.innerHTML.replace(/\[([^\]]+)\]/g, (_match, formula) => {
            return `<span class="katex">${katex.renderToString(formula, { throwOnError: false })}</span>`
          })
        })
      },
      updated: el => {
        el.querySelectorAll('p').forEach((block) => {
          block.innerHTML = block.innerHTML.replace(/\[([^\]]+)\]/g, (_match, formula) => {
            return `<span class="katex">${katex.renderToString(formula, { throwOnError: false })}</span>`
          })
        })
      }
    }
  },
  mermaid: {
    name: 'mermaid',
    handlers: {
      mounted: renderMermaidDiagrams,
      updated: renderMermaidDiagrams
    }
  },
  think: {
    name: 'think',
    handlers: {
      mounted: el => {
        const title = el.querySelector('.chat-think-title')
        const content = el.querySelector('.think-content')

        if (title && content) {
          title.style.cursor = 'pointer'
          const clickHandler = () => {
            const isHidden = content.style.display === 'none'
            content.style.display = isHidden ? 'block' : 'none'
            if (isHidden) {
              title.classList.add('expanded')
            } else {
              title.classList.remove('expanded')
            }
          }
          el._thinkClickHandler = clickHandler
          title.addEventListener('click', clickHandler)
        }
      },
      unmounted: el => {
        const title = el.querySelector('.chat-think-title')
        if (title && el._thinkClickHandler) {
          title.removeEventListener('click', el._thinkClickHandler)
          delete el._thinkClickHandler
        }
      }
    }
  }
}

/**
 * Registers the directives for the application
 * @param {App} app - The Vue app instance
 */
export function registerDirective(app) {
  if (!app) return

  // Register all directives
  Object.values(DIRECTIVE_CONFIG).forEach(({ name, handlers }) => {
    app.directive(name, handlers)
  })
}
