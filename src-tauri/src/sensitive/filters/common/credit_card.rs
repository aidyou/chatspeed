use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting common credit card numbers (Visa, Mastercard).
pub struct CreditCardFilter {
    regex: Regex,
}

impl CreditCardFilter {
    /// Creates a new `CreditCardFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"\b(?:4\d{3}|5[1-5]\d{2})[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "credit_card_regex".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for CreditCardFilter {
    fn filter_type(&self) -> &'static str {
        "CreditCard"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new() // Language-agnostic
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = self
            .regex
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 1.0,
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_visa() {
        let filter = CreditCardFilter::new().unwrap();
        let text = "My Visa is 4123-4567-8901-2345.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_mastercard() {
        let filter = CreditCardFilter::new().unwrap();
        let text = "Mastercard: 5123 4567 8901 2345";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}