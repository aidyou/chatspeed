use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting North American / English style phone numbers.
pub struct MobileFilter;

// This regex matches various common formats for North American phone numbers.
// e.g., (123) 456-7890, 123-456-7890, 123.456.7890, 123 456 7890, 1234567890
// It also optionally handles a +1 country code prefix.
static MOBILE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\b(?:\+?1[-\s.]?)?\(?\d{3}\)?[-\s.]?\d{3}[-\s.]?\d{4}\b"#).unwrap());

impl SensitiveDataFilter for MobileFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishMobile"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        // A simple heuristic: if the text contains many numbers, it's less likely to be a specific phone number.
        // This is a placeholder for more advanced logic. For now, we just run the regex.
        let candidates = MOBILE_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.85, // High, but not 1.0 as it could be other numeric data.
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        2 // Same priority as ChineseMobile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_standard_format() {
        let filter = MobileFilter;
        let text = "Call me at 123-456-7890.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 12);
        assert_eq!(result[0].end, 24);
        assert_eq!(result[0].filter_type, "EnglishMobile");
    }

    #[test]
    fn test_filter_with_parentheses_and_spaces() {
        let filter = MobileFilter;
        let text = "My number is (123) 456-7890 now.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 13);
        assert_eq!(result[0].end, 27);
    }
    
    #[test]
    fn test_filter_with_country_code() {
        let filter = MobileFilter;
        let text = "It's +1 987.654.3210 for international.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 6);
        assert_eq!(result[0].end, 21);
    }

    #[test]
    fn test_contiguous_digits() {
        let filter = MobileFilter;
        let text = "Number: 1234567890";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 8);
        assert_eq!(result[0].end, 18);
    }

    #[test]
    fn test_no_mobile_numbers() {
        let filter = MobileFilter;
        let text = "This is a regular sentence.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_number_too_long() {
        let filter = MobileFilter;
        let text = "Too long: 12345678901"; // 11 digits, doesn't fit the 10-digit pattern without formatting
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }
}
