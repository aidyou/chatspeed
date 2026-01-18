use super::{
    filters::{
        common::{
            credit_card::CreditCardFilter, email::EmailFilter,
            international_credit_card::InternationalCreditCardFilter, ip_address::IpAddressFilter,
        },
        localized::{
            en::{
                address::AddressFilter as EnAddressFilter, company::CompanyFilter as EnCompanyFilter,
                financial::FinancialFilter as EnFinancialFilter, mobile::MobileFilter as EnMobileFilter,
                name::NameFilter as EnNameFilter, project::ProjectFilter as EnProjectFilter,
                social::SocialFilter as EnSocialFilter, ssn::SsnFilter,
            },
            zh::{
                address::AddressFilter as ZhAddressFilter, company::CompanyFilter as ZhCompanyFilter,
                financial::FinancialFilter as ZhFinancialFilter, id_card::IdCardFilter,
                landline::LandlineFilter, mobile::MobileFilter as ZhMobileFilter,
                name::NameFilter as ZhNameFilter, project::ProjectFilter as ZhProjectFilter,
                social::SocialFilter as ZhSocialFilter, unionpay::UnionPayFilter,
            },
        },
    },
    i18n_support::ReplacementTextCache,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use log::error;
use crate::error::Result;

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
    /// Performs pre-compilation of all regexes and returns an error if initialization fails.
    pub fn new() -> Result<Self> {
        let mut filters: Vec<Box<dyn SensitiveDataFilter + Send + Sync>> = Vec::new();

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

        // Sort filters by priority (lower number = higher priority)
        filters.sort_by_key(|f| f.priority());

        Ok(Self { filters })
    }

    /// This is the main entry point for filtering text.
    pub fn filter_text(&self, text: &str, languages: &[&str], config: &SensitiveConfig) -> String {
        if !config.enabled {
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
                    ranges.push(m.0..m.0 + allow_word.len());
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

            if supported_langs.is_empty() || supported_langs.iter().any(|sl| languages.contains(sl)) {
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
                let c_range = m.0..m.0 + block_word.len();
                // Custom blocks also respect allowlist
                let is_allowed = if !allow_ranges.is_empty() {
                    let idx = allow_ranges.partition_point(|r| r.start < c_range.end);
                    idx != 0 && ranges_overlap(c_range.clone(), allow_ranges[idx - 1].clone())
                } else {
                    false
                };

                if !is_allowed {
                    all_candidates.push(FilterCandidate {
                        start: m.0,
                        end: m.0 + block_word.len(),
                        filter_type: "CustomBlock",
                        confidence: 1.0,
                    });
                }
            }
        }

        let final_candidates = Self::resolve_conflicts(all_candidates);

        let mut sanitized_text = text.to_string();
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
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
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
