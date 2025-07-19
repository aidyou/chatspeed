/**
 * Executes the scraping process and emits the result as an event.
 * @param {string|null} selector - A CSS selector for a specific element to scrape.
 */
function executeScrape(selector) {
    const { emit } = window.__TAURI__.event;
    try {
        // Initialize Turndown service
        const turndownService = new TurndownService({
            headingStyle: 'atx',
            hr: '---',
            bulletListMarker: '*',
            codeBlockStyle: 'fenced',
            emDelimiter: '_',
        });

        let targetElement = document.body;
        if (selector) {
            targetElement = document.querySelector(selector);
            if (!targetElement) {
                emit('scrape_result', { error: `Element with selector "${selector}" not found.` });
                return;
            }
        }

        // Clone the element to avoid modifying the live DOM
        const content = targetElement.cloneNode(true);

        // Remove unwanted elements
        const elementsToRemove = content.querySelectorAll('script, style, noscript, nav, footer, header, aside, form');
        elementsToRemove.forEach(el => el.remove());

        // Convert the cleaned HTML to Markdown
        let markdown = turndownService.turndown(content);

        // Post-processing to clean up the Markdown
        markdown = markdown
            .replace(/\\\[(.*?)\\\]/g, '[$1]') // Fix escaped brackets
            .replace(/\n{3,}/g, '\n\n') // Collapse multiple newlines
            .replace(/(\s*\n\s*){2,}/g, '\n\n') // Normalize spacing
            .trim();

        emit('scrape_result', { success: markdown });
    } catch (error) {
        emit('scrape_result', { error: error.message });
    }
}

// Notify the backend that the page has loaded and then execute the scrape
window.addEventListener('load', () => {
    const { emit } = window.__TAURI__.event;
    emit('page_loaded');
    // The actual call to executeScrape will be injected from Rust.
});
