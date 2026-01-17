use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting international credit card numbers from various providers.
pub struct InternationalCreditCardFilter;

// This regex combines patterns for major international credit card providers.
// It allows for numbers to be contiguous, or separated by spaces or hyphens.
static INTERNATIONAL_CREDIT_CARD_REGEX: Lazy<Regex> = Lazy::new(|| {
    let patterns = vec![
        // Visa: starts with 4, length 13 or 16 digits
        r#"\b4\d{3}(?:[-\s]?\d{4}){2,3}\b"#,
        // Mastercard: starts with 51-55 or 2221-2720, length 16 digits
        r#"\b(5[1-5]\d{2}|222[1-9]|22[3-9]\d|2[3-6]\d{2}|27[01]\d|2720)(?:[-\s]?\d{4}){3}\b"#,
        // American Express: starts with 34 or 37, length 15 digits
        r#"\b3[47]\d{2}[-\s]?\d{6}[-\s]?\d{5}\b"#,
        // Discover: starts with 6011, 65, 644-649, length 16 digits
        r#"\b(6011\d{12}|65\d{14}|644\d{13}|645\d{13}|646\d{13}|647\d{13}|648\d{13}|649\d{13})\b"#,
        // JCB: starts with 3528-3589, length 16 digits
        r#"\b(352[8-9]\d|35[3-8]\d|358\d)(?:[-\s]?\d{4}){3}\b"#,
        // Diners Club: starts with 36 or 38, length 14-16 digits
        r#"\b(3[068]\d{11,13})(?:[-\s]?\d{0,3})?\b"#,
    ];
    Regex::new(&patterns.join("|")).unwrap()
});

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
        let candidates = INTERNATIONAL_CREDIT_CARD_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 1.0, // Regex matches are certain
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // High priority, as it's critical financial PII
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visa_no_spaces() {
        let filter = InternationalCreditCardFilter;
        let text = "Card: 4916123456789012";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 6);
        assert_eq!(result[0].end, 22);
        assert_eq!(result[0].filter_type, "InternationalCreditCard");
    }

    #[test]
    fn test_mastercard_new_range() {
        let filter = InternationalCreditCardFilter;
        let text = "Pay with 2221 0000 0000 0009 please.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 9);
        assert_eq!(result[0].end, 28);
    }

    #[test]
    fn test_discover_card() {
        let filter = InternationalCreditCardFilter;
        let text = "Discover: 6011 1234 5678 9012";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 10);
        assert_eq!(result[0].end, 29);
    }

    #[test]
    fn test_jcb_card() {
        let filter = InternationalCreditCardFilter;
        let text = "JCB 3530-1113-3330-0000";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 4);
        assert_eq!(result[0].end, 23);
    }

    #[test]
    fn test_diners_club() {
        let filter = InternationalCreditCardFilter;
        let text = "Diners 38520000023237";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 7);
        assert_eq!(result[0].end, 21);
    }

    #[test]
    fn test_multiple_cards() {
        let filter = InternationalCreditCardFilter;
        let text = "First 4916123456789012, then 3530111333300000.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].filter_type, "InternationalCreditCard");
        assert_eq!(result[1].filter_type, "InternationalCreditCard");
    }

    #[test]
    fn test_no_credit_cards() {
        let filter = InternationalCreditCardFilter;
        let text = "This text has no sensitive card numbers.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_invalid_card_number() {
        let filter = InternationalCreditCardFilter;
        // Starts with a 63, which doesn't match our patterns.
        let text = "Invalid card: 6300 1234 5678 9012";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }
}