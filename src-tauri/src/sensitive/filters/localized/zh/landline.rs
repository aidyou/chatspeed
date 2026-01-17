use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese Landline phone numbers.
/// Matches formats like: 010-12345678, 021-12345678, 0755-12345678
pub struct LandlineFilter;

static LANDLINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Area code: 0 followed by 2-3 digits (e.g., 010, 021, 0755)
    // Separator: optional hyphen
    // Number: 7-8 digits
    Regex::new(r#"\b0\d{2,3}[-]?\d{7,8}\b"#).unwrap()
});

impl SensitiveDataFilter for LandlineFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseLandline"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = LANDLINE_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.9,
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // High priority, distinct from mobile
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_landline() {
        let filter = LandlineFilter;
        let text = "联系电话：010-85650764";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 15); // Byte index
        assert_eq!(text.get(result[0].start..result[0].end).unwrap(), "010-85650764");
    }

    #[test]
    fn test_filter_landline_no_hyphen() {
        let filter = LandlineFilter;
        let text = "电话:057128102475";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(text.get(result[0].start..result[0].end).unwrap(), "057128102475");
    }
}
