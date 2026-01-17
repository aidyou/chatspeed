use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting credit card numbers for major providers.
pub struct CreditCardFilter;

// This regex combines patterns for Visa, Mastercard, and American Express.
// It allows for numbers to be contiguous, or separated by spaces or hyphens.
static CREDIT_CARD_REGEX: Lazy<Regex> = Lazy::new(|| {
    let visa_pattern = r#"\b4\d{3}(?:[-\s]?\d{4}){3}\b"#;
    let mastercard_pattern = r#"\b5[1-5]\d{2}(?:[-\s]?\d{4}){3}\b"#;
    let amex_pattern = r#"\b3[47]\d{2}[-\s]?\d{6}[-\s]?\d{5}\b"#;
    Regex::new(&format!(
        "{}|{}|{}",
        visa_pattern, mastercard_pattern, amex_pattern
    ))
    .unwrap()
});

impl SensitiveDataFilter for CreditCardFilter {
    fn filter_type(&self) -> &'static str {
        "CreditCard"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = CREDIT_CARD_REGEX
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
        // High priority, as it's critical financial PII.
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visa_no_spaces() {
        let filter = CreditCardFilter;
        let text = "Card: 4916123456789012";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 6);
        assert_eq!(result[0].end, 22);
        assert_eq!(result[0].filter_type, "CreditCard");
    }

    #[test]
    fn test_mastercard_with_spaces() {
        let filter = CreditCardFilter;
        let text = "Pay with 5222 1234 5678 9012 please.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 9);
        assert_eq!(result[0].end, 28);
    }

    #[test]
    fn test_amex_with_hyphens() {
        let filter = CreditCardFilter;
        let text = "Amex card 3782-822463-12345 is on file.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 10);
        assert_eq!(result[0].end, 27);
    }
    
    #[test]
    fn test_multiple_cards() {
        let filter = CreditCardFilter;
        let text = "First 4916123456789012, then 5111-1111-1111-1111.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].filter_type, "CreditCard");
        assert_eq!(result[1].filter_type, "CreditCard");
    }

    #[test]
    fn test_no_credit_cards() {
        let filter = CreditCardFilter;
        let text = "This text has no sensitive card numbers.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_invalid_card_number() {
        let filter = CreditCardFilter;
        // Starts with a 6, which doesn't match our patterns.
        let text = "Invalid card: 6011 1234 5678 9012";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }
}
