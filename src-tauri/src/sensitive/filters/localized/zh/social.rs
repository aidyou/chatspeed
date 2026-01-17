use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese Social Media accounts (WeChat, QQ).
pub struct SocialFilter;

static WECHAT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches "微信：xxx", "微信号：xxx", "WeChat：xxx"
    // WeChat IDs: 6-20 characters, starts with a letter, can contain numbers, hyphens, and underscores.
    Regex::new(r#"(?i)(?:微信|微信号|WeChat)[:：]?\s*([a-zA-Z][a-zA-Z0-9_-]{5,19})"#).unwrap()
});

static QQ_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches "QQ：xxx"
    // QQ numbers: 5-12 digits.
    Regex::new(r#"(?i)QQ[:：]?\s*([1-9][0-9]{4,11})"#).unwrap()
});

impl SensitiveDataFilter for SocialFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseSocial"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let mut candidates = Vec::new();

        // 1. WeChat
        for cap in WECHAT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: "WeChatID",
                    confidence: 0.9,
                });
            }
        }

        // 2. QQ
        for cap in QQ_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: "QQNumber",
                    confidence: 0.9,
                });
            }
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        15 // Below Phone and Email
    }
}
