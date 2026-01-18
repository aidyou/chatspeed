use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct SocialFilter {
    wechat_regex: Regex,
    qq_regex: Regex,
}

impl SocialFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let wechat_regex = Regex::new(r#"(?i)(?:微信|微信号|WeChat)[:：]?\s*([a-zA-Z][a-zA-Z0-9_-]{5,19})"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "zh_social_wechat".to_string(), message: e.to_string() })?;
        let qq_regex = Regex::new(r#"(?i)QQ[:：]?\s*([1-9][0-9]{4,11})"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "zh_social_qq".to_string(), message: e.to_string() })?;
        Ok(Self { wechat_regex, qq_regex })
    }
}

impl SensitiveDataFilter for SocialFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseSocial"
    }

    fn produced_types(&self) -> Vec<&'static str> {
        vec!["WeChatID", "QQNumber"]
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }
    fn filter(&self, text: &str, _language: &str) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let mut candidates = Vec::new();
        for cap in self.wechat_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) { candidates.push(FilterCandidate { start: m.start(), end: m.end(), filter_type: "WeChatID", confidence: 0.9 }); }
        }
        for cap in self.qq_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) { candidates.push(FilterCandidate { start: m.start(), end: m.end(), filter_type: "QQNumber", confidence: 0.9 }); }
        }
        Ok(candidates)
    }
    fn priority(&self) -> u32 { 15 }
}