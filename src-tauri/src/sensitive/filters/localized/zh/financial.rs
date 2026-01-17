use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese Financial information (Amounts and Bank Accounts).
pub struct FinancialFilter;

static MONEY_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches patterns like "总费用为9万元整" or "支付金额是1000元"
    // Added keywords like "合计", "共计" and connectors like "为", "是".
    Regex::new(r#"(?i)(?:总费用|总金额|总款项|支付|即|共计|合计|报酬|金额|费用)(?:[:：]|为|是|合计|共计|\s)*([0-9.,]+[元万亿整]+(?:（[\u{4e00}-\u{9fa5}]+）)?)"#).unwrap()
});

static BANK_ACCOUNT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches "账号：1202 xxx6 ..."
    Regex::new(r#"(?:账号|卡号|银行账号|账户)[:：]\s*([0-9\s-]{12,25})"#).unwrap()
});

static BANK_NAME_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches "开户行：xxx支行"
    Regex::new(r#"(?:开户行|开户银行|收款行)[:：]\s*([\u{4e00}-\u{9fa5}]{4,25})"#).unwrap()
});

impl SensitiveDataFilter for FinancialFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseFinancial"
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

        // 1. Filter Money
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

        // 2. Filter Bank Accounts
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

        // 3. Filter Bank Names
        for cap in BANK_NAME_REGEX.captures_iter(text) {
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
        10 // High priority
    }
}
