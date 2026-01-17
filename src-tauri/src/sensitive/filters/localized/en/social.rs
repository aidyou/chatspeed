use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting English/International Social Media handles.
pub struct SocialFilter;

static SOCIAL_HANDLE_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common handles preceded by platform name.
    // Includes: WeChat, WhatsApp, Telegram, Skype, Twitter, X, Instagram.
    Regex::new(r#"(?i)(?:WeChat|WhatsApp|Telegram|Skype|Twitter|Instagram|ID)[:：]?\s*([a-zA-Z0-9_-]{4,32})"#).unwrap()
});

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
        let candidates = SOCIAL_HANDLE_REGEX
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
