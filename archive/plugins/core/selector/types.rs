use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for CSS selector
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectorConfig {
    /// HTML content to parse
    pub html: String,
    /// Map of named selectors, where key is the field name and value is the CSS selector
    pub selectors: HashMap<String, SelectorRule>,
}

/// Rule for CSS selector extraction
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectorRule {
    /// CSS selector string
    pub selector: String,
    /// Content extraction mode
    pub extract: ExtractMode,
    /// Attributes to extract from matched elements
    pub attributes: Option<Vec<String>>,
}

/// Content extraction mode
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractMode {
    /// Extract only text content
    Text,
    /// Extract inner HTML
    Html,
    /// Extract outer HTML (including the matched element)
    OuterHtml,
}

/// Results from selector execution
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectorResult {
    /// Map of field name to extracted values
    pub fields: HashMap<String, Vec<ElementData>>,
}

/// Data extracted from a single element
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ElementData {
    /// Content based on extraction mode (text/html/outer_html)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Map of attribute name to value
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub attributes: HashMap<String, String>,
}
