use crate::sensitive::{
    error::SensitiveError,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting Chinese mobile phone numbers (11-digit).
pub struct MobileFilter {
    regex: Regex,
}

impl MobileFilter {
    /// Creates a new `MobileFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"1[3-9]\d{9}"#).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "zh_mobile_regex".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Self { regex })
    }
}

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
                    confidence: 0.9,
                }
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_mobile_number() {
        let filter = MobileFilter::new().unwrap();
        let text = "我的手机号是13812345678, 请联系我。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(
            text.get(candidate.start..candidate.end).unwrap(),
            "13812345678"
        );
    }
}
