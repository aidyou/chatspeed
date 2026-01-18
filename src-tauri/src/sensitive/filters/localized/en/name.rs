use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::{Regex, RegexSet};

/// A filter for detecting English names based on patterns and context.
pub struct NameFilter {
    name_pattern: Regex,
    context_keywords: RegexSet,
}

impl NameFilter {
    /// Creates a new `NameFilter` and pre-compiles its regexes.
    pub fn new() -> Result<Self, SensitiveError> {
        let name_pattern = Regex::new(r#"\b[A-Z][a-z]+(?:\s[A-Z][a-z]+)*\b"#).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "en_name_pattern".to_string(),
                message: e.to_string(),
            }
        })?;

        let context_keywords = RegexSet::new(&[
            r"(?i)my name is",
            r"(?i)i am",
            r"(?i)this is",
            r"(?i)call me",
            r"(?i)name[:：]",
            r"(?i)contact[:：]",
        ])
        .map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "en_name_context".to_string(),
            message: e.to_string(),
        })?;

        Ok(Self {
            name_pattern,
            context_keywords,
        })
    }
}

impl SensitiveDataFilter for NameFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishName"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let mut candidates = Vec::new();

        for m in self.name_pattern.find_iter(text) {
            let mut confidence = 0.4;

            let start = m.start();
            let context_start = start.saturating_sub(20);
            let context_window = &text[context_start..start];

            if self.context_keywords.is_match(context_window) {
                confidence += 0.5;
            }

            if confidence >= 0.8 {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence,
                });
            }
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_name_with_context() {
        let filter = NameFilter::new().unwrap();
        let text = "My name is John Smith.";
        let result = filter.filter(text, "en").unwrap();
        assert!(!result.is_empty());
    }
}
