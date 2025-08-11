//! Handles loading of scraper configurations from the filesystem.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Wry};
use url::Url;

use super::scraper_config::FullConfig;

/// Manages the loading of scraper configurations.
pub struct ConfigLoader {
    schema_dir: PathBuf,
}

impl ConfigLoader {
    /// Creates a new `ConfigLoader`.
    ///
    /// It determines the base directory for schema files, which is typically
    /// the application's data directory.
    pub fn new(_app: &AppHandle<Wry>) -> Result<Self> {
        #[cfg(debug_assertions)]
        let schema_dir = { &*crate::STORE_DIR.read() };

        #[cfg(not(debug_assertions))]
        let schema_dir = app.path().app_data_dir().ok_or_else(|| {
            anyhow!("Failed to resolve app data directory for scraper config loading")
        })?;

        Ok(Self {
            schema_dir: schema_dir.join("schema"),
        })
    }

    /// Loads a search engine configuration file based on the provider name.
    ///
    /// For example, a `provider` of "google" will attempt to load
    /// `{schema_dir}/search/google.json`.
    pub fn load_search_config(&self, provider: &str) -> Result<FullConfig> {
        let config_path = self
            .schema_dir
            .join("search")
            .join(format!("{}.json", provider));
        self.read_config_from_path(&config_path)
            .with_context(|| format!("Failed to load search config for provider: '{}'", provider))
    }

    /// Loads a content extraction configuration based on the URL's domain.
    ///
    /// This function implements a fallback strategy:
    /// 1. It first tries to find a config matching the full hostname (e.g., `sub.domain.com.json`).
    /// 2. If not found, it tries to find a config for the base domain (e.g., `domain.com.json`).
    /// 3. If neither is found, it returns `Ok(None)`.
    pub fn load_content_config(&self, url: &Url) -> Result<Option<FullConfig>> {
        let host = match url.host_str() {
            Some(h) => h,
            None => return Ok(None), // Cannot determine host, so no config is possible.
        };

        // 1. Try full hostname (e.g., `www.rust-lang.org.json`)
        let full_host_filename = format!("{}.json", host);
        let full_host_path = self.schema_dir.join("content").join(full_host_filename);

        if full_host_path.exists() {
            return self.read_config_from_path(&full_host_path).map(Some);
        }

        // 2. Try base domain (e.g., `rust-lang.org.json`)
        // This is a simple heuristic. A more robust solution might use a crate like `psl`.
        let domain_parts: Vec<&str> = host.split('.').collect();
        if domain_parts.len() > 2 {
            let base_domain = domain_parts.iter().skip(1).cloned().collect::<Vec<&str>>().join(".");
            let base_domain_filename = format!("{}.json", base_domain);
            let base_domain_path = self.schema_dir.join("content").join(base_domain_filename);

            if base_domain_path.exists() {
                return self.read_config_from_path(&base_domain_path).map(Some);
            }
        }

        // 3. If no config is found, return None.
        Ok(None)
    }

    /// Helper function to read and parse a config file from a given path.
    fn read_config_from_path(&self, path: &Path) -> Result<FullConfig> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read scraper config file at: {:?}", path))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse scraper config file at: {:?}", path))
    }
}