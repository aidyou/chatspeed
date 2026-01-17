use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting Chinese Company names.
/// Uses context keywords (甲方, 乙方) and suffix keywords (有限公司, 集团).
pub struct CompanyFilter;

static COMPANY_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common contract roles and financial entities followed by a colon and the company name.
    // Includes: 甲方, 乙方, ..., 开户企业, 收款方, 受票方, 付款方.
    Regex::new(r#"(?:甲方|乙方|丙方|丁方|委托方|受托方|承揽方|定作方|发包方|承包方|出租方|承租方|转让方|受让方|出卖方|买受方|贷款方|借款方|保证人|抵押人|质押人|开户企业|收款方|受票方|付款方)[:：]\s*([\u4e00-\u9fa5a-zA-Z0-9()（）]{4,})"#).unwrap()
});

static COMPANY_SUFFIX_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches names ending in typical company suffixes
    Regex::new(r#"[\u4e00-\u9fa5()（）]{2,20}(?:有限公司|责任公司|集团|总厂|分公司)"#).unwrap()
});

impl SensitiveDataFilter for CompanyFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseCompany"
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

        // 1. Context-based matching (Very High confidence)
        for cap in COMPANY_CONTEXT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                // Heuristic: Must contain at least some Chinese or "Company" related keywords if it's purely English?
                // For now, accept what follows the colon.
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.95,
                });
            }
        }

        // 2. Suffix-based matching (High confidence)
        for m in COMPANY_SUFFIX_REGEX.find_iter(text) {
             candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.9,
            });
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        40 // Medium-High priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_company_context() {
        let filter = CompanyFilter;
        let text = "甲方：北京xxxx在线信息咨询服务有限公司\n地址...";
        let result = filter.filter(text, "zh").unwrap();
        
        let match_text = &text[result[0].start..result[0].end];
        assert_eq!(match_text, "北京xxxx在线信息咨询服务有限公司");
    }

    #[test]
    fn test_filter_company_suffix() {
        let filter = CompanyFilter;
        let text = "我们是杭州xxxx网络科技有限公司的员工。";
        let result = filter.filter(text, "zh").unwrap();
        
        let match_text = &text[result[0].start..result[0].end];
        assert_eq!(match_text, "杭州xxxx网络科技有限公司");
    }
    
    #[test]
    fn test_filter_company_context_brackets() {
        let filter = CompanyFilter;
        let text = "乙方：杭州xxxx网络科技有限公司（承揽方）";
        let result = filter.filter(text, "zh").unwrap();
        
        // Matches until space or newline. "（承揽方）" is included in regex [\u4e00-\u9fa5a-zA-Z0-9()（）]
        let match_text = &text[result[0].start..result[0].end];
        assert!(match_text.contains("杭州xxxx网络科技有限公司"));
    }
}
