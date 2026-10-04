use super::types::{FullConfig, GenericContentRule};
use super::webview_wrapper::WebviewScraper;
use crate::libs::webview_proxy::WebviewProxy;
use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, EventId, Listener, WebviewWindow, Wry};
use tokio::sync::{Mutex, Semaphore};

// const MIN_POOL_SIZE: usize = 1;
const MAX_POOL_SIZE: usize = 10;
const MAX_CONCURRENT_SCRAPES: usize = 1;
const IDLE_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Represents a webview resource in the pool, including its listeners and usage metadata.
pub struct WebViewResource {
    pub webview: Arc<WebviewWindow<Wry>>,
    pub listeners: Vec<EventId>,
    pub last_used: Instant,
}

/// Manages a collection of reusable `WebViewResource` instances.
pub struct ScraperPool {
    pool: Arc<Mutex<Vec<WebViewResource>>>,
    scraper: Arc<WebviewScraper>,
    semaphore: Arc<Semaphore>,
    app_handle: AppHandle<Wry>,
}

impl ScraperPool {
    /// Creates a new `ScraperPool` and starts the cleanup timer.
    pub fn new(app_handle: AppHandle<Wry>) -> Arc<Self> {
        let scraper = Arc::new(WebviewScraper::new(app_handle.clone()));
        let pool = Arc::new(Mutex::new(Vec::with_capacity(MAX_POOL_SIZE)));

        let scraper_pool = Arc::new(Self {
            pool: pool.clone(),
            scraper,
            // WebKit-backed hidden windows are not reliable when multiple searches
            // create and navigate them concurrently, especially in release builds.
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SCRAPES)),
            app_handle,
        });

        // Start the cleanup timer
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(IDLE_TIMEOUT_SECS)).await;
                let mut pool = pool_clone.lock().await;
                let now = Instant::now();
                let initial_len = pool.len();

                // Use retain to efficiently remove items
                pool.retain(|resource| {
                    if now.duration_since(resource.last_used).as_secs() > IDLE_TIMEOUT_SECS {
                        // Cleanup before dropping
                        for &id in &resource.listeners {
                            resource.webview.unlisten(id);
                        }
                        if let Err(e) = resource.webview.close() {
                            log::error!("Failed to close webview during cleanup: {}", e);
                        }
                        false // Remove from pool
                    } else {
                        true // Keep in pool
                    }
                });

                let removed_count = initial_len - pool.len();
                if removed_count > 0 {
                    log::debug!("Cleanup removed {} idle webviews from pool", removed_count);
                    // The cleanup only removes webviews from the pool, not active ones
                    // Active webviews have their permits held by the scrape() method
                }
            }
        });

        scraper_pool
    }

    /// Retrieves a webview from the pool or creates a new one if none are available.
    ///
    /// `debug_mode` and `proxy` come from the runtime configuration snapshot the
    /// caller read for this scrape, so a setting changed since the last scrape is
    /// seen by the webview this call creates or reuses.
    async fn get(
        &self,
        debug_mode: bool,
        proxy: Option<WebviewProxy>,
    ) -> Result<(WebViewResource, tokio::sync::OwnedSemaphorePermit)> {
        // Acquire a permit, waiting if the pool is at max capacity
        let permit = self.semaphore.clone().acquire_owned().await?;
        let mut pool = self.pool.lock().await;

        if let Some(mut resource) = pool.pop() {
            resource.last_used = Instant::now();
            if debug_mode {
                let _ = resource.webview.show();
            } else if resource.webview.is_visible().unwrap_or(false) {
                let _ = resource.webview.hide();
            }
            Ok((resource, permit))
        } else {
            let webview = self
                .scraper
                .create_webview("about:blank", debug_mode, true, proxy)?;
            Ok((
                WebViewResource {
                    webview: Arc::new(webview),
                    listeners: Vec::new(),
                    last_used: Instant::now(),
                },
                permit,
            ))
        }
    }

    /// Returns a webview resource to the pool for future reuse.
    /// The permit parameter is crucial - it will be automatically released when this method ends,
    /// which decrements the semaphore count and allows new requests to acquire resources.
    async fn release(
        &self,
        mut resource: WebViewResource,
        _permit: tokio::sync::OwnedSemaphorePermit,
        debug_mode: bool,
    ) {
        // Clear old listeners before releasing back to the pool
        for listener_id in resource.listeners.drain(..) {
            resource.webview.unlisten(listener_id);
        }

        // Only navigate to blank page in debug mode or when pool is full
        // This preserves the performance benefit of webview pooling
        if debug_mode {
            // In debug mode, keep the page for inspection
            log::debug!("Debug mode: keeping current page for inspection");
        } else {
            // Always clear JavaScript state to prevent pollution between scrapes
            let cleanup_script = r#"
                // Reset global variables that might cause state pollution
                if (typeof hasSentResult !== 'undefined') {
                    hasSentResult = false;
                }
                // Clear any remaining timers or intervals that might interfere
                for (let i = 1; i < 1000; i++) {
                    window.clearTimeout(i);
                    window.clearInterval(i);
                }
            "#;

            if let Err(e) = resource.webview.eval(cleanup_script) {
                log::warn!("Failed to run cleanup script: {}", e);
            }
        }

        resource.last_used = Instant::now();
        let mut pool = self.pool.lock().await;

        // Only clear page if we're not reusing the webview
        let should_clear_page = pool.len() >= MAX_POOL_SIZE / 2;

        if should_clear_page && !debug_mode {
            if let Ok(blank_url) = url::Url::parse("about:blank") {
                if let Err(e) = resource.webview.navigate(blank_url) {
                    log::warn!("Failed to navigate to blank page: {}", e);
                }
            }
        }

        // Limit pool size to prevent memory accumulation
        if pool.len() >= MAX_POOL_SIZE / 2 {
            log::debug!("Pool size limit reached, closing webview instead of reusing");
            for &id in &resource.listeners {
                resource.webview.unlisten(id);
            }
            if let Err(e) = resource.webview.close() {
                log::error!("Failed to close webview: {}", e);
            }
            // Don't push to pool, webview is closed
            // Permit will be automatically released when it goes out of scope
        } else {
            pool.push(resource);
            // Permit will be automatically released when it goes out of scope
            // This makes the resource available for the next get() call
        }
    }

    /// Executes the scraping process using a webview from the pool.
    pub async fn scrape(
        &self,
        url: &str,
        config: Option<FullConfig>,
        generic_content_rule: Option<GenericContentRule>,
    ) -> Result<String> {
        // Read the runtime configuration once per scrape. The runtime owns these
        // values, so a debug/proxy the user just changed is seen by the webview
        // this scrape creates; before the runtime publishes a snapshot the debug
        // display stays off and no proxy is applied.
        let snapshot = crate::runtime_config::current_or_load(&self.app_handle).await;
        let debug_mode = snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.scraper_debug_mode());
        let proxy = snapshot
            .as_ref()
            .and_then(|snapshot| WebviewProxy::from_snapshot(snapshot));

        let (mut resource, permit) = self.get(debug_mode, proxy).await?;

        let (scrape_result, listeners) = self
            .scraper
            .scrape(&resource.webview, url, config, generic_content_rule, debug_mode)
            .await;

        resource.listeners = listeners; // Always assign listeners

        match scrape_result {
            Ok(result) => {
                self.release(resource, permit, debug_mode).await;
                Ok(result)
            }
            Err(e) => {
                // On error, we still need to release the resource
                self.release(resource, permit, debug_mode).await;
                Err(e)
            }
        }
    }
}
