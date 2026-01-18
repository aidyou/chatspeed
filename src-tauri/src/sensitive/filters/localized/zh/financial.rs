use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting Chinese Financial information (Amounts and Bank Accounts).
pub struct FinancialFilter {
    money_regex: Regex,
    bank_account_regex: Regex,
    bank_name_regex: Regex,
}

impl FinancialFilter {
    /// Creates a new `FinancialFilter` and pre-compiles its regexes.
    pub fn new() -> Result<Self, SensitiveError> {
        let money_regex = Regex::new(
            r#"(?i)(?:总费用|总金额|总款项|支付|即|共计|合计|报酬|金额|费用)(?:[:：]|为|是|合计|共计|\s)*([0-9.,]+[元万亿整]+(?:（\p{Han}+）)?)"#,
        )
        .map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "zh_financial_money".to_string(),
            message: e.to_string(),
        })?;

        let bank_account_regex =
            Regex::new(r#"(?:账号|卡号|银行账号|账户)[:：]\s*([0-9\s-]{12,25})"#).map_err(|e| {
                SensitiveError::RegexCompilationFailed {
                    pattern: "zh_financial_bank_account".to_string(),
                    message: e.to_string(),
                }
            })?;

        let bank_name_regex = Regex::new(r#"(?:开户行|开户银行|收款行)[:：]\s*([\p{Han}]{4,25})"#)
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "zh_financial_bank_name".to_string(),
                message: e.to_string(),
            })?;

        Ok(Self {
            money_regex,
            bank_account_regex,
            bank_name_regex,
        })
    }
}

impl SensitiveDataFilter for FinancialFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseFinancial"
    }

    fn produced_types(&self) -> Vec<&'static str> {
        vec!["ChineseFinancial", "BankAccount", "BankName"]
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
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

        for cap in self.bank_name_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: "BankName",
                    confidence: 0.9,
                });
            }
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        10
    }
}
