use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Listener, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use uuid::Uuid;

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
            timeout: Duration::from_secs(30),
        }
    }

    /// Scrapes a given URL by creating a hidden webview, executing JavaScript,
    /// and returning the result via events.
    pub async fn scrape_url(&self, url: &str, selector: Option<&str>) -> Result<String> {
        let webview = self.create_webview(url)?;
        let (tx_page_load, rx_page_load) = tokio::sync::oneshot::channel::<()>();
        let tx_page_load = Arc::new(Mutex::new(Some(tx_page_load)));

        let (tx_scrape_result, rx_scrape_result) =
            tokio::sync::oneshot::channel::<Result<String>>();
        let tx_scrape_result = Arc::new(Mutex::new(Some(tx_scrape_result)));

        let tx_page_load_clone = tx_page_load.clone();
        let page_load_listener = webview.listen("page_loaded", move |_event| {
            if let Some(tx) = tx_page_load_clone.lock().unwrap().take() {
                let _ = tx.send(());
            }
        });

        // Wait for the page to load
        if tokio::time::timeout(self.timeout, rx_page_load)
            .await
            .is_err()
        {
            webview.unlisten(page_load_listener);
            let _ = webview.close();
            return Err(anyhow!("Page load timed out for URL: {}", url));
        }
        webview.unlisten(page_load_listener);

        // Now that the page is loaded, listen for the scrape result
        let tx_scrape_result_clone = tx_scrape_result.clone();
        let scrape_result_listener = webview.listen("scrape_result", move |event| {
            if let Some(tx) = tx_scrape_result_clone.lock().unwrap().take() {
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
        });

        let turndown_js = include_str!("../../assets/scrape/turndown.min.js");
        let scrape_logic_js = include_str!("../../assets/scrape/scrape_logic.js");

        let js_code = format!(
            r#"
            {turndown_js}
            {scrape_logic_js}
            executeScrape({selector_value});
            "#,
            selector_value = selector
                .map(|s| format!("'{}'", s))
                .unwrap_or("null".to_string())
        );

        webview.eval(&js_code)?;

        // Wait for the scrape result
        let result = match tokio::time::timeout(self.timeout, rx_scrape_result).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => return Err(anyhow!("Scrape result channel closed unexpectedly.")),
            Err(_) => Err(anyhow!("Scraping timed out for URL: {}", url)),
        };

        webview.unlisten(scrape_result_listener);
        let _ = webview.close();
        result
    }

    /// Creates a new hidden webview window navigated to the specified URL.
    fn create_webview(&self, url: &str) -> Result<WebviewWindow<Wry>> {
        let window_label = format!("scraper-{}", Uuid::new_v4());
        WebviewWindowBuilder::new(&self.app_handle, &window_label, WebviewUrl::App(url.into()))
            .title("Hidden Scraper")
            .visible(false)
            .build()
            .map_err(|e| anyhow!("Failed to create webview window: {}", e))
    }
}
