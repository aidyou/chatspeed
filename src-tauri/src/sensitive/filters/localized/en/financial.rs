use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting English Financial information.
pub struct FinancialFilter;

static MONEY_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches patterns like "Total Fee is $10,000" or "Amount of USD 5,000"
    // Added supports for "is", "of", "totaling".
    Regex::new(r#"(?i)(?:Total\s+Fee|Total\s+Amount|Sum|Price|Payment|Paid|Amount)(?:[:：]|\s+is\s+|\s+of\s+|\s+totaling\s+|\s)*([$¥£€]?[0-9.,]+\s*(?:USD|RMB|EUR|GBP|dollars|million|billion)?)"#).unwrap()
});

static BANK_ACCOUNT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches "Account Number: XXXX", "IBAN: XXXX", "SWIFT: XXXX"
    Regex::new(r#"(?i)(?:Account\s+Number|IBAN|SWIFT|Sort\s+Code)[:：]\s*([A-Z0-9\s-]{8,34})"#).unwrap()
});

impl SensitiveDataFilter for FinancialFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishFinancial"
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

        for cap in MONEY_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }

        for cap in BANK_ACCOUNT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: "BankAccount",
                    confidence: 0.95,
                });
            }
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        10
    }
}
