use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Listener, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

#[allow(unused_imports)]
use tauri::Manager;

use super::scraper_config::FullConfig;

#[derive(Deserialize, Debug, Clone)]
struct ScrapeResultMessage {
    success: Option<String>,
    error: Option<String>,
}

/// A wrapper for creating and managing webviews for scraping purposes.
pub struct WebviewScraper {
    app_handle: AppHandle<Wry>,
    timeout: Duration,
}

impl WebviewScraper {
    pub fn new(app_handle: AppHandle<Wry>) -> Self {
        Self {
            app_handle,
            timeout: Duration::from_secs(60),
        }
    }

    pub async fn scrape(&self, url: &str, config: Option<FullConfig>) -> Result<String> {
        let (tx_page_load, rx_page_load) = tokio::sync::oneshot::channel::<()>();
        let page_load_signal = Arc::new(Mutex::new(Some(tx_page_load)));

        let (tx_dom_content_loaded, rx_dom_content_loaded) = tokio::sync::oneshot::channel::<()>();
        let dom_content_loaded_signal = Arc::new(Mutex::new(Some(tx_dom_content_loaded)));

        let (tx_scrape_result, rx_scrape_result) =
            tokio::sync::oneshot::channel::<Result<String>>();
        let scrape_result_signal = Arc::new(Mutex::new(Some(tx_scrape_result)));

        let webview = self.create_webview(
            url,
            page_load_signal,
            dom_content_loaded_signal,
            scrape_result_signal,
        )?;

        if tokio::time::timeout(self.timeout, rx_page_load)
            .await
            .is_err()
        {
            let _ = webview.close();
            return Err(anyhow!("Page load timed out for URL: {}", url));
        }

        // Wait for DOMContentLoaded before injecting and running the script
        if tokio::time::timeout(self.timeout, rx_dom_content_loaded)
            .await
            .is_err()
        {
            let _ = webview.close();
            return Err(anyhow!("DOMContentLoaded timed out for URL: {}", url));
        }

        self.inject_and_run_script(&webview, config)?;

        let result = match tokio::time::timeout(self.timeout, rx_scrape_result).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(anyhow!("Scrape result channel closed unexpectedly.")),
            Err(_) => Err(anyhow!("Scraping timed out for URL: {}", url)),
        };

        let _ = webview.close();
        result
    }

    fn inject_and_run_script(
        &self,
        webview: &WebviewWindow<Wry>,
        config: Option<FullConfig>,
    ) -> Result<()> {
        let logger_js = include_str!("../../assets/scrape/utility.js");
        let turndown_js = include_str!("../../assets/scrape/turndown.min.js");
        let scrape_logic_js = include_str!("../../assets/scrape/scrape_logic.js");

        let config_json_str =
            serde_json::to_string(&config).context("Failed to serialize scraper config to JSON")?;

        let js_code = format!(
            r#"
            try {{
                console.debug('Starting script injection...');
                {logger_js}
                {turndown_js}
                logger.debug('Turndown library loaded');
                {scrape_logic_js}
                logger.debug('Scrape logic loaded');
                logger.debug('Executing scrape with config: {config_json_str}');
                window.performScrape({config_json_str});
            }} catch (error) {{
                logger.error('Script injection failed:', error);
            }}
            "#,
        );

        webview.eval(&js_code)?;
        Ok(())
    }

    fn create_webview(
        &self,
        url: &str,
        page_load_signal: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        dom_content_loaded_signal: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        scrape_result_signal: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<String>>>>>,
    ) -> Result<WebviewWindow<Wry>> {
        let window_label = format!("scraper-{}", uuid::Uuid::new_v4().simple().to_string());

        let init_script = format!(
            r#"
            window.addEventListener('load', () => console.log('Page fully loaded'));
            window.addEventListener('error', (event) => console.error('Page load error:', event));
            window.addEventListener('DOMContentLoaded', () => {{
                console.log('DOMContentLoaded fired');
                if (window.__TAURI__?.event) {{
                    window.__TAURI__.event.emit('DOMContentLoaded_ready');
                }}
            }});
        "#
        );

        WebviewWindowBuilder::new(
            &self.app_handle,
            &window_label,
            WebviewUrl::External(url.parse()?),
        )
        .title("Scraper")
        .initialization_script(init_script)
        .visible(true)
        .on_page_load(move |webview, _payload| {
            let tx_dom_clone = dom_content_loaded_signal.clone();
            webview.once("DOMContentLoaded_ready", move |_event| {
                if let Ok(mut guard) = tx_dom_clone.lock() {
                    if let Some(tx) = guard.take() {
                        if tx.send(()).is_err() {
                            log::error!(
                                "Failed to send DOMContentLoaded_ready signal, receiver dropped."
                            );
                        }
                    }
                }
            });

            let tx_scrape_clone = scrape_result_signal.clone();
            webview.listen("scrape_result", move |event| {
                let data_str = event.payload();
                log::debug!("Raw scrape_result payload: {:?}", data_str);
                if let Ok(result) = serde_json::from_str::<ScrapeResultMessage>(&data_str) {
                    log::debug!("Attempting to deserialize: {}", data_str);
                    if let Ok(mut guard) = tx_scrape_clone.lock() {
                        if let Some(tx) = guard.take() {
                            if let Some(success) = result.success {
                                let _ = tx.send(Ok(success));
                            } else if let Some(error) = result.error {
                                let _ = tx.send(Err(anyhow!(error)));
                            }
                        }
                    }
                } else {
                    log::warn!(
                        "Failed to deserialize scrape_result payload as ScrapeResultMessage: {}",
                        data_str
                    );
                }
            });

            #[cfg(debug_assertions)]
            {
                webview.listen("logger", move |event| {
                    if let Ok(payload) = serde_json::from_str::<Value>(event.payload()) {
                        log::debug!("{}", payload.get("message").unwrap_or(&Value::Null));
                    }
                });
            }

            if let Ok(mut guard) = page_load_signal.lock() {
                if let Some(tx) = guard.take() {
                    if tx.send(()).is_err() {
                        log::error!("Failed to send page_load signal, receiver dropped.");
                    }
                }
            }
        })
        .build()
        .map_err(|e| anyhow!("Failed to create webview window: {}", e))
    }
}
