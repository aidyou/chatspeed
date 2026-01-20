use crate::sensitive::{
    error::SensitiveError,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting email addresses.
pub struct EmailFilter {
    regex: Regex,
}

impl EmailFilter {
    /// Creates a new `EmailFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let regex = Regex::new(r#"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "email_regex".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for EmailFilter {
    fn filter_type(&self) -> &'static str {
        "Email"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new() // Language-agnostic
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = self
            .regex
            .find_iter(text)
            .map(|m| {
                let start = adjust_to_char_boundary(text, m.start());
                let end = adjust_to_char_boundary(text, m.end());
                FilterCandidate {
                    start,
                    end,
                    filter_type: self.filter_type(),
                    confidence: 1.0, // Email pattern is very reliable.
                }
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_valid_email() {
        let filter = EmailFilter::new().unwrap();
        let text = "My email is test@example.com, contact me.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(candidate.start, 12);
        assert_eq!(candidate.end, 28);
        assert_eq!(candidate.filter_type, "Email");
    }

    #[test]
    fn test_multiple_emails() {
        let filter = EmailFilter::new().unwrap();
        let text = "Contact: first@test.com, second@test.com.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_no_email() {
        let filter = EmailFilter::new().unwrap();
        let text = "This is a sentence with no email addresses.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_invalid_email_format() {
        let filter = EmailFilter::new().unwrap();
        let text = "This is not an email: test@example, @example.com, test@.com.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }
}