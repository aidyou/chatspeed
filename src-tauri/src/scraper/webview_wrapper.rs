use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Listener, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use uuid::Uuid;

use super::scraper_config::FullConfig;

#[derive(Deserialize, Debug)]
struct ScrapeResult {
    success: Option<String>,
    error: Option<String>,
}

/// A wrapper for creating and managing webviews for scraping purposes.
pub struct WebviewScraper {
    app_handle: AppHandle<Wry>,
    timeout: Duration,
}

impl WebviewScraper {
    /// Creates a new `WebviewScraper`.
    pub fn new(app_handle: AppHandle<Wry>) -> Self {
        Self {
            app_handle,
            timeout: Duration::from_secs(60), // Increased timeout for complex pages
        }
    }

    /// Scrapes a given URL using a specific configuration.
    ///
    /// It creates a hidden webview, injects the necessary JavaScript, and executes
    /// the scraping logic based on the provided configuration.
    ///
    /// # Arguments
    /// * `url` - The URL to scrape.
    /// * `config` - An `Option<FullConfig>`. If `Some`, it performs schema-based
    ///   scraping. If `None`, it performs generic content extraction.
    ///
    /// # Returns
    /// A `Result<String>` containing the scraped data, usually as a JSON string.
    pub async fn scrape(&self, url: &str, config: Option<FullConfig>) -> Result<String> {
        let webview = self.create_webview(url)?;

        // Channel for page load signal
        let (tx_page_load, rx_page_load) = tokio::sync::oneshot::channel::<()>();
        let tx_page_load = Arc::new(Mutex::new(Some(tx_page_load)));

        // Channel for the final scrape result
        let (tx_scrape_result, rx_scrape_result) =
            tokio::sync::oneshot::channel::<Result<String>>();
        let tx_scrape_result = Arc::new(Mutex::new(Some(tx_scrape_result)));

        // Listen for the 'page_loaded' event from the injected script
        let url_clone = url.to_string();
        let page_load_listener = webview.listen("page_loaded", move |_event| {
            log::debug!("page_loaded event received for URL: {}", url_clone);
            if let Ok(mut guard) = tx_page_load.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            } else {
                log::error!("Failed to acquire tx_page_load lock");
            }
        });

        // Wait for the page and our initial script to load
        if tokio::time::timeout(self.timeout, rx_page_load)
            .await
            .is_err()
        {
            webview.unlisten(page_load_listener);
            let _ = webview.close();

            return Err(anyhow!("Page script injection timed out for URL: {}", url));
        }
        webview.unlisten(page_load_listener);

        // Now that the page is ready, listen for the 'scrape_result' event
        let tx_scrape_result_clone = tx_scrape_result.clone();
        let url_clone = url.to_string();
        let scrape_result_listener = webview.listen("scrape_result", move |event| {
            #[cfg(debug_assertions)]
            log::debug!(
                "scrape_result event received for URL: {}, payload: {}",
                url_clone,
                event.payload()
            );

            if let Ok(mut guard) = tx_scrape_result_clone.lock() {
                if let Some(tx) = guard.take() {
                    let payload = event.payload();
                    match serde_json::from_str::<ScrapeResult>(payload) {
                        Ok(result) => {
                            if let Some(success) = result.success {
                                let _ = tx.send(Ok(success));
                            } else if let Some(error) = result.error {
                                let _ = tx.send(Err(anyhow!(error)));
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(anyhow!("Failed to parse scrape result: {}", e)));
                        }
                    }
                }
            } else {
                log::error!("Failed to acquire tx_scrape_result_clone lock");
            }
        });

        // Prepare the JavaScript to be executed
        let turndown_js = include_str!("../../assets/scrape/turndown.min.js");
        let scrape_logic_js = include_str!("../../assets/scrape/scrape_logic.js");

        // Serialize the config to pass to the JS function.
        // It becomes "null" if config is None.
        let config_json_str =
            serde_json::to_string(&config).context("Failed to serialize scraper config to JSON")?;

        let js_code = format!(
            r#"
            try {{
                console.log('Starting script injection...');

                // Inject libraries first
                {turndown_js}
                console.log('Turndown library loaded');

                // Inject core logic
                {scrape_logic_js}
                console.log('Scrape logic loaded');

                // Execute the scrape
                console.log('Executing scrape with config: {config_json_str}');
                window.performScrape({config_json_str});
            }} catch (error) {{
                console.error('Script injection failed:', error);
                if (window.__TAURI__?.event) {{
                    window.__TAURI__.event.emit('scrape_result', {{
                        error: 'Script injection failed: ' + error.message
                    }});
                }}
            }}
            "#,
        );

        #[cfg(debug_assertions)]
        log::debug!("JS code to be executed: {}", js_code);

        webview.eval(&js_code)?;

        // Wait for the scrape result from the event listener
        let result = match tokio::time::timeout(self.timeout, rx_scrape_result).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(anyhow!("Scrape result channel closed unexpectedly.")),
            Err(_) => Err(anyhow!("Scraping timed out for URL: {}", url)),
        };

        webview.unlisten(scrape_result_listener);
        let _ = webview.close();
        result
    }

    /// Creates a new hidden webview window navigated to the specified URL.
    fn create_webview(&self, url: &str) -> Result<WebviewWindow<Wry>> {
        // Use a simple counter-based approach for window naming
        // This ensures the window names match those defined in scraper.json
        use std::sync::atomic::{AtomicUsize, Ordering};
        static WINDOW_COUNTER: AtomicUsize = AtomicUsize::new(0);

        let window_id = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed) % 10; // Cycle through 0-9
        let window_label = format!("scraper{}", window_id);

        // This script will run as soon as the webview is created.
        // It waits for Tauri API to be available and then emits the page_loaded event.
        let init_script = r#"
            function waitForTauriAPI() {
                return new Promise((resolve) => {
                    if (window.__TAURI__?.event) {
                        resolve();
                    } else {
                        const checkInterval = setInterval(() => {
                            if (window.__TAURI__?.event) {
                                clearInterval(checkInterval);
                                resolve();
                            }
                        }, 50);
                        // Fallback timeout after 5 seconds
                        setTimeout(() => {
                            clearInterval(checkInterval);
                            resolve();
                        }, 5000);
                    }
                });
            }

            // 监听页面加载状态
            window.addEventListener('load', () => {
                console.log('Page fully loaded');
            });

            window.addEventListener('error', (event) => {
                console.error('Page load error:', event);
            });

            window.addEventListener('DOMContentLoaded', async () => {
                console.log('DOMContentLoaded fired, waiting for Tauri API...');
                await waitForTauriAPI();
                if (window.__TAURI__?.event) {
                    window.__TAURI__.event.emit('page_loaded');
                    console.log('DOMContentLoaded event fired');
                } else {
                    console.warn('Tauri API not available');
                }
            });
        "#;

        let webview = WebviewWindowBuilder::new(
            &self.app_handle,
            &window_label,
            WebviewUrl::External(url.parse()?),
        )
        .title("Scraper")
        .initialization_script(init_script)
        .visible(true)
        .additional_browser_args("--disable-web-security --disable-features=VizDisplayCompositor --disable-site-isolation-trials")
        .content_protected(false)
        .on_page_load(|w,x| {
            #[cfg(debug_assertions)]
            log::debug!("Page loaded for URL: {}, event: {:?}", w.url().map(|u|u.to_string()).unwrap_or_default(),x.event());

        })
        .build()
        .map_err(|e| anyhow!("Failed to create webview window: {}", e))?;

        #[cfg(debug_assertions)]
        log::debug!("Created scraper window with label: {}", window_label);

        Ok(webview)
    }
}
