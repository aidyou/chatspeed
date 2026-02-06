use super::{
    filters::{
        common::{
            credit_card::CreditCardFilter, email::EmailFilter,
            international_credit_card::InternationalCreditCardFilter, ip_address::IpAddressFilter,
        },
        localized::{
            en::{
                address::AddressFilter as EnAddressFilter,
                company::CompanyFilter as EnCompanyFilter,
                financial::FinancialFilter as EnFinancialFilter,
                mobile::MobileFilter as EnMobileFilter, name::NameFilter as EnNameFilter,
                project::ProjectFilter as EnProjectFilter, social::SocialFilter as EnSocialFilter,
                ssn::SsnFilter,
            },
            zh::{
                address::AddressFilter as ZhAddressFilter,
                company::CompanyFilter as ZhCompanyFilter,
                financial::FinancialFilter as ZhFinancialFilter, id_card::IdCardFilter,
                landline::LandlineFilter, mobile::MobileFilter as ZhMobileFilter,
                name::NameFilter as ZhNameFilter, project::ProjectFilter as ZhProjectFilter,
                social::SocialFilter as ZhSocialFilter, unionpay::UnionPayFilter,
            },
        },
    },
    i18n_support::ReplacementTextCache,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use crate::error::Result;
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
    /// Indicates if the manager was successfully initialized.
    pub is_healthy: bool,
    /// Detailed error message if initialization failed.
    pub error_message: Option<String>,
}

impl FilterManager {
    /// Creates a new `FilterManager` and registers all available filters.
    /// Performs pre-compilation of all regexes.
    pub fn new() -> Self {
        let mut filters: Vec<Box<dyn SensitiveDataFilter + Send + Sync>> = Vec::new();

        let registration_res = (|| -> Result<()> {
            // Register common filters
            filters.push(Box::new(EmailFilter::new()?));
            filters.push(Box::new(IpAddressFilter::new()?));
            filters.push(Box::new(CreditCardFilter::new()?));
            filters.push(Box::new(InternationalCreditCardFilter::new()?));

            // Register localized filters (ZH)
            filters.push(Box::new(IdCardFilter::new()?));
            filters.push(Box::new(ZhMobileFilter::new()?));
            filters.push(Box::new(LandlineFilter::new()?));
            filters.push(Box::new(ZhNameFilter::new()?));
            filters.push(Box::new(UnionPayFilter::new()?));
            filters.push(Box::new(ZhAddressFilter::new()?));
            filters.push(Box::new(ZhCompanyFilter::new()?));
            filters.push(Box::new(ZhProjectFilter::new()?));
            filters.push(Box::new(ZhSocialFilter::new()?));
            filters.push(Box::new(ZhFinancialFilter::new()?));

            // Register localized filters (EN)
            filters.push(Box::new(EnMobileFilter::new()?));
            filters.push(Box::new(EnNameFilter::new()?));
            filters.push(Box::new(SsnFilter::new()?));
            filters.push(Box::new(EnAddressFilter::new()?));
            filters.push(Box::new(EnCompanyFilter::new()?));
            filters.push(Box::new(EnProjectFilter::new()?));
            filters.push(Box::new(EnFinancialFilter::new()?));
            filters.push(Box::new(EnSocialFilter::new()?));
            Ok(())
        })();

        match registration_res {
            Ok(_) => {
                // Sort filters by priority (lower number = higher priority)
                filters.sort_by_key(|f| f.priority());
                Self {
                    filters,
                    is_healthy: true,
                    error_message: None,
                }
            }
            Err(e) => Self {
                filters: Vec::new(),
                is_healthy: false,
                error_message: Some(e.to_string()),
            },
        }
    }

    /// This is the main entry point for filtering text.
    pub fn filter_text(&self, text: &str, languages: &[&str], config: &SensitiveConfig) -> String {
        if !self.is_healthy || !config.enabled {
            return text.to_string();
        }

        let mut all_candidates = Vec::new();
        let primary_lang = languages.first().copied().unwrap_or("en");
        let mut replacement_cache = ReplacementTextCache::new(primary_lang);

        // 1. Pre-process Allowlist (Sorted and merged ranges for efficient binary search)
        let allow_ranges = if !config.allowlist.is_empty() {
            let mut ranges = Vec::new();
            for allow_word in &config.allowlist {
                if allow_word.is_empty() {
                    continue;
                }
                for m in text.match_indices(allow_word) {
                    let start = adjust_to_char_boundary(text, m.0);
                    let end = adjust_to_char_boundary(text, m.0 + allow_word.len());
                    ranges.push(start..end);
                }
            }
            // Sort by start position
            ranges.sort_by_key(|r| r.start);
            // Merge overlapping allowlist ranges
            let mut merged = Vec::new();
            if let Some(mut current) = ranges.first().cloned() {
                for next in ranges.iter().skip(1) {
                    if next.start < current.end {
                        if next.end > current.end {
                            current.end = next.end;
                        }
                    } else {
                        merged.push(current);
                        current = next.clone();
                    }
                }
                merged.push(current);
            }
            merged
        } else {
            Vec::new()
        };

        // 2. Run built-in filters
        for filter in &self.filters {
            let supported_langs = filter.supported_languages();
            if supported_langs.is_empty() && !config.common_enabled {
                continue;
            }

            if supported_langs.is_empty() || supported_langs.iter().any(|sl| languages.contains(sl))
            {
                match filter.filter(text, primary_lang) {
                    Ok(mut candidates) => {
                        // Filter candidates by allowlist immediately using binary search
                        if !allow_ranges.is_empty() {
                            candidates.retain(|c| {
                                let c_range = c.start..c.end;
                                // Binary search for any allow_range that might overlap with c_range
                                let idx = allow_ranges.partition_point(|r| r.start < c_range.end);
                                if idx == 0 {
                                    true
                                } else {
                                    // Check the range at idx-1 which has the largest start < c_range.end
                                    !ranges_overlap(c_range.clone(), allow_ranges[idx - 1].clone())
                                }
                            });
                        }
                        all_candidates.append(&mut candidates);
                    }
                    Err(e) => {
                        error!("Filter '{}' failed: {}", filter.filter_type(), e);
                    }
                }
            }
        }

        // 3. Run custom blocklist
        for block_word in &config.custom_blocklist {
            if block_word.is_empty() {
                continue;
            }
            for m in text.match_indices(block_word) {
                let start = adjust_to_char_boundary(text, m.0);
                let end = adjust_to_char_boundary(text, m.0 + block_word.len());
                let c_range = start..end;
                // Custom blocks also respect allowlist
                let is_allowed = if !allow_ranges.is_empty() {
                    let idx = allow_ranges.partition_point(|r| r.start < c_range.end);
                    idx != 0 && ranges_overlap(c_range.clone(), allow_ranges[idx - 1].clone())
                } else {
                    false
                };

                if !is_allowed {
                    all_candidates.push(FilterCandidate {
                        start,
                        end,
                        filter_type: "CustomBlock",
                        confidence: 1.0,
                    });
                }
            }
        }

        let final_candidates = Self::resolve_conflicts(all_candidates);

        // Use cumulative offset method for replacement to avoid position changes after replacement
        let mut result = String::with_capacity(text.len());
        let mut last_end = 0;

        for candidate in final_candidates {
            let placeholder = if candidate.filter_type == "CustomBlock" {
                "[BLOCK]".to_string()
            } else {
                replacement_cache.get(candidate.filter_type)
            };

            // Ensure indices are valid character boundaries
            let start = adjust_to_char_boundary(text, candidate.start);
            let end = adjust_to_char_boundary(text, candidate.end);

            // Add text before the candidate
            if start > last_end {
                result.push_str(&text[last_end..start]);
            }

            // Add replacement text
            result.push_str(&placeholder);

            last_end = end;
        }

        // Add text after the last candidate
        if last_end < text.len() {
            result.push_str(&text[last_end..]);
        }

        result
    }

    /// Resolves conflicts among overlapping candidates.
    /// Memory efficiency: O(N) where N is number of candidates, independent of text length.
    fn resolve_conflicts(
        mut candidates: Vec<super::traits::FilterCandidate>,
    ) -> Vec<super::traits::FilterCandidate> {
        // Sort by 'best' candidate first.
        // Order: 1. Longer match length (greedy) 2. Higher confidence score 3. Higher priority (lower number)
        candidates.sort_by(|a, b| {
            (b.end - b.start)
                .cmp(&(a.end - a.start))
                .then_with(|| b.confidence.total_cmp(&a.confidence))
                .then_with(|| find_priority(a.filter_type).cmp(&find_priority(b.filter_type)))
        });

        let mut final_candidates = Vec::new();

        for candidate in candidates {
            let c_range = candidate.start..candidate.end;
            // No overlap check with already selected final_candidates
            if !final_candidates
                .iter()
                .any(|f: &FilterCandidate| ranges_overlap(c_range.clone(), f.start..f.end))
            {
                final_candidates.push(candidate);
            }
        }

        // Sort by start index for backward replacement.
        final_candidates.sort_by_key(|c| c.start);
        final_candidates
    }

    /// Returns a list of all supported filter types (flattened from all filters).
    pub fn supported_filter_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self
            .filters
            .iter()
            .flat_map(|f| f.produced_types().into_iter().map(|s| s.to_string()))
            .collect();

        types.sort();
        types.dedup();
        types
    }
}

fn ranges_overlap(a: std::ops::Range<usize>, b: std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn find_priority(filter_type: &str) -> u32 {
    match filter_type {
        "CustomBlock" => 0,
        "ChineseIDCard" => 1,
        "SSN" => 1,
        "ChineseMobile" => 2,
        "EnglishMobile" => 2,
        "ChineseLandline" => 3,
        "UnionPay" => 3,
        "CreditCard" => 5,
        "InternationalCreditCard" => 5,
        "Email" => 10,
        "IPAddress" => 10,
        "ChineseFinancial" => 10,
        "EnglishFinancial" => 10,
        "BankAccount" => 10,
        "BankName" => 10,
        "ChineseCompany" => 40,
        "EnglishCompany" => 40,
        "ChineseProject" => 45,
        "EnglishProject" => 45,
        "ChineseAddress" => 50,
        "EnglishAddress" => 50,
        "ChineseName" => 60,
        "EnglishName" => 60,
        _ => 100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitive::traits::FilterCandidate;

    #[test]
    fn test_filter_manager_creation() {
        let manager = FilterManager::new();
        assert!(manager.is_healthy);
    }

    #[test]
    fn test_filter_text_empty() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let result = manager.filter_text("", &["en"], &config);
        assert_eq!(result, "");
    }

    #[test]
    fn test_filter_text_no_sensitive_info() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();
        let text = "This is a normal sentence without any sensitive information.";
        let result = manager.filter_text(text, &["en"], &config);
        assert_eq!(result, text);
    }

    #[test]
    fn test_filter_text_multiple_replacements() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();

        // Text with multiple sensitive information that will be replaced with different length placeholders
        let text = "Contact: phone 13812345678, email test@example.com, IP 192.168.1.1";
        let result = manager.filter_text(text, &["zh", "en"], &config);

        // Should contain replacements but not panic
        assert!(
            result.contains("[ChineseMobile]")
                || result.contains("[Email]")
                || result.contains("[IPAddress]")
        );
        assert_ne!(result, text);
    }

    #[test]
    fn test_filter_text_unicode_boundary() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();

        // Text with Unicode characters and sensitive info
        let text = "中文测试📱手机号：13812345678，邮箱：test@example.com🎯";
        let result = manager.filter_text(text, &["zh"], &config);

        // Should not panic with Unicode characters
        assert!(result.len() > 0);
    }

    #[test]
    fn test_filter_text_overlapping_matches() {
        let manager = FilterManager::new();
        let config = SensitiveConfig::default();

        // Create a scenario with multiple detectable sensitive information
        // Using email and phone which are reliably detected
        let text = "Contact: test@example.com and phone: 13812345678";
        let result = manager.filter_text(text, &["zh", "en"], &config);

        // Should contain replacements
        assert!(result.contains("[Email]") || result.contains("[ChineseMobile]"));
        // Original sensitive info should be replaced
        assert!(!result.contains("test@example.com") || !result.contains("13812345678"));
    }

    #[test]
    fn test_filter_text_with_allowlist() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();

        // Add an email to allowlist
        config.allowlist.push("test@example.com".to_string());

        let text = "Allowed: test@example.com, Blocked: other@example.com";
        let result = manager.filter_text(text, &["en"], &config);

        // Allowed email should remain, other should be replaced
        assert!(result.contains("test@example.com"));
        assert!(result.contains("[Email]"));
    }

    #[test]
    fn test_filter_text_with_custom_blocklist() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();

        // Add custom blocklist
        config.custom_blocklist.push("secret".to_string());
        config.custom_blocklist.push("confidential".to_string());

        let text = "This is a secret document with confidential information.";
        let result = manager.filter_text(text, &["en"], &config);

        // Custom blocklist words should be replaced
        assert!(result.contains("[BLOCK]"));
        assert!(!result.contains("secret"));
        assert!(!result.contains("confidential"));
    }

    #[test]
    fn test_filter_text_disabled() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();
        config.enabled = false;

        let text = "Phone: 13812345678, Email: test@example.com";
        let result = manager.filter_text(text, &["zh", "en"], &config);

        // When disabled, should return original text
        assert_eq!(result, text);
    }

    #[test]
    fn test_filter_text_common_disabled() {
        let manager = FilterManager::new();
        let mut config = SensitiveConfig::default();
        config.common_enabled = false;

        // Test with only common filters (Email) and only localized filters (Chinese ID)
        // Email is common filter (supported_languages is empty)
        // Chinese ID is localized filter (supports zh)
        let text = "Email: test@example.com";
        let result = manager.filter_text(text, &["zh", "en"], &config);

        // When common_enabled = false, common filters should be disabled
        // Email should NOT be replaced since it's a common filter
        assert!(result.contains("test@example.com"));

        // Also test that localized filters still work when common filters are disabled
        let text2 = "身份证：44010619990101123X";
        let result2 = manager.filter_text(text2, &["zh"], &config);
        // Chinese ID should still be detected (it's a localized filter, not common)
        assert_ne!(result2, text2);
    }

    #[test]
    fn test_resolve_conflicts_basic() {
        let candidates = vec![
            FilterCandidate {
                start: 0,
                end: 10,
                filter_type: "Test1",
                confidence: 0.9,
            },
            FilterCandidate {
                start: 5,
                end: 15,
                filter_type: "Test2",
                confidence: 0.8,
            },
            FilterCandidate {
                start: 20,
                end: 30,
                filter_type: "Test3",
                confidence: 0.7,
            },
        ];

        let resolved = FilterManager::resolve_conflicts(candidates);

        // Should resolve overlapping conflicts (0-10 and 5-15 overlap)
        // The one with higher confidence (Test1) should win
        assert_eq!(resolved.len(), 2); // One overlapping pair resolved to one, plus non-overlapping
    }

    #[test]
    fn test_resolve_conflicts_same_confidence() {
        let candidates = vec![
            FilterCandidate {
                start: 0,
                end: 15, // Longer match
                filter_type: "Test1",
                confidence: 0.8,
            },
            FilterCandidate {
                start: 5,
                end: 10, // Shorter match
                filter_type: "Test2",
                confidence: 0.8, // Same confidence
            },
        ];

        let resolved = FilterManager::resolve_conflicts(candidates);

        // Longer match should win when confidence is equal
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].filter_type, "Test1");
    }

    #[test]
    fn test_supported_filter_types() {
        let manager = FilterManager::new();
        let types = manager.supported_filter_types();

        // Should return a non-empty list of filter types
        assert!(!types.is_empty());
        assert!(types.iter().any(|t| t.contains("Email")));
        assert!(types.iter().any(|t| t.contains("Chinese")));
    }
}
