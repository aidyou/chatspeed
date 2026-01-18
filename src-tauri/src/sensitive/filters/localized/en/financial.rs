use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

pub struct FinancialFilter {
    money_regex: Regex,
    bank_account_regex: Regex,
}

impl FinancialFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        let money_regex = Regex::new(r#"(?i)(?:Total\s+Fee|Total\s+Amount|Sum|Price|Payment|Paid|Amount)(?:[:：]|\s+is\s+|\s+of\s+|\s+totaling\s+|\s)*([$¥£€]?[0-9.,]+\s*(?:USD|RMB|EUR|GBP|dollars|million|billion)?)"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_financial_money".to_string(), message: e.to_string() })?;
        let bank_account_regex = Regex::new(
            r#"(?i)(?:Account\s+Number|IBAN|SWIFT|Sort\s+Code)[:：]\s*([A-Z0-9\s-]{8,34})"#,
        )
        .map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "en_financial_bank".to_string(),
            message: e.to_string(),
        })?;
        Ok(Self {
            money_regex,
            bank_account_regex,
        })
    }
}

impl SensitiveDataFilter for FinancialFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishFinancial"
    }

    fn produced_types(&self) -> Vec<&'static str> {
        vec!["EnglishFinancial", "BankAccount"]
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
        for cap in self.money_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }
        for cap in self.bank_account_regex.captures_iter(text) {
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
