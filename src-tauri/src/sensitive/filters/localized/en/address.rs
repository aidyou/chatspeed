use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting US Addresses.
/// Address detection is complex. This filter uses a heuristic approach based on common patterns:
/// Street Number + Street Name + Street Type + (Optional Apt) + City + State + Zip
pub struct AddressFilter;

// A simplified regex for US addresses.
static US_ADDRESS_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\b\d+\s+[A-Za-z0-9\s.,]+(?:St|Ave|Blvd|Rd|Ln|Dr|Way|Ct|Plz|Pl)\.?\s*,?\s*[A-Za-z\s]+,?\s*[A-Z]{2}\s+\d{5}(?:-\d{4})?\b"#).unwrap()
});

static ADDRESS_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common address labels in English contracts
    Regex::new(r#"(?i)(?:Address|Registered\s+Office|Located\s+at|Principal\s+Place\s+of\s+Business)[:：]\s*([^\n\r]+)"#).unwrap()
});

impl SensitiveDataFilter for AddressFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishAddress"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let mut candidates = Vec::new();

        // 1. Context-based matching (High confidence)
        for cap in ADDRESS_CONTEXT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }

        // 2. Pattern-based matching (Existing)
        for m in US_ADDRESS_REGEX.find_iter(text) {
            candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.7,
            });
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        50 // Medium priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_address() {
        let filter = AddressFilter;
        let text = "I live at 123 Main St, New York, NY 10001.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        let match_text = &text[result[0].start..result[0].end];
        assert_eq!(match_text, "123 Main St, New York, NY 10001");
    }

    #[test]
    fn test_filter_address_variations() {
        let filter = AddressFilter;
        let text = "Ship to 456 Elm Ave., Los Angeles, CA 90210 please.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}
