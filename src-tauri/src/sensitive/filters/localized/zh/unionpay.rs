use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting UnionPay card numbers.
pub struct UnionPayFilter {
    regex: Regex,
}

impl UnionPayFilter {
    /// Creates a new `UnionPayFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let patterns = vec![
            r#"\b62\d{14,17}\b"#,
            r#"\b(?:62\d{2})[-\s]\d{4,6}[-\s]\d{4,6}[-\s]\d{4,6}\b"#,
        ];
        let regex = Regex::new(&patterns.join("|")).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "unionpay_regex".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for UnionPayFilter {
    fn filter_type(&self) -> &'static str {
        "UnionPay"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
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
                confidence: 1.0,
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_unionpay() {
        let filter = UnionPayFilter::new().unwrap();
        let text = "银行卡号：6225888888888888";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
    }
}
