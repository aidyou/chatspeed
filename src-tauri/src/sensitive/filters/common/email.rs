use crate::sensitive::{
    error::{SensitiveError,},
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting email addresses.
pub struct EmailFilter;

// Using once_cell::sync::Lazy for thread-safe, one-time regex compilation.
static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b"#).unwrap()
});

impl SensitiveDataFilter for EmailFilter {
    /// Returns the identifier for the filter type.
    fn filter_type(&self) -> &'static str {
        "Email"
    }

    /// This is a common filter, so it supports all languages.
    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Executes the email filter.
    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = EMAIL_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 1.0, // Regex matches are considered certain.
            })
            .collect();
        Ok(candidates)
    }

    /// Priority for the email filter.
    fn priority(&self) -> u32 {
        // High priority as emails are a very common and clear-cut PII.
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_single_email() {
        let filter = EmailFilter;
        let text = "Contact me at john.doe@example.com for more information.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 14);
        assert_eq!(result[0].end, 34);
        assert_eq!(result[0].filter_type, "Email");
    }

    #[test]
    fn test_filter_multiple_emails() {
        let filter = EmailFilter;
        let text = "My emails are test@test.com and another@example.org.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start, 14);
        assert_eq!(result[0].end, 27); // Including the word boundary after .com
        assert_eq!(result[1].start, 32);
        assert_eq!(result[1].end, 51); // Including the word boundary after .org
    }

    #[test]
    fn test_no_emails() {
        let filter = EmailFilter;
        let text = "There is no email address in this text.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_email_with_special_chars() {
        let filter = EmailFilter;
        let text = "Special one: user+alias@sub.domain.co.uk";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 13);
        assert_eq!(result[0].end, 40); // Including the word boundary after .uk
    }

    #[test]
    fn test_false_positives() {
        let filter = EmailFilter;
        // This text looks a bit like an email but isn't one.
        let text = "This is not an email: user@localhost";
        let result = filter.filter(text, "en").unwrap();
        // The regex requires a TLD of at least 2 chars, so "localhost" is not matched.
        assert!(result.is_empty());
    }
}
