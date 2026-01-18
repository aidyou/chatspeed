use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct MobileFilter {
    regex: Regex,
}

impl MobileFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"\b(?:\+?1[-\s.]?)?\(?\d{3}\)?[-\s.]?\d{3}[-\s.]?\d{4}\b"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "en_mobile_regex".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { regex })
    }
}

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
        2
    }
}
