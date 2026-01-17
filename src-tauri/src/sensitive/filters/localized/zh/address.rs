use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese addresses.
/// Uses context keywords (地址, 住址) and suffix keywords (省, 市, 区, 路, 号, 大厦).
pub struct AddressFilter;

static ADDRESS_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common address labels in contracts.
    // Includes: 地址, 住址, 所在地, 住所地, 注册地址, 通讯地址.
    Regex::new(r#"(?:地址|住址|所在地|住所地|注册地址|通讯地址)[:：]\s*([^\n\s]+(?:省|市|区|县|街|路|号|大厦|层|室|单元|栋)[^\n\s]*)"#).unwrap()
});

static ADDRESS_PATTERN_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common address patterns without explicit context "地址:"
    // e.g. "北京市朝阳区..."
    // Heuristic: Must start with a location name and contain typical address suffixes
    Regex::new(r#"(?:[\u4e00-\u9fa5]{2,}(?:省|市|区|县))[\u4e00-\u9fa50-9-]{2,}(?:路|街|道|号|院|大厦|广场|中心)"#).unwrap()
});

impl SensitiveDataFilter for AddressFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseAddress"
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

        // 1. Context-based matching (High confidence)
        for cap in ADDRESS_CONTEXT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.95,
                });
            }
        }

        // 2. Pattern-based matching (Medium confidence)
        // Only run if we haven't found overlaps (handled by Manager mainly, but we can do simple check or just let Manager handle)
        for m in ADDRESS_PATTERN_REGEX.find_iter(text) {
             candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.8,
            });
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        50 // Medium priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_address_context() {
        let filter = AddressFilter;
        let text = "地址：北京市朝外大街22号泛利大厦10层\n联系人...";
        let result = filter.filter(text, "zh").unwrap();
        
        // Should find at least one candidate
        assert!(!result.is_empty());
        let match_text = &text[result[0].start..result[0].end];
        assert_eq!(match_text, "北京市朝外大街22号泛利大厦10层");
    }

    #[test]
    fn test_filter_address_pattern() {
        let filter = AddressFilter;
        let text = "公司位于杭州江干区钱江路1366号华润大厦B2502。";
        let result = filter.filter(text, "zh").unwrap();
        
        let match_text = &text[result[0].start..result[0].end];
        // Regex might match "杭州江干区钱江路1366号华润大厦"
        assert!(match_text.contains("杭州江干区"));
        assert!(match_text.contains("华润大厦"));
    }
}
