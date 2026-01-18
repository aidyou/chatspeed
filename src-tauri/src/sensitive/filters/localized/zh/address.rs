use crate::sensitive::error::SensitiveError;
use crate::sensitive::traits::{FilterCandidate, SensitiveDataFilter};
use regex::Regex;

pub struct AddressFilter {
    context_regex: Regex,
    pattern_regex: Regex,
}

impl AddressFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let context_regex = Regex::new(r#"(?:地址|住址|所在地|住所地|注册地址|通讯地址)[:：]\s*([^\n\s]+(?:省|市|区|县|街|路|号|大厦|层|室|单元|栋)[^\n\s]*)"#).map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "zh_address_context".to_string(), message: e.to_string() })?;
        let pattern_regex = Regex::new(
            r#"(?:\p{Han}{2,}(?:省|市|区|县))[\p{Han}0-9-]{2,}(?:路|街|道|号|院|大厦|广场|中心)"#,
        )
        .map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "zh_address_pattern".to_string(),
            message: e.to_string(),
        })?;
        Ok(Self {
            context_regex,
            pattern_regex,
        })
    }
}

impl SensitiveDataFilter for AddressFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseAddress"
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
        for cap in self.context_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.95,
                });
            }
        }
        for m in self.pattern_regex.find_iter(text) {
            candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.8,
            });
        }
        Ok(candidates)
    }
    fn priority(&self) -> u32 {
        50
    }
}
