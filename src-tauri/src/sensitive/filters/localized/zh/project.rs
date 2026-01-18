use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct ProjectFilter {
    regex: Regex,
}

impl ProjectFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"(?:项目名称|工程名称|课题名称|项目|标段名称)[:：]\s*([^\n\r]+)"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "zh_project_regex".to_string(), message: e.to_string() })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for ProjectFilter {
    fn filter_type(&self) -> &'static str { "ChineseProject" }
    fn supported_languages(&self) -> Vec<&'static str> { vec!["zh", "zh-Hans", "zh-Hant"] }
    fn filter(&self, text: &str, _language: &str) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = self.regex.captures_iter(text).filter_map(|cap| cap.get(1)).map(|m| FilterCandidate { start: m.start(), end: m.end(), filter_type: self.filter_type(), confidence: 0.9 }).collect();
        Ok(candidates)
    }
    fn priority(&self) -> u32 { 45 }
}