use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting English Company names.
/// Uses context keywords (Party A, Contractor) and suffix keywords (Inc., Ltd.).
pub struct CompanyFilter;

static COMPANY_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches common English contract roles followed by a colon or "is"
    // Roles: Party A/B/C/D, Contractor, Employer, Tenant, Landlord, Buyer, Seller, 
    //        Lender, Borrower, Client, Consultant, Vendor, Supplier.
    Regex::new(r#"(?i)(?:Party\s+[A-D]|Contractor|Employer|Tenant|Landlord|Buyer|Seller|Lender|Borrower|Client|Consultant|Vendor|Supplier)[:：]\s*([A-Z][A-Za-z0-9\s&,.'-]{3,})"#).unwrap()
});

static COMPANY_SUFFIX_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches names ending in typical English company suffixes
    Regex::new(r#"\b[A-Z][A-Za-z0-9\s&,.'-]{2,}(?:Inc\.|Ltd\.|Corp\.|LLC|L\.L\.C\.|PLC|P\.L\.C\.|GmbH|S\.A\.|B\.V\.)\b"#).unwrap()
});

impl SensitiveDataFilter for CompanyFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishCompany"
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

        // 1. Context-based matching (High confidence)
        for cap in COMPANY_CONTEXT_REGEX.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }

        // 2. Suffix-based matching (Medium-High confidence)
        for m in COMPANY_SUFFIX_REGEX.find_iter(text) {
             candidates.push(FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.85,
            });
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        40 // Consistent with Chinese
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_company_context() {
        let filter = CompanyFilter;
        let text = "Party A: Global Tech Solutions Inc.";
        let result = filter.filter(text, "en").unwrap();
        assert!(!result.is_empty());
        let match_text = &text[result[0].start..result[0].end];
        assert_eq!(match_text, "Global Tech Solutions Inc.");
    }

    #[test]
    fn test_filter_company_suffix() {
        let filter = CompanyFilter;
        let text = "We are working with Apple Inc. today.";
        let result = filter.filter(text, "en").unwrap();
        assert!(!result.is_empty());
        assert_eq!(&text[result[0].start..result[0].end], "Apple Inc.");
    }
}
