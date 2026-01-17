use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese mobile phone numbers (11-digit).
pub struct MobileFilter;

// This regex matches 11-digit numbers starting with '1' and a second digit from 3 to 9.
// Added \b boundary to avoid matching sub-strings in longer digit sequences.
static MOBILE_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Match 1[3-9] followed by 9 digits
    Regex::new(r#"\b1[3-9]\d{9}\b"#).unwrap()
});

impl SensitiveDataFilter for MobileFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseMobile"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = MOBILE_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.9, // High, but slightly less certain than an ID card.
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // High priority, just below ID cards.
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_mobile_number() {
        let filter = MobileFilter;
        let text = "我的手机号是13812345678, 请联系我。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(candidate.start, 18); // Byte position of "1"
        assert_eq!(candidate.end, 29);   // Byte position after "8"
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "13812345678");
        assert_eq!(candidate.filter_type, "ChineseMobile");
    }

    #[test]
    fn test_filter_number_at_string_start() {
        let filter = MobileFilter;
        let text = "15987654321是我的号码。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 0);
        assert_eq!(result[0].end, 11);
    }

    #[test]
    fn test_no_mobile_number() {
        let filter = MobileFilter;
        let text = "这里没有手机号码，只有一个11位的数字12345678901。";
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty()); // "12..." does not start with 1[3-9]
    }

    #[test]
    fn test_number_too_short() {
        let filter = MobileFilter;
        let text = "太短了：1381234567。"; // 10 digits
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty());
    }
}
