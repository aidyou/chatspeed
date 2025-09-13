//! The main engine for the scraper module.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, Wry};
use url::Url;

use super::config_loader::ConfigLoader;
use super::pool::ScraperPool;
use crate::libs::util;
use crate::scraper::types::{ContentOptions, GenericContentRule, ScrapeRequest};
use crate::search::SearchResult;

/// The primary entry point for the scraper module.
///
/// This function orchestrates the entire scraping process:
/// 1. Loads the appropriate configuration based on the request type.
/// 2. Constructs the target URL (for search requests).
/// 3. Initializes and runs the `WebviewScraper`.
/// 4. Returns the scraped data as a string (typically JSON).
pub async fn run(app_handle: AppHandle<Wry>, request: ScrapeRequest) -> Result<String> {
    let config_loader = ConfigLoader::new(&app_handle)?;
    let scraper_pool = app_handle.state::<Arc<ScraperPool>>().inner();

    match request {
        ScrapeRequest::Search(options) => {
            let config = config_loader.load_search_config(&options.provider)?;

            // URL-encode the query to ensure it's safe for the template.
            let encoded_query = util::urlencode(&options.query);

            // Use provided values or defaults.
            let max_per_page = if config.config.max_results_per_page > 0 {
                config.config.max_results_per_page
            } else {
                10
            };
            let page = options.page.unwrap_or(1).max(1);
            let number = options.number.unwrap_or(5).min(10) as usize;
            let start = SystemTime::now();
            let since_the_epoch = start.duration_since(UNIX_EPOCH).map_err(|e| {
                anyhow::anyhow!("Failed to get duration since epoch: {}", e.to_string())
            })?;
            let timestamp = since_the_epoch.as_millis().to_string();

            let page_param = match config.config.pagination_param.as_str() {
                "offset" => "offset",
                _ => "page",
            };
            let page_value = match config.config.pagination_type.as_str() {
                "offset" => {
                    if config.config.url_template.contains("duckduckgo.com") {
                        get_duckduckgo_offset(page)
                    } else {
                        (page - 1).checked_mul(number as u32).unwrap_or(0)
                    }
                }
                _ => {
                    let page_with_offset = (page as i32).saturating_add(config.config.page_offset);
                    page_with_offset.max(0) as u32
                }
            };

            let time_period = if let Some(time_period_str) = options.time_period.as_deref() {
                match config.config.url_template.as_str() {
                    url if url.contains("bing.com") => get_bing_time_period(time_period_str),
                    url if url.contains("sogou.com") => get_sogou_time_period(time_period_str),
                    url if url.contains("brave.com") => get_brave_time_period(time_period_str),
                    url if url.contains("so.com") || url.contains("duckduckgo.com") => {
                        get_time_period(time_period_str)
                    }
                    _ => String::new(),
                }
            } else {
                "".to_string()
            };

            let url = config
                .config
                .url_template
                .replace("{kw}", &encoded_query)
                .replace("{number}", &max_per_page.to_string())
                .replace("{timestamp}", &timestamp)
                .replace("{time_period}", &time_period)
                .replace("{rand}", &uuid::Uuid::new_v4().simple().to_string()[..8])
                .replace(
                    format!("{{{}}}", page_param).as_str(),
                    &page_value.to_string(),
                );

            log::debug!("Search url: {}", &url);

            scraper_pool
                .scrape(&url, Some(config), None)
                .await
                .map(|result_str| {
                    if let Ok(results) = serde_json::from_str::<Vec<SearchResult>>(&result_str) {
                        let limited_results = if results.len() > number {
                            &results[..number]
                        } else {
                            &results[..]
                        };
                        serde_json::to_string(limited_results).unwrap_or_default()
                    } else {
                        result_str
                    }
                })
        }
        ScrapeRequest::Content(ContentOptions {
            url,
            content_format,
            keep_link,
            keep_image,
        }) => {
            let url_obj = Url::parse(&url).context("Failed to parse content URL")?;
            let config = config_loader.load_content_config(&url_obj)?;
            let generic_content_rule = GenericContentRule {
                r#format: content_format.to_string(),
                keep_link,
                keep_image,
            };
            scraper_pool
                .scrape(&url, config, Some(generic_content_rule))
                .await
        }
        ScrapeRequest::Normal(ContentOptions {
            url,
            content_format,
            keep_link,
            keep_image,
        }) => {
            let generic_content_rule = GenericContentRule {
                r#format: content_format.to_string(),
                keep_link,
                keep_image,
            };
            scraper_pool
                .scrape(&url, None, Some(generic_content_rule))
                .await
        }
    }
}

fn get_time_period(time_period: &str) -> String {
    if time_period.is_empty() {
        return String::new();
    }
    return time_period[..1].to_string();
}

fn get_bing_time_period(time_period: &str) -> String {
    match time_period {
        "day" => "ex1%3a\"ez1\"".to_string(),
        "week" => "ex1%3a\"ez2\"".to_string(),
        "month" => "ex1%3a\"ez3\"".to_string(),
        "year" => "ex1%3a\"ez5\"".to_string(),
        _ => String::new(), // Return empty string for invalid input
    }
}

fn get_sogou_time_period(time_period: &str) -> String {
    match time_period {
        "day" => "1".to_string(),
        "week" => "2".to_string(),
        "month" => "3".to_string(),
        "year" => "4".to_string(),
        _ => String::new(),
    }
}

fn get_brave_time_period(time_period: &str) -> String {
    if time_period.is_empty() {
        return String::new();
    }
    return format!("p{}", time_period[..1].to_string());
}

fn get_duckduckgo_offset(page: u32) -> u32 {
    if page <= 1 {
        0
    } else {
        10 + (page - 2) * 15
    }
}
