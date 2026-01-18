use rust_i18n::t;
use std::collections::HashMap;

/// Get the localized replacement text for a given filter type.
pub fn get_replacement_text(filter_type: &str, language: &str) -> String {
    match filter_type {
        "Email" => t!("sensitive.filter.email", locale = language).to_string(),
        "IPAddress" => t!("sensitive.filter.ip_address", locale = language).to_string(),
        "CreditCard" => t!("sensitive.filter.credit_card", locale = language).to_string(),
        "InternationalCreditCard" => {
            t!("sensitive.filter.international_credit_card", locale = language).to_string()
        }
        "UnionPay" => t!("sensitive.filter.unionpay", locale = language).to_string(),
        "ChineseIDCard" => t!("sensitive.filter.chinese_id_card", locale = language).to_string(),
        "ChineseMobile" => t!("sensitive.filter.chinese_mobile", locale = language).to_string(),
        "ChineseName" => t!("sensitive.filter.chinese_name", locale = language).to_string(),
        "EnglishMobile" => t!("sensitive.filter.english_mobile", locale = language).to_string(),
        "EnglishName" => t!("sensitive.filter.english_name", locale = language).to_string(),
        "ChineseLandline" => t!("sensitive.filter.chinese_landline", locale = language).to_string(),
        "ChineseAddress" => t!("sensitive.filter.chinese_address", locale = language).to_string(),
        "EnglishAddress" => t!("sensitive.filter.english_address", locale = language).to_string(),
        "ChineseCompany" => t!("sensitive.filter.chinese_company", locale = language).to_string(),
        "EnglishCompany" => t!("sensitive.filter.english_company", locale = language).to_string(),
        "ChineseProject" => t!("sensitive.filter.chinese_project", locale = language).to_string(),
        "EnglishProject" => t!("sensitive.filter.english_project", locale = language).to_string(),
        "ChineseFinancial" => {
            t!("sensitive.filter.chinese_financial", locale = language).to_string()
        }
        "EnglishFinancial" => {
            t!("sensitive.filter.english_financial", locale = language).to_string()
        }
        "BankAccount" => t!("sensitive.filter.bank_account", locale = language).to_string(),
        "BankName" => t!("sensitive.filter.bank_name", locale = language).to_string(),
        "WeChatID" => t!("sensitive.filter.wechat_id", locale = language).to_string(),
        "QQNumber" => t!("sensitive.filter.qq_number", locale = language).to_string(),
        "CustomBlock" => t!("sensitive.filter.custom_block", locale = language).to_string(),
        _ => format!("[{}]", filter_type),
    }
}

/// A cache to store already translated filter types to avoid repeated lookups
pub struct ReplacementTextCache {
    cache: HashMap<String, String>,
    language: String,
}

impl ReplacementTextCache {
    /// Create a new cache for the specified language
    pub fn new(language: &str) -> Self {
        Self {
            cache: HashMap::new(),
            language: language.to_string(),
        }
    }

    /// Get the replacement text, using cache if available
    pub fn get(&mut self, filter_type: &str) -> String {
        let cache_key = format!("{}:{}", self.language, filter_type);

        if let Some(text) = self.cache.get(&cache_key) {
            text.clone()
        } else {
            let text = get_replacement_text(filter_type, &self.language);
            self.cache.insert(cache_key, text.clone());
            text
        }
    }
}
