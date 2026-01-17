use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese Project names.
pub struct ProjectFilter;

static PROJECT_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common Chinese project labels in contracts.
    // Includes: 项目名称, 工程名称, 课题名称, 项目, 标段名称.
    Regex::new(r#"(?:项目名称|工程名称|课题名称|项目|标段名称)[:：]\s*([^\n\r]+)""#).unwrap()
});

impl SensitiveDataFilter for ProjectFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseProject"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = PROJECT_CONTEXT_REGEX
            .captures_iter(text)
            .filter_map(|cap| cap.get(1))
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
        45 // Consistent with other institution/context filters
    }
}
