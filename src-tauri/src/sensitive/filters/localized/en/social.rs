use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct SocialFilter {
    regex: Regex,
}

impl SocialFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"(?i)(?:WeChat|WhatsApp|Telegram|Skype|Twitter|Instagram|ID)[:：]?\s*([a-zA-Z0-9_-]{4,32})"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_social_regex".to_string(), message: e.to_string() })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for SocialFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishSocial"
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
            .captures_iter(text)
            .filter_map(|cap| cap.get(1))
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.85,
            })
            .collect();
        Ok(candidates)
    }
    fn priority(&self) -> u32 {
        15
    }
}
