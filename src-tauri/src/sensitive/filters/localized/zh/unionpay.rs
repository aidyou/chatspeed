use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese UnionPay (银联) card numbers.
pub struct UnionPayFilter;

// Chinese UnionPay cards typically have 16-19 digits.
// They start with 62 (assigned to China UnionPay) or other bank-specific prefixes.
static UNIONPAY_REGEX: Lazy<Regex> = Lazy::new(|| {
    // UnionPay cards start with 62 and have exactly 16-19 digits
    // We need to ensure we don't match partial numbers
    // Put longer patterns first to ensure full match
    let patterns = vec![
        r#"62\d{18}"#,  // 19 digits
        r#"62\d{17}"#,  // 18 digits
        r#"62\d{16}"#,  // 17 digits
        r#"62\d{15}"#,  // 16 digits
        r#"62\d{3}[-\s]\d{4}[-\s]\d{4}[-\s]\d{4}"#, // With spaces/hyphens
    ];
    Regex::new(&patterns.join("|")).unwrap()
});

impl SensitiveDataFilter for UnionPayFilter {
    fn filter_type(&self) -> &'static str {
        "UnionPay"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = UNIONPAY_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.95, // High confidence as the pattern is specific
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // High priority, similar to other financial filters
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unionpay_16_digits() {
        let filter = UnionPayFilter;
        let text = "6217000010001234567"; // Just the card number
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 0);
        assert_eq!(result[0].end, 19); // 19 digits
        assert_eq!(result[0].filter_type, "UnionPay");
    }

    #[test]
    fn test_unionpay_with_spaces() {
        let filter = UnionPayFilter;
        let text = "卡号: 6225 8888 8888 8888";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 3);
        assert_eq!(result[0].end, 22);
    }

    #[test]
    fn test_unionpay_with_hyphens() {
        let filter = UnionPayFilter;
        let text = "银行卡6225888888888888-123";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 3);
        assert_eq!(result[0].end, 22);
    }

    #[test]
    fn test_no_unionpay_cards() {
        let filter = UnionPayFilter;
        let text = "这不是银行卡号：1234567890123456";
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_not_supported_language() {
        let filter = UnionPayFilter;
        let text = "My card is 6217000010001234567";
        // The filter should not run for "en"
        let supported = filter.supported_languages();
        assert!(!supported.contains(&"en"));
    }
}