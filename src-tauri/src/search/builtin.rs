use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use tauri::{AppHandle, Wry};

use crate::scraper::engine;
use crate::scraper::types::{ScrapeRequest, SearchOptions};

use super::{SearchParams, SearchPeriod, SearchProvider, SearchProviderName, SearchResult};

pub struct BuiltInSearch {
    pub app_handle: AppHandle<Wry>,
    pub provider: SearchProviderName,
}

#[async_trait]
impl SearchProvider for BuiltInSearch {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        let search_params = SearchParams::try_from(params)?;

        let query = search_params.query;
        let provider = self.provider.clone();

        let time_period = search_params.period.map(|p| match p {
            SearchPeriod::Hour => "hour".to_string(),
            SearchPeriod::Day => "day".to_string(),
            SearchPeriod::Week => "week".to_string(),
            SearchPeriod::Month => "month".to_string(),
            SearchPeriod::Year => "year".to_string(),
        });

        let opts = SearchOptions {
            provider: provider.to_string(),
            query,
            number: search_params.count,
            page: search_params.page,
            time_period,
        };

        let request = ScrapeRequest::Search(opts);
        let res = engine::run(self.app_handle.clone(), request).await?;

        if res.is_empty() {
            return Ok(vec![]);
        }

        serde_json::from_str(&res)
            .map_err(|e| anyhow!("Failed to parse search results from scraper: {}", e))
    }
}
