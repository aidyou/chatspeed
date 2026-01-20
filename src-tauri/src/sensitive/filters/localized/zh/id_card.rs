use crate::sensitive::{
    error::SensitiveError,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting Chinese ID Card numbers (15 or 18-digit).
pub struct IdCardFilter {
    regex: Regex,
}

impl IdCardFilter {
    /// Creates a new `IdCardFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let pattern = r#"(?:[1-9]\d{5}(?:18|19|20)\d{2}(?:(?:0[1-9])|(?:1[0-2]))(?:(?:[0-2][1-9])|10|20|30|31)\d{3}[\dXx]|[1-9]\d{5}\d{2}(?:(?:0[1-9])|(?:1[0-2]))(?:(?:[0-2][1-9])|10|20|30|31)\d{3})"#;
        let regex = Regex::new(pattern).map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "id_card_regex".to_string(),
            message: e.to_string(),
        })?;
        Ok(Self { regex })
    }
}

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
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_id_card() {
        let filter = IdCardFilter::new().unwrap();
        let text = "他的身份证号码是44010619990101123X。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(
            text.get(candidate.start..candidate.end).unwrap(),
            "44010619990101123X"
        );
    }
}
