use super::types::{FullConfig, GenericContentRule};
use super::webview_wrapper::WebviewScraper;
use crate::constants::CFG_SCRAPER_DEBUG_MODE;
use crate::db::MainStore;
use anyhow::Result;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, EventId, Listener, Manager, WebviewWindow, Wry};
use tokio::sync::{Mutex, Semaphore};

const MIN_POOL_SIZE: usize = 1;
const MAX_POOL_SIZE: usize = 10;
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
            semaphore: Arc::new(Semaphore::new(MAX_POOL_SIZE)),
            app_handle,
        });

        // Start the cleanup timer
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(IDLE_TIMEOUT_SECS)).await;
                let mut pool = pool_clone.lock().await;
                let now = Instant::now();
                let len = pool.len();

                // Use retain to efficiently remove items
                pool.retain(|resource| {
                    if len > MIN_POOL_SIZE
                        && now.duration_since(resource.last_used).as_secs() > IDLE_TIMEOUT_SECS
                    {
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
            }
        });

        scraper_pool
    }

    /// Retrieves a webview from the pool or creates a new one if none are available.
    async fn get(&self) -> Result<(WebViewResource, tokio::sync::OwnedSemaphorePermit)> {
        // Acquire a permit, waiting if the pool is at max capacity
        let permit = self.semaphore.clone().acquire_owned().await?;
        let mut pool = self.pool.lock().await;

        // Create a new webview if the pool is empty
        let debug_mode = self
            .app_handle
            .state::<Arc<RwLock<MainStore>>>()
            .read()
            .map(|store| store.get_config(CFG_SCRAPER_DEBUG_MODE, false))
            .unwrap_or(false);

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
                .create_webview("about:blank", debug_mode, true)?;
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
    async fn release(&self, mut resource: WebViewResource) {
        // Clear old listeners before releasing back to the pool
        for listener_id in resource.listeners.drain(..) {
            resource.webview.unlisten(listener_id);
        }

        let debug_mode = self
            .app_handle
            .state::<Arc<RwLock<MainStore>>>()
            .read()
            .map(|store| store.get_config(CFG_SCRAPER_DEBUG_MODE, false))
            .unwrap_or(false);

        // If not in debug mode, navigate to a blank page to free up resources.
        if !debug_mode {
            if let Ok(blank_url) = url::Url::parse("about:blank") {
                if let Err(e) = resource.webview.navigate(blank_url) {
                    log::warn!("Failed to navigate to blank page: {}", e);
                }
            }
        }

        resource.last_used = Instant::now();
        let mut pool = self.pool.lock().await;
        pool.push(resource);
    }

    /// Executes the scraping process using a webview from the pool.
    pub async fn scrape(
        &self,
        url: &str,
        config: Option<FullConfig>,
        generic_content_rule: Option<GenericContentRule>,
    ) -> Result<String> {
        let (mut resource, _permit) = self.get().await?;

        let scrape_result = self
            .scraper
            .scrape(&resource.webview, url, config, generic_content_rule)
            .await;

        match scrape_result {
            Ok((result, listeners)) => {
                resource.listeners = listeners;
                self.release(resource).await;
                Ok(result)
            }
            Err(e) => {
                // On error, we still need to release the resource
                self.release(resource).await;
                Err(e)
            }
        }
    }
}
