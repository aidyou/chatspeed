use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// Represents the options for a search scrape request.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub query: String,
    pub provider: String,
    pub page: Option<u32>,
    pub number: Option<u32>,
    pub time_period: Option<String>,
}

#[derive(Clone)]
pub struct ContentOptions {
    pub url: String,
    pub content_format: StrapeContentFormat,
    pub keep_link: bool,
    pub keep_image: bool,
}

impl Default for ContentOptions {
    fn default() -> Self {
        ContentOptions {
            url: String::new(),
            content_format: StrapeContentFormat::Markdown,
            keep_link: true,
            keep_image: false,
        }
    }
}

/// Represents the type of scraping request.
pub enum ScrapeRequest {
    /// A request to scrape search engine results.
    Search(SearchOptions),
    /// A request to scrape content from a specific URL.
    Content(ContentOptions),
    Normal(ContentOptions),
}

#[derive(Clone)]
pub enum StrapeContentFormat {
    Markdown,
    Text,
}

impl From<StrapeContentFormat> for String {
    fn from(format: StrapeContentFormat) -> Self {
        match format {
            StrapeContentFormat::Markdown => "markdown".to_string(),
            StrapeContentFormat::Text => "text".to_string(),
        }
    }
}

impl From<String> for StrapeContentFormat {
    fn from(format: String) -> Self {
        match format.as_str() {
            "text" => StrapeContentFormat::Text,
            _ => StrapeContentFormat::Markdown,
        }
    }
}

impl Display for StrapeContentFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrapeContentFormat::Markdown => write!(f, "markdown"),
            StrapeContentFormat::Text => write!(f, "text"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct Fallback {
    #[serde(default)]
    pub selector: String,
    #[serde(default)]
    pub method: String,
}

/// Represents a single field to be extracted from a webpage.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct Field {
    pub name: String,
    #[serde(default)]
    pub selector: String,
    #[serde(rename = "type")]
    pub field_type: String, // e.g., "text", "attribute", "markdown", "html"
    #[serde(default)]
    pub attribute: String,
    #[serde(default)]
    pub required: Option<bool>,
}

/// Represents a scraping event.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct Event {
    pub event: String,
    pub selector: String,
}

/// Represents the set of selectors for a scraping target.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct Selectors {
    pub base_selector: String,
    pub fields: Vec<Field>,
}

/// A flexible configuration struct that can represent both
/// search engine scrapers and content extraction schemas.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub struct ScraperConfig {
    pub name: String,
    #[serde(default)]
    pub url_template: String,
    #[serde(default)]
    pub wait_for: String,
    #[serde(default)]
    pub wait_timeout: u64,
    #[serde(default)]
    pub page_timeout: u64,
    #[serde(default)]
    pub max_results_per_page: u32,
    #[serde(default)]
    pub pagination_type: String,
    #[serde(default)]
    pub pagination_param: String,
    #[serde(default)]
    pub page_offset: i32,
    // #[serde(default)]
    // pub js_code: String,
    // #[serde(default)]
    // pub pages_selector: String,
    // #[serde(default)]
    // pub events: Vec<Event>,
}

/// The complete structure of a scraper or schema configuration file.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct FullConfig {
    pub config: ScraperConfig,
    pub selectors: Selectors,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct GenericContentRule {
    pub r#format: String,
    pub keep_link: bool,
    pub keep_image: bool,
}

impl Default for GenericContentRule {
    fn default() -> Self {
        GenericContentRule {
            r#format: "markdown".to_string(),
            keep_link: true,
            keep_image: false,
        }
    }
}
