use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Defines a single search result item.
/// The `skip_serializing_if` attribute ensures that `Option` fields
/// are omitted from the JSON output when they are `None`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Defines the time period for a search query.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchPeriod {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// Defines the parameters for a search query.
/// This struct is deserialized from a `serde_json::Value` for flexibility,
/// especially for AI tool use cases.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// The search query string.
    pub query: String,
    /// The number of search results to return.
    pub count: Option<u32>,
    /// The language for the search, e.g., "en-US".
    pub language: Option<String>,
    /// The time period for the search, e.g., "week", "month".
    pub period: Option<SearchPeriod>,
}

impl TryFrom<&Value> for SearchParams {
    type Error = anyhow::Error;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.clone())
            .map_err(|e| anyhow!("Failed to deserialize search params: {}", e))
    }
}

/// A generic trait for all search engine providers.
/// Any struct implementing this trait can be used as a search engine.
#[async_trait]
pub trait SearchProvider {
    /// Executes the search.
    ///
    /// # Arguments
    ///
    /// * `params` - A `serde_json::Value` that can be deserialized into `SearchParams`.
    ///   This provides flexibility for extending parameters in the future.
    ///
    /// # Returns
    ///
    /// A `Result` containing a vector of `SearchResult` items or an error.
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>>;
}
