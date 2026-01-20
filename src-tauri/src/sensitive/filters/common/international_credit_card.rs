use crate::sensitive::{
    error::SensitiveError,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting international credit card numbers (Amex, Discover, etc.).
pub struct InternationalCreditCardFilter {
    regex: Regex,
}

impl InternationalCreditCardFilter {
    /// Creates a new `InternationalCreditCardFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let patterns = vec![
            r#"\b3[47]\d{2}[-\s]?\d{6}[-\s]?\d{5}\b"#, // Amex
            r#"\b6(?:011|5\d{2})[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b"#, // Discover
            r#"\b(?:30[0-5]|36\d|38\d)[-\s]?\d{4}[-\s]?\d{6}\b"#, // Diners Club
            r#"\b(?:2131|1800|35(?:2[89]|[3-8][0-9]|90))[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b"#, // JCB
        ];
        let regex = Regex::new(&patterns.join("|")).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "international_credit_card_regex".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for InternationalCreditCardFilter {
    fn filter_type(&self) -> &'static str {
        "InternationalCreditCard"
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
            .map(|m| {
                let start = adjust_to_char_boundary(text, m.start());
                let end = adjust_to_char_boundary(text, m.end());
                FilterCandidate {
                    start,
                    end,
                    filter_type: self.filter_type(),
                    confidence: 1.0,
                }
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
    fn test_filter_amex() {
        let filter = InternationalCreditCardFilter::new().unwrap();
        let text = "My Amex is 3412-345678-90123.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_jcb() {
        let filter = InternationalCreditCardFilter::new().unwrap();
        let text = "JCB card: 3528 1234 5678 9012";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}
