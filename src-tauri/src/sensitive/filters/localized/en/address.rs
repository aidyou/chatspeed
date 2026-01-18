use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct AddressFilter {
    us_address_regex: Regex,
    address_context_regex: Regex,
}

impl AddressFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let us_address_regex = Regex::new(r#"\b\d+\s+[A-Za-z0-9\s.,]+(?:St|Ave|Blvd|Rd|Ln|Dr|Way|Ct|Plz|Pl)\.?\s*,?\s*[A-Za-z\s]+,?\s*[A-Z]{2}\s+\d{5}(?:-\d{4})?\b"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_address_pattern".to_string(), message: e.to_string() })?;
        let address_context_regex = Regex::new(r#"(?i)(?:Address|Registered\s+Office|Located\s+at|Principal\s+Place\s+of\s+Business)[:：]\s*([^\n\r]+)"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_address_context".to_string(), message: e.to_string() })?;
        Ok(Self {
            us_address_regex,
            address_context_regex,
        })
    }
}

impl SensitiveDataFilter for AddressFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishAddress"
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
        for cap in self.address_context_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }
        for m in self.us_address_regex.find_iter(text) {
            candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.7,
            });
        }
        Ok(candidates)
    }
    fn priority(&self) -> u32 {
        50
    }
}
