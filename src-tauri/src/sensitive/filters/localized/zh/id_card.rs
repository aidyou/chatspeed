use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese ID Card numbers (18-digit).
pub struct IdCardFilter;

// This regex is designed to match 15 or 18-digit Chinese ID card format.
// 18-digit: standard format with checksum.
// 15-digit: old format (all digits).
static ID_CARD_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\b(?:[1-9]\d{5}(?:18|19|20)\d{2}(?:(?:0[1-9])|(?:1[0-2]))(?:(?:[0-2][1-9])|10|20|30|31)\d{3}[\dXx]|[1-9]\d{5}\d{2}(?:(?:0[1-9])|(?:1[0-2]))(?:(?:[0-2][1-9])|10|20|30|31)\d{3})\b"#).unwrap()
});

impl SensitiveDataFilter for IdCardFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseIDCard"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = ID_CARD_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 1.0, // The pattern is very specific.
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // Very high priority due to its specificity and sensitivity.
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_id_card() {
        let filter = IdCardFilter;
        let text = "他的身份证号码是44010619990101123X。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(candidate.start, 7);
        assert_eq!(candidate.end, 25);
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "44010619990101123X");
        assert_eq!(candidate.filter_type, "ChineseIDCard");
    }

    #[test]
    fn test_filter_id_card_without_context() {
        let filter = IdCardFilter;
        let text = "这是一串数字110101198001011234。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 6);
        assert_eq!(result[0].end, 24);
    }

    #[test]
    fn test_no_id_card() {
        let filter = IdCardFilter;
        let text = "这是一个普通的句子，没有敏感信息。";
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_invalid_id_card_wrong_length() {
        let filter = IdCardFilter;
        let text = "错误的号码：12345678901234567。"; // 17 digits
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty());
    }
    
    #[test]
    fn test_not_supported_language() {
        let filter = IdCardFilter;
        let text = "His ID is 44010619990101123X.";
        // The filter should not run for "en"
        let supported = filter.supported_languages();
        assert!(!supported.contains(&"en"));
    }
}
