use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::{
    AppHandle, EventId, Listener, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent, Wry,
};

#[cfg(debug_assertions)]
use serde_json::Value;

use super::types::{FullConfig, GenericContentRule};
use crate::constants::CFG_SCRAPER_DEBUG_MODE;
use crate::db::MainStore;

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
        #[cfg(debug_assertions)]
        app_handle.listen("logger_event", move |event| {
            if let Ok(payload) = serde_json::from_str::<Value>(event.payload()) {
                log::debug!(
                    "window:{}, message:{}",
                    payload.get("window").unwrap_or(&Value::Null),
                    payload.get("message").unwrap_or(&Value::Null)
                );
            }
        });

        Self {
            app_handle,
            timeout: Duration::from_secs(30),
        }
    }

    /// Scrapes the content of a webpage using a webview.
    pub async fn scrape(
        &self,
        webview: &WebviewWindow<Wry>,
        url: &str,
        config: Option<FullConfig>,
        generic_content_rule: Option<GenericContentRule>,
    ) -> Result<(String, Vec<EventId>)> {
        let (tx_page_load, rx_page_load) = tokio::sync::oneshot::channel::<()>();
        let (tx_dom_content_loaded, rx_dom_content_loaded) = tokio::sync::oneshot::channel::<()>();
        let (tx_scrape_result, rx_scrape_result) =
            tokio::sync::oneshot::channel::<Result<String>>();

        let scrape_result_sender = Arc::new(Mutex::new(Some(tx_scrape_result)));

        let window_label = webview.label();
        // --- Attach listeners directly ---
        let page_loaded_id = webview.once(format!("page_loaded_{}", window_label), move |_event| {
            let _ = tx_page_load.send(());
        });

        let dom_content_loaded_id = webview.once(
            format!("DOMContentLoaded_ready_{}", window_label),
            move |_event| {
                let _ = tx_dom_content_loaded.send(());
            },
        );

        let is_completed = Arc::new(AtomicBool::new(false));

        let sender_for_listen = scrape_result_sender.clone();
        let is_completed_for_listen = is_completed.clone();
        let window_label_clone = window_label.to_string();
        let scrape_result_listener_id = webview.listen(
            format!("scrape_result_{}", &window_label_clone),
            move |event| {
                let data_str = event.payload();

                #[cfg(debug_assertions)]
                log::debug!(
                    "receive scrape_result_{}, result: {}",
                    &window_label_clone,
                    &data_str
                );

                // Check if already completed before doing any processing
                if is_completed_for_listen.load(Ordering::SeqCst) {
                    return;
                }

                match serde_json::from_str::<ScrapeResultMessage>(data_str) {
                    Ok(result) => {
                        // Use compare_exchange to ensure atomic check-and-set
                        match is_completed_for_listen.compare_exchange(
                            false,
                            true,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        ) {
                            Ok(_) => {
                                // Successfully claimed the handler
                                if let Ok(mut guard) = sender_for_listen.lock() {
                                    if let Some(tx) = guard.take() {
                                        if let Some(success) = result.success {
                                            let _ = tx.send(Ok(success));
                                        } else if let Some(error) = result.error {
                                            let _ = tx.send(Err(anyhow!(error)));
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                // Already handled by another thread
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to parse scrape result message for scrape_result_{}: {}, rawData: {}",
                            &window_label_clone,
                            e,
                            data_str
                        );
                    }
                }
            },
        );

        let main_store = self.app_handle.state::<Arc<RwLock<MainStore>>>();
        let debug_mode = if let Ok(store) = main_store.read() {
            store.get_config(CFG_SCRAPER_DEBUG_MODE, false)
        } else {
            false
        };

        if debug_mode {
            let webview_clone = webview.clone();
            webview.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = webview_clone.hide();
                }
            });
        }

        let page_timeout = config
            .as_ref()
            .map(|cfg| Duration::from_millis(cfg.config.page_timeout))
            .unwrap_or(self.timeout);

        webview
            .navigate(url.parse()?)
            .map_err(|e| anyhow!(e.to_string()))?;

        let result = async {
            let page_load_result = tokio::time::timeout(page_timeout, rx_page_load).await;
            if page_load_result.is_err() {
                // Mark as completed to prevent further processing
                is_completed.store(true, Ordering::SeqCst);
                return Err(anyhow!("Page load timed out for URL: {}", url));
            }

            let dom_content_result =
                tokio::time::timeout(page_timeout, rx_dom_content_loaded).await;
            if dom_content_result.is_err() {
                // Mark as completed to prevent further processing
                is_completed.store(true, Ordering::SeqCst);
                return Err(anyhow!("DOMContentLoaded timed out for URL: {}", url));
            }

            if url.contains("bing.com") {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }

            self.inject_and_run_script(&webview, config, generic_content_rule)?;

            let scrape_result = tokio::time::timeout(page_timeout, rx_scrape_result).await;
            // Mark as completed after receiving result or timeout
            is_completed.store(true, Ordering::SeqCst);

            match scrape_result {
                Ok(Ok(res)) => res,
                Ok(Err(_)) => Err(anyhow!("Scrape result channel closed unexpectedly.")),
                Err(_) => Err(anyhow!("Scraping timed out for URL: {}", url)),
            }
        }
        .await;

        let listeners = vec![
            page_loaded_id,
            dom_content_loaded_id,
            scrape_result_listener_id,
        ];

        Ok((result?, listeners))
    }

    /// Injects and runs the script for scraping.
    fn inject_and_run_script(
        &self,
        webview: &WebviewWindow<Wry>,
        config: Option<FullConfig>,
        generic_content_rule: Option<GenericContentRule>,
    ) -> Result<()> {
        log::debug!(
            "=== Injecting and running script for scraping window {} === ",
            webview.label()
        );
        let utility_js = include_str!("../../assets/scrape/utility.min.js");
        let turndown_js = include_str!("../../assets/scrape/turndown.min.js");
        let scrape_logic_js = include_str!("../../assets/scrape/scrape_logic.min.js");

        let config_json_str =
            serde_json::to_string(&config).context("Failed to serialize scraper config to JSON")?;
        let generic_content_rule_json_str =
            serde_json::to_string(&generic_content_rule.unwrap_or_default())
                .context("Failed to serialize generic content rule to JSON")?;
        let js_code = format!(
            r#"
            window.performScrape || (()=>{{
                try {{
                    console.debug('Starting script injection...');
                    {utility_js}
                    {turndown_js}
                    {scrape_logic_js}
                    console.debug('end script injection');
                    window.performScrape({config_json_str}, {generic_content_rule_json_str});
                }} catch (error) {{
                    console.error('Script injection failed:', error);
                }}
            }})();
            "#,
        );
        webview.eval(&js_code)?;
        Ok(())
    }

    /// Creates a new webview window for scraping.
    pub fn create_webview(
        &self,
        url: &str,
        visible: bool,
        _block_images: bool,
    ) -> Result<WebviewWindow<Wry>> {
        let window_label = format!(
            "scraper-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );

        let init_script = format!(
            r#"
            const windowLabel = '{window_label}';
            window.addEventListener('load', () => {{
                if (window.__TAURI__?.event) {{
                    window.__TAURI__.event.emit('page_loaded_{window_label}');
                }}
            }});
            window.addEventListener('DOMContentLoaded', () => {{
                if (window.__TAURI__?.event) {{
                    window.__TAURI__.event.emit('DOMContentLoaded_ready_{window_label}');
                }}
            }});
        "#
        );

        // if block_images {
        //     let block_script = r#"
        //         const blankImage = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
        //         const observer = new MutationObserver((mutations) => {
        //           for (const mutation of mutations) {
        //             for (const node of mutation.addedNodes) {
        //               if (node.tagName === "IMG") {
        //                 node.src = blankImage;
        //               }
        //               if (node.querySelectorAll) {
        //                 node.querySelectorAll("img").forEach((img) => (img.src = blankImage));
        //               }
        //             }
        //           }
        //         });

        //         observer.observe(document, {
        //           childList: true,
        //           subtree: true
        //         });

        //         document.addEventListener("DOMContentLoaded",() => {
        //             document.querySelectorAll("img").forEach((img) => (img.src = blankImage));
        //           },
        //           { once: true }
        //         );
        //     "#;
        //     init_script.push_str(block_script);
        // }

        #[allow(unused_mut)]
        let mut webview_builder = WebviewWindowBuilder::new(
            &self.app_handle,
            &window_label,
            WebviewUrl::External(url.parse()?),
        )
        .title("Chatspeed Web Scraper")
        .initialization_script(&init_script)
        .visible(visible);

        #[cfg(target_os = "windows")]
        {
            let main_store = self.app_handle.state::<Arc<std::sync::RwLock<MainStore>>>();
            let mut proxy_arg_option: Option<String> = None;

            if let Ok(store) = main_store.read() {
                let proxy_type = store.get_config("proxy_type", String::new());
                let proxy_server = store.get_config("proxy_server", String::new());
                let proxy_username = store.get_config("proxy_username", String::new());
                let proxy_password = store.get_config("proxy_password", String::new());

                if proxy_type == "http"
                    && !proxy_server.is_empty()
                    && proxy_username.is_empty()
                    && proxy_password.is_empty()
                {
                    proxy_arg_option = Some(format!("--proxy-server={}", proxy_server));
                } else if !proxy_server.is_empty() {
                    log::warn!(
                        "Scraper webview is skipping authenticated proxy settings as it is not supported. It will use system network settings instead."
                    );
                }
            }

            if let Some(arg) = proxy_arg_option {
                log::info!("Applying proxy for scraper webview: {}", arg);
                webview_builder = webview_builder.additional_browser_args(&arg);
            }
        }

        webview_builder
            .build()
            .map_err(|e| anyhow!("Failed to create webview window: {}", e))
    }
}
