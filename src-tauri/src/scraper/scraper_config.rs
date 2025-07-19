use serde::{Deserialize, Serialize};

/// Represents a single field to be extracted from a webpage.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    pub selector: String,
    #[serde(rename = "type")]
    pub field_type: String, // e.g., "text", "attribute", "markdown"
    #[serde(default)]
    pub attribute: String,
}

/// Represents the set of selectors for a scraping target.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Selectors {
    pub base_selector: String,
    pub fields: Vec<Field>,
}

/// A flexible configuration struct that can represent both
/// search engine scrapers and content extraction schemas.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScraperConfig {
    pub name: String,
    #[serde(default)]
    pub url_template: String,
    #[serde(default)]
    pub pagination_type: String,
    #[serde(default)]
    pub pagination_param: String,
    pub wait_for: String,
    #[serde(default)]
    pub wait_timeout: u64,
    pub page_timeout: u64,
    #[serde(default)]
    pub max_results_per_page: u32,
    #[serde(default)]
    pub js_code: String,
}

/// The complete structure of a scraper or schema configuration file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FullConfig {
    pub config: ScraperConfig,
    pub selectors: Selectors,
}