/**
 * @file This script is injected into a webview to perform scraping tasks.
 * It supports two modes:
 *  1. Schema-based scraping: Extracts data based on a provided JSON configuration.
 *  2. Generic scraping: Extracts the main content of a page as Markdown.
 */

(function () {
    // 等待Tauri API可用的辅助函数
    function waitForTauriAPI() {
        return new Promise((resolve) => {
            if (window.__TAURI__?.event) {
                resolve(window.__TAURI__.event);
            } else {
                const checkInterval = setInterval(() => {
                    if (window.__TAURI__?.event) {
                        clearInterval(checkInterval);
                        resolve(window.__TAURI__.event);
                    }
                }, 50);
                // 5秒后超时
                setTimeout(() => {
                    clearInterval(checkInterval);
                    resolve(null);
                }, 5000);
            }
        });
    }

    // 初始化事件发射器
    let eventEmitter = null;

    /**
     * Waits for an element matching the selector to appear in the DOM.
     * @param {string} selector - The CSS selector to wait for.
     * @param {number} timeout - Maximum time to wait in milliseconds.
     * @returns {Promise<Element>} A promise that resolves with the element or rejects on timeout.
     */
    function waitForElement(selector, timeout = 10000) {
        return new Promise((resolve, reject) => {
            const element = document.querySelector(selector);
            if (element) {
                return resolve(element);
            }

            const observer = new MutationObserver(() => {
                const el = document.querySelector(selector);
                if (el) {
                    observer.disconnect();
                    resolve(el);
                }
            });

            observer.observe(document.body, {
                childList: true,
                subtree: true,
            });

            setTimeout(() => {
                observer.disconnect();
                reject(new Error(`Timeout waiting for element: ${selector}`));
            }, timeout);
        });
    }

    /**
     * Extracts data from a single element based on a field configuration.
     * @param {Element} element - The parent element to search within.
     * @param {object} field - The field configuration object.
     * @returns {string|null} The extracted data.
     */
    function extractField(element, field) {
        const target = element.querySelector(field.selector);
        if (!target) return null;

        switch (field.type) {
            case 'text':
                return target.innerText;
            case 'attribute':
                return target.getAttribute(field.attribute);
            case 'html':
                return target.innerHTML;
            case 'markdown':
                const turndownService = new TurndownService();
                return turndownService.turndown(target);
            default:
                return null;
        }
    }

    /**
     * 发送抓取结果到Rust后端
     * @param {object} data - 要发送的数据
     */
    async function sendScrapeResult(data) {
        if (eventEmitter) {
            eventEmitter.emit('scrape_result', data);
        } else {
            console.error('Tauri事件发射器不可用');
        }
    }

    /**
     * Performs scraping based on a provided schema configuration.
     * @param {object} config - The scraper configuration.
     */
    async function scrapeWithSchema(config) {
        try {
            await waitForElement(config.selectors.baseSelector, config.config.waitForTimeout || 10000);

            const results = [];
            const elements = document.querySelectorAll(config.selectors.baseSelector);

            elements.forEach(element => {
                const item = {};
                config.selectors.fields.forEach(field => {
                    item[field.name] = extractField(element, field);
                });
                results.push(item);
            });

            await sendScrapeResult({ success: JSON.stringify(results) });
        } catch (error) {
            await sendScrapeResult({ error: `Schema scraping failed: ${error.message}` });
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
                emDelimiter: '_',
            });

            let title = document.title || document.querySelector('h1, h2, h3')?.innerText || '';

            const content = document.body.cloneNode(true);

            // Remove non-content elements
            const elementsToRemove = content.querySelectorAll(
                'script, style, noscript, nav, footer, header, aside, form, iframe, frame, video, audio, link'
            );
            elementsToRemove.forEach(el => el.remove());

            let markdown = turndownService.turndown(content);

            // Basic cleanup
            markdown = markdown
                .replace(/\\\\[(.*?)\\\\]/g, '[$1]') // Fix escaped brackets
                .replace(/\n{3,}/g, '\n\n')      // Collapse multiple newlines
                .trim();

            const result = { title, content: markdown };
            sendScrapeResult({ success: JSON.stringify(result) });
        } catch (error) {
            sendScrapeResult({ error: `Generic scraping failed: ${error.message}` });
        }
    }

    /**
     * Main entry point for the scraping logic.
     * @param {object | null} config - The configuration object, or null for generic scraping.
     */
    window.performScrape = function (config) {
        // The TurndownService might not be loaded immediately.
        // We wait for it to be available before proceeding.
        const checkTurndown = setInterval(() => {
            if (typeof TurndownService !== 'undefined') {
                clearInterval(checkTurndown);
                if (config) {
                    scrapeWithSchema(config);
                } else {
                    scrapeGeneric();
                }
            }
        }, 100);
    };

    // 初始化并通知后端
    waitForTauriAPI().then(event => {
        eventEmitter = event;
        if (eventEmitter) {
            eventEmitter.emit('page_loaded');
        } else {
            console.warn('Tauri API不可用，无法发送事件');
        }
    });
})();

// 添加全局错误处理
window.addEventListener('error', (event) => {
    console.error('Global error:', event.error);
    if (window.__TAURI__?.event) {
        window.__TAURI__.event.emit('scrape_result', {
            error: 'Global error: ' + event.message
        });
    }
});

window.addEventListener('unhandledrejection', (event) => {
    console.error('Unhandled promise rejection:', event.reason);
    if (window.__TAURI__?.event) {
        window.__TAURI__.event.emit('scrape_result', {
            error: 'Unhandled promise rejection: ' + event.reason
        });
    }
});
