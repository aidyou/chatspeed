use super::{
    filters::{
        common::{
            credit_card::CreditCardFilter, email::EmailFilter,
            international_credit_card::InternationalCreditCardFilter, ip_address::IpAddressFilter,
        },
        localized::{
            en::{
                address::AddressFilter, company::CompanyFilter as EnCompanyFilter,
                financial::FinancialFilter as EnFinancialFilter, mobile as en_mobile,
                name as en_name, project::ProjectFilter as EnProjectFilter,
                social::SocialFilter as EnSocialFilter, ssn::SsnFilter,
            },
            zh::{
                address::AddressFilter as ZhAddressFilter, company::CompanyFilter,
                financial::FinancialFilter as ZhFinancialFilter, id_card::IdCardFilter,
                landline::LandlineFilter, mobile::MobileFilter, name::NameFilter,
                project::ProjectFilter as ZhProjectFilter, social::SocialFilter as ZhSocialFilter,
                unionpay::UnionPayFilter,
            },
        },
    },
    i18n_support::ReplacementTextCache,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use log::error;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SensitiveConfig {
    pub enabled: bool,
    pub common_enabled: bool,
    pub custom_blocklist: Vec<String>,
    pub allowlist: Vec<String>,
}

impl Default for SensitiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            common_enabled: true,
            custom_blocklist: Vec::new(),
            allowlist: Vec::new(),
        }
    }
}

/// Manages the registration and execution of sensitive data filters.
pub struct FilterManager {
    /// A vector holding all registered filters as trait objects.
    filters: Vec<Box<dyn SensitiveDataFilter + Send + Sync>>,
}

impl FilterManager {
    /// Creates a new `FilterManager` and registers all available filters.
    pub fn new() -> Self {
        let mut filters: Vec<Box<dyn SensitiveDataFilter + Send + Sync>> = Vec::new();

        // Register common filters
        filters.push(Box::new(EmailFilter));
        filters.push(Box::new(IpAddressFilter));
        filters.push(Box::new(CreditCardFilter));
        filters.push(Box::new(InternationalCreditCardFilter));

        // Register localized filters
        filters.push(Box::new(IdCardFilter));
        filters.push(Box::new(MobileFilter)); // Chinese
        filters.push(Box::new(LandlineFilter)); // Chinese
        filters.push(Box::new(NameFilter)); // Chinese
        filters.push(Box::new(UnionPayFilter)); // Chinese
        filters.push(Box::new(ZhAddressFilter)); // Chinese
        filters.push(Box::new(CompanyFilter)); // Chinese
        filters.push(Box::new(ZhProjectFilter)); // Chinese
        filters.push(Box::new(ZhFinancialFilter)); // Chinese
        filters.push(Box::new(ZhSocialFilter)); // Chinese
        filters.push(Box::new(en_mobile::MobileFilter)); // English
        filters.push(Box::new(en_name::NameFilter)); // English
        filters.push(Box::new(SsnFilter)); // English
        filters.push(Box::new(AddressFilter)); // English
        filters.push(Box::new(EnCompanyFilter)); // English
        filters.push(Box::new(EnProjectFilter)); // English
        filters.push(Box::new(EnFinancialFilter)); // English
        filters.push(Box::new(EnSocialFilter)); // English

        // Sort filters by priority (lower number = higher priority)
        filters.sort_by_key(|f| f.priority());

        Self { filters }
    }

    /// This is the main entry point for filtering text.
    /// The full implementation will follow the "Unified Filtering Process"
    /// described in the plan.md document.
    ///
    /// # Arguments
    /// * `text` - The text to be sanitized.
    /// * `languages` - The languages of the text (e.g. detected + system).
    /// * `config` - The configuration for sensitive data filtering.
    ///
    /// # Returns
    /// A sanitized version of the text.
    pub fn filter_text(&self, text: &str, languages: &[&str], config: &SensitiveConfig) -> String {
        if !config.enabled {
            return text.to_string();
        }

        let mut all_candidates = Vec::new();
        // Use the first language for replacement text cache, or default to "en"
        let primary_lang = languages.first().copied().unwrap_or("en");
        let mut replacement_cache = ReplacementTextCache::new(primary_lang);

        // 1. Run built-in filters
        for filter in &self.filters {
            // Check common filter switch
            let supported_langs = filter.supported_languages();
            if supported_langs.is_empty() && !config.common_enabled {
                continue;
            }

            // Check language support: Run if filter is common OR supports ANY of the provided languages
            if supported_langs.is_empty() || supported_langs.iter().any(|sl| languages.contains(sl)) {
                // We pass the primary language to the filter, though most don't use it for matching logic
                match filter.filter(text, primary_lang) {
                    Ok(mut candidates) => all_candidates.append(&mut candidates),
                    Err(e) => {
                        error!("Filter '{}' failed: {}", filter.filter_type(), e);
                    }
                }
            }
        }

        // 2. Run custom blocklist (Simple exact match for now)
        for block_word in &config.custom_blocklist {
            if block_word.is_empty() {
                continue;
            }
            for m in text.match_indices(block_word) {
                all_candidates.push(FilterCandidate {
                    start: m.0,
                    end: m.0 + block_word.len(),
                    filter_type: "CustomBlock",
                    confidence: 1.0,
                });
            }
        }

        // 3. Filter out allowlisted items
        // We remove candidates that are *inside* or *equal to* allowlist items.
        // Or if the candidate text *contains* an allowlist item?
        // Usually, allowlist means "If this exact string appears, don't mask it".
        // So if a candidate range covers an allowlist item, we might need to split the candidate or discard it?
        // Simplest logic: If a candidate's text *is* in the allowlist, discard it.
        // Better logic: If the allowlist item is found in the text, any candidate overlapping with it is invalid?
        // Let's go with: If a candidate range *overlaps* with an allowlist match, discard the candidate.
        if !config.allowlist.is_empty() {
            let mut allow_ranges = Vec::new();
            for allow_word in &config.allowlist {
                if allow_word.is_empty() {
                    continue;
                }
                for m in text.match_indices(allow_word) {
                    allow_ranges.push(m.0..m.0 + allow_word.len());
                }
            }

            all_candidates.retain(|c| {
                let c_range = c.start..c.end;
                !allow_ranges
                    .iter()
                    .any(|a_range| ranges_overlap(c_range.clone(), a_range.clone()))
            });
        }

        // Tier 3: Decision & Conflict Resolution
        let final_candidates = Self::resolve_conflicts(all_candidates, text.len());

        // Tier 4: Unified Replacement
        let mut sanitized_text = text.to_string();
        // Iterate backwards to ensure indices remain valid after replacement.
        for candidate in final_candidates.iter().rev() {
            let placeholder = if candidate.filter_type == "CustomBlock" {
                "[BLOCK]".to_string()
            } else {
                replacement_cache.get(candidate.filter_type)
            };
            sanitized_text.replace_range(candidate.start..candidate.end, &placeholder);
        }

        sanitized_text
    }

    /// Resolves conflicts among overlapping candidates. The winner is chosen based on:
    /// 1. Higher confidence score.
    /// 2. Higher priority (lower number).
    /// 3. Longer match length.
    fn resolve_conflicts(
        mut candidates: Vec<super::traits::FilterCandidate>,
        text_len: usize,
    ) -> Vec<super::traits::FilterCandidate> {
        // Sort by 'best' candidate first.
        // Order: 1. Longer match length (greedy) 2. Higher confidence score 3. Higher priority (lower number)
        candidates.sort_by(|a, b| {
            (b.end - b.start)
                .cmp(&(a.end - a.start))
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| find_priority(a.filter_type).cmp(&find_priority(b.filter_type)))
        });

        let mut final_candidates = Vec::new();
        let mut is_covered = vec![false; text_len];

        for candidate in candidates {
            if !is_covered[candidate.start..candidate.end]
                .iter()
                .any(|&x| x) {
                // No overlap, this is a good candidate.
                for i in candidate.start..candidate.end {
                    is_covered[i] = true;
                }
                final_candidates.push(candidate);
            }
        }

        // Sort by start index for replacement.
        final_candidates.sort_by_key(|c| c.start);
        final_candidates
    }

    /// Returns a list of all supported filter types.
    pub fn supported_filter_types(&self) -> Vec<String> {
        self.filters
            .iter()
            .map(|f| f.filter_type().to_string())
            .collect()
    }
}

fn ranges_overlap(a: std::ops::Range<usize>, b: std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

// Helper to get priority for conflict resolution. In a real app, this might
// come from a shared registry or the filter instance itself.
fn find_priority(filter_type: &str) -> u32 {
    match filter_type {
        "CustomBlock" => 0, // Highest priority
        "ChineseIDCard" => 1,
        "SSN" => 1,
        "ChineseMobile" => 2,
        "EnglishMobile" => 2,
        "ChineseLandline" => 3,
        "UnionPay" => 3, // Chinese UnionPay cards
        "CreditCard" => 5,
        "InternationalCreditCard" => 5,
        "Email" => 10,
        "IPAddress" => 10,
        "ChineseFinancial" => 10,
        "EnglishFinancial" => 10,
        "BankAccount" => 10,
        "BankName" => 10,
        "ChineseSocial" => 15,
        "EnglishSocial" => 15,
        "WeChatID" => 15,
        "QQNumber" => 15,
        "ChineseCompany" => 40,
        "EnglishCompany" => 40,
        "ChineseProject" => 45,
        "EnglishProject" => 45,
        "ChineseAddress" => 50,
        "EnglishAddress" => 50,
        "ChineseName" => 60,
        "EnglishName" => 60,
        "Longer" => 20,
        "Shorter" => 20,
        _ => 100,
    }
}

impl Default for FilterManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitive::{error::SensitiveError, traits::FilterCandidate};

    #[test]
    fn test_manager_creation_and_registration() {
        let manager = FilterManager::new();
        // Check if all filters are registered.
        assert!(manager.filters.len() >= 10);
        assert!(manager
            .filters
            .iter()
            .any(|f| f.filter_type() == "EnglishName"));
    }

    #[test]
    fn test_basic_text_replacement() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "This is a test with an email: test@example.com.";
        let expected_text = "This is a test with an email: [邮箱].";
        let filtered_text = manager.filter_text(original_text, &["zh-Hans"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_multiple_replacements() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "Emails: first@test.com and second@test.com.";
        let expected_text = "Emails: [Email] and [Email].";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_mixed_filter_types() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "Contact me at my_email@example.com or on server 192.168.1.1.";
        let expected_text = "Contact me at [Email] or on server [IPAddress].";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_credit_card_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "My card is 4916-1234-5678-9012.";
        let expected_text = "My card is [CreditCard].";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_language_specific_zh_id_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "我的身份证是44010619990101123X。";
        
        // Should filter when language is "zh"
        let expected_text_zh = "我的身份证是[ChineseIDCard]。";
        let filtered_text_zh = manager.filter_text(original_text, &["zh"], &config);
        assert_eq!(expected_text_zh, filtered_text_zh);

        // Should NOT filter when language is "en"
        let filtered_text_en = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(original_text, filtered_text_en);
    }

    #[test]
    fn test_language_specific_zh_mobile_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "我的手机是13812345678。";
        
        // Should filter when language is "zh"
        let expected_text_zh = "我的手机是[ChineseMobile]。";
        let filtered_text_zh = manager.filter_text(original_text, &["zh"], &config);
        assert_eq!(expected_text_zh, filtered_text_zh);

        // Should NOT filter when language is "en"
        let filtered_text_en = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(original_text, filtered_text_en);
    }
    
    #[test]
    fn test_language_specific_en_mobile_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "My number is (123) 456-7890.";
        
        // Should filter when language is "en"
        let expected_text_en = "My number is [EnglishMobile].";
        let filtered_text_en = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text_en, filtered_text_en);

        // Should NOT filter when language is "zh"
        let filtered_text_zh = manager.filter_text(original_text, &["zh"], &config);
        assert_eq!(original_text, filtered_text_zh);
    }

    #[test]
    fn test_heuristic_name_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        // With strong context, it should be filtered.
        let original_text = "你好，我叫王伟。";
        let expected_text = "你好，我叫[ChineseName]。";
        let filtered_text = manager.filter_text(original_text, &["zh"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_language_specific_en_name_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let original_text = "My name is John Smith.";
        let expected_text = "My name is [EnglishName].";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);

        // Should not trigger on Chinese text
        let filtered_text_zh = manager.filter_text(original_text, &["zh"], &config);
        assert_eq!(original_text, filtered_text_zh);
    }

    #[test]
    fn test_custom_blocklist() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();
        config.custom_blocklist.push("ProjectX".to_string());
        
        let original_text = "Do not leak ProjectX status.";
        let expected_text = "Do not leak [BLOCK] status.";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_allowlist() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();
        config.allowlist.push("192.168.1.1".to_string());
        
        // 192.168.1.1 should usually be filtered by IpAddressFilter.
        // But since it's in allowlist, it should remain.
        let original_text = "My local IP is 192.168.1.1.";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(original_text, filtered_text);

        // Other IPs should still be filtered
        let mixed_text = "IPs: 192.168.1.1 and 10.0.0.1.";
        let expected_mixed = "IPs: 192.168.1.1 and [IPAddress].";
        let filtered_mixed = manager.filter_text(mixed_text, &["en"], &config);
        assert_eq!(expected_mixed, filtered_mixed);
    }

    #[test]
    fn test_config_disabled() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();
        config.enabled = false;
        
        let original_text = "My email is test@example.com";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(original_text, filtered_text);
    }

    // Mocks for overlap testing
    struct MockFilter {
        filter_type: &'static str,
        priority: u32,
        matches: Vec<(usize, usize)>,
    }

    impl SensitiveDataFilter for MockFilter {
        fn filter_type(&self) -> &'static str {
            self.filter_type
        }
        fn supported_languages(&self) -> Vec<&'static str> {
            Vec::new()
        }
        fn filter(
            &self,
            _text: &str,
            _language: &str,
        ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
            Ok(self
                .matches
                .iter()
                .map(|(start, end)| FilterCandidate {
                    start: *start,
                    end: *end,
                    filter_type: self.filter_type,
                    confidence: 1.0,
                })
                .collect())
        }
        fn priority(&self) -> u32 {
            self.priority
        }
    }

    #[test]
    fn test_overlapping_candidates_longest_wins() {
        let mut manager = FilterManager::new();
        manager.filters.clear(); // Clear default filters
        let config = SensitiveConfig::default();

        // Longer match has the same priority, so it should win.
        let shorter_filter = MockFilter {
            filter_type: "Shorter",
            priority: 20,
            matches: vec![(5, 10)], // "is is"
        };
        let longer_filter = MockFilter {
            filter_type: "Longer",
            priority: 20,
            matches: vec![(5, 15)], // "is is a te"
        };

        manager.filters.push(Box::new(shorter_filter));
        manager.filters.push(Box::new(longer_filter));

        let original_text = "This is is a test.";
        let expected_text = "This [Longer]st.";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_overlapping_candidates_priority_wins() {
        let mut manager = FilterManager::new();
        manager.filters.clear();
        let config = SensitiveConfig::default();

        // high_priority_filter is shorter, but its priority is higher (lower number).
        let high_priority_filter = MockFilter {
            filter_type: "ChineseIDCard", // priority 1
            priority: 1,
            matches: vec![(10, 20)],
        };
        let low_priority_filter = MockFilter {
            filter_type: "Longer", // priority 20
            priority: 20,
            matches: vec![(5, 25)],
        };

        manager.filters.push(Box::new(high_priority_filter));
        manager.filters.push(Box::new(low_priority_filter));

        let original_text = "Text text text text text text"; // Length 29
        let expected_text = "Text [ChineseIDCard] text text";
        let filtered_text = manager.filter_text(original_text, &["en"], &config);
        assert_eq!(expected_text, filtered_text);
    }

    #[test]
    fn test_chinese_agreement_filtering() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        
        let agreement_text = r#"\
委托协议

甲方：北京科技有限公司
法定代表人：张三
地址：北京市海淀区中关村大街1号
联系电话：13812345678
邮箱：zhangsan@example.com
身份证号：110101199001011234
银行卡号：6225888888888888

乙方：上海创新科技有限公司
法定代表人：李四
地址：上海市浦东新区世纪大道100号
联系电话：13987654321
邮箱：lisi@company.com
身份证号：310101198502022345
银行卡号：6217000010001234567

协议内容：
1. 甲方委托乙方进行技术开发
2. 合同金额：100,000元
3. 支付方式：银行转账至账号6225888888888888
4. 联系人：王五，电话13812345678

签署日期：2024年1月1日
"#;
        
        let filtered_text = manager.filter_text(agreement_text, &["zh-Hans"], &config);
        
        // 先打印过滤后的文本
        println!("\n过滤后的文本:\n{}", filtered_text);
        
        // 调试：检查各个过滤器是否工作
        let simple_text = "张三 13812345678 zhangsan@example.com";
        let simple_filtered = manager.filter_text(simple_text, &["zh-Hans"], &config);
        println!("\n简单测试 - 原文: {}", simple_text);
        println!("简单测试 - 过滤后: {}", simple_filtered);
        
        // 验证敏感信息被过滤
        assert!(filtered_text.contains("[邮箱]"));
        assert!(filtered_text.contains("[身份证号]"));
        
        // 验证非敏感信息保留
        assert!(filtered_text.contains("委托协议"));
        assert!(filtered_text.contains("技术开发"));
        assert!(filtered_text.contains("100,000元"));
        assert!(filtered_text.contains("2024年1月1日"));
        
        println!("原始文本长度: {}", agreement_text.len());
        println!("过滤后文本长度: {}", filtered_text.len());
        println!("\n过滤后的文本:\n{}", filtered_text);
    }
}