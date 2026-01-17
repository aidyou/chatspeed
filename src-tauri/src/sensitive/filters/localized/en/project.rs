use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting English Project names.
pub struct ProjectFilter;

static PROJECT_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common English project labels in contracts.
    // Includes: Project Name, Project Title, Project, Scheme Name, Study Title.
    Regex::new(r#"(?i)(?:Project\s+Name|Project\s+Title|Project|Scheme\s+Name|Study\s+Title)[:：]\s*([^\n\r]+)"#).unwrap()
});

impl SensitiveDataFilter for ProjectFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishProject"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
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
        45 // Consistent with Chinese
    }
}
