use super::error::SensitiveError;

/// Represents a candidate for a sensitive data match.
#[derive(Debug)]
pub struct FilterCandidate {
    pub start: usize,
    pub end: usize,
    pub filter_type: &'static str,
    /// Confidence score for the match, from 0.0 to 1.0.
    pub confidence: f32,
}

/// The core trait for all sensitive data filters.
pub trait SensitiveDataFilter {
    /// Identifier for the filter.
    fn filter_type(&self) -> &'static str;

    /// Returns a list of all candidate types this filter can produce.
    /// Default implementation returns just the [filter_type].
    fn produced_types(&self) -> Vec<&'static str> {
        vec![self.filter_type()]
    }

    /// List of supported languages. An empty vector means it's a common filter.
    fn supported_languages(&self) -> Vec<&'static str>;

    /// Executes the filter on the given text.
    ///
    /// # Arguments
    /// * `text` - The input text to scan.
    /// * `language` - The language of the text.
    ///
    /// # Returns
    /// A `Result` containing a vector of `FilterCandidate`s if successful,
    /// or a `SensitiveError` if an error occurs.
    fn filter(&self, text: &str, language: &str)
        -> std::result::Result<Vec<FilterCandidate>, SensitiveError>;

    /// Returns the priority of the filter.
    /// Lower values have higher priority.
    fn priority(&self) -> u32 {
        100
    }
}
