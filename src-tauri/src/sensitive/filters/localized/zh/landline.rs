use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting Chinese Landline phone numbers.
pub struct LandlineFilter {
    regex: Regex,
}

impl LandlineFilter {
    /// Creates a new `LandlineFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"\b0\d{2,3}[-]?\d{7,8}\b"#).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "zh_landline_regex".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Self { regex })
    }
}

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
        let candidates = self
            .regex
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
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_landline() {
        let filter = LandlineFilter::new().unwrap();
        let text = "联系电话：010-85650764";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            text.get(result[0].start..result[0].end).unwrap(),
            "010-85650764"
        );
    }
}
