use std::{fmt::Display, str::FromStr};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::search::{BuiltInSearch, GoogleSearch, SerperSearch, TavilySearch};

/// Defines a single search result item.
/// The `skip_serializing_if` attribute ensures that `Option` fields
/// are omitted from the JSON output when they are `None`.
#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    #[serde(default, skip_deserializing, skip_serializing_if = "is_zero")]
    pub id: usize,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sitename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub score: Option<f32>,
}

impl Display for SearchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<cs:webpage>\n")?;

        if self.id > 0 {
            write!(f, "<cs:id>{}</cs:id>\n", self.id)?;
        }

        write!(
            f,
            "<cs:title>{}</cs:title>\n<cs:url>{}</cs:url>\n",
            self.title, self.url
        )?;

        let mut write_content = false;
        if let Some(c) = self.content.as_ref() {
            if !c.is_empty() {
                write!(f, "<cs:content>{}</cs:content>\n", c)?;
                write_content = true;
            }
        }
        if !write_content {
            if let Some(s) = self.snippet.as_ref() {
                write!(f, "<cs:snippet>{}</cs:snippet>\n", s)?;
            }
        }
        if let Some(s) = self.sitename.as_ref() {
            write!(f, "<cs:sitename>{}</cs:sitename>\n", s)?;
        }
        if let Some(pb) = self.publish_date.as_ref() {
            write!(f, "<cs:publish_date>{}</cs:publish_date>\n", pb)?;
        }
        if let Some(s) = self.score.as_ref() {
            write!(f, "<cs:score>{}</cs:score>\n", s)?;
        }
        write!(f, "</cs:webpage>\n")?;
        Ok(())
    }
}

fn is_zero<T: PartialEq + From<u8>>(value: &T) -> bool {
    *value == T::from(0)
}

#[derive(PartialEq, Clone, Debug)]
pub enum SearchProviderName {
    Bing,
    Brave,
    DuckDuckGo,
    So,
    Sogou,
    Google,
    Serper,
    Tavily,
}

impl Display for SearchProviderName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bing => write!(f, "bing"),
            Self::Brave => write!(f, "brave"),
            Self::DuckDuckGo => write!(f, "duckduckgo"),
            Self::So => write!(f, "so"),
            Self::Sogou => write!(f, "sogou"),
            Self::Google => write!(f, "google"),
            Self::Serper => write!(f, "serper"),
            Self::Tavily => write!(f, "tavily"),
        }
    }
}

impl From<SearchProviderName> for String {
    fn from(provider: SearchProviderName) -> Self {
        provider.to_string()
    }
}

impl FromStr for SearchProviderName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl TryFrom<&str> for SearchProviderName {
    type Error = String;

    fn try_from(provider: &str) -> Result<Self, Self::Error> {
        match provider {
            "bing" => Ok(Self::Bing),
            "brave" => Ok(Self::Brave),
            "duckduckgo" => Ok(Self::DuckDuckGo),
            "so" => Ok(Self::So),
            "sogou" => Ok(Self::Sogou),
            "tavily" => Ok(Self::Tavily),
            "google" => Ok(Self::Google),
            "serper" => Ok(Self::Serper),
            _ => Err(format!("Invalid search provider: {}", provider)),
        }
    }
}

impl TryFrom<String> for SearchProviderName {
    type Error = String;

    fn try_from(provider: String) -> Result<Self, Self::Error> {
        Self::try_from(provider.as_str())
    }
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
    pub page: Option<u32>,
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

pub enum SearchFactory {
    Google(GoogleSearch),
    Serper(SerperSearch),
    Tavily(TavilySearch),
    Builtin(BuiltInSearch),
}

#[async_trait]
impl SearchProvider for SearchFactory {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        let result = match self {
            Self::Google(gs) => gs.search(params).await,
            Self::Serper(sp) => sp.search(params).await,
            Self::Tavily(t) => t.search(params).await,
            Self::Builtin(b) => b.search(params).await,
        };
        result
    }
}

impl Display for SearchFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Google(_) => write!(f, "Google"),
            Self::Serper(_) => write!(f, "Serper"),
            Self::Tavily(_) => write!(f, "Tavily"),
            Self::Builtin(b) => write!(f, "Builtin({})", b.provider),
        }
    }
}
