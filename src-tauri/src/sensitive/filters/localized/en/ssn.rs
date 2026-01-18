use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting US Social Security Numbers (SSN).
pub struct SsnFilter {
    regex: Regex,
}

impl SsnFilter {
    /// Creates a new `SsnFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"\b\d{3}[-\s]?\d{2}[-\s]?\d{4}\b"#).map_err(|e| {
            SensitiveError::RegexCompilationFailed {
                pattern: "en_ssn_regex".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for SsnFilter {
    fn filter_type(&self) -> &'static str {
        "SSN"
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
            .find_iter(text)
            .filter(|m| {
                // Post-process to exclude invalid SSN sequences
                let s = m.as_str().replace(['-', ' '], "");
                if s.len() != 9 {
                    return false;
                }

                let area = &s[0..3];
                let group = &s[3..5];
                let serial = &s[5..9];

                if area == "000" || area == "666" || area.starts_with('9') {
                    return false;
                }
                if group == "00" {
                    return false;
                }
                if serial == "0000" {
                    return false;
                }
                true
            })
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
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_ssn() {
        let filter = SsnFilter::new().unwrap();
        let text = "My SSN is 123-45-6789.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}
