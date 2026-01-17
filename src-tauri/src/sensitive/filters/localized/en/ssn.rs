use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting US Social Security Numbers (SSN).
pub struct SsnFilter;

// Matches SSN format: XXX-XX-XXXX, XXX XX XXXX, or XXXXXXXXX
// To reduce false positives, we primarily target the hyphenated format or context-aware matches.
// This regex focuses on the standard hyphenated format: \b\d{3}-\d{2}-\d{4}\b
static SSN_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\b(?!000|666|9\d{2})\d{3}[-\s]?(?!00)\d{2}[-\s]?(?!0000)\d{4}\b"#).unwrap()
});

impl SensitiveDataFilter for SsnFilter {
    fn filter_type(&self) -> &'static str {
        "SSN"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = SSN_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.9, // High confidence for standard format
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        5 // High priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_ssn() {
        let filter = SsnFilter;
        let text = "My SSN is 123-45-6789.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 10);
        assert_eq!(result[0].end, 21);
    }

    #[test]
    fn test_filter_ssn_space() {
        let filter = SsnFilter;
        let text = "Number: 123 45 6789";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}
