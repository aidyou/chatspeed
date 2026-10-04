use crate::sensitive::error::SensitiveError;
use crate::sensitive::traits::SensitiveDataFilter;
#[cfg(not(feature = "desktop"))]
use crate::sensitive::traits::FilterCandidate;
#[cfg(not(feature = "desktop"))]
use regex::Regex;

pub struct CompanyFilter {
    #[cfg(not(feature = "desktop"))]
    context_regex: Regex,
    #[cfg(not(feature = "desktop"))]
    suffix_regex: Regex,
}

impl CompanyFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        #[cfg(not(feature = "desktop"))]
        {
            let context_regex = Regex::new(r#"(?i)(?:Party\s+[A-D]|Contractor|Employer|Tenant|Landlord|Buyer|Seller|Lender|Borrower|Client|Consultant|Vendor|Supplier)[:：]\s*([A-Z][A-Za-z0-9\s&,.'-]{3,})"#)
                .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_company_context".to_string(), message: e.to_string() })?;
            let suffix_regex = Regex::new(r#"\b[A-Z][A-Za-z0-9\s&,.'-]{2,}(?:Inc\.|Ltd\.|Corp\.|LLC|L\.L\.C\.|PLC|P\.L\.C\.|GmbH|S\.A\.|B\.V\.)\b"#)
                .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_company_suffix".to_string(), message: e.to_string() })?;
            Ok(Self {
                context_regex,
                suffix_regex,
            })
        }
        #[cfg(feature = "desktop")]
        {
            Ok(Self {})
        }
    }
}

impl SensitiveDataFilter for CompanyFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishCompany"
    }
    #[cfg(not(feature = "desktop"))]
    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["en"]
    }
    #[cfg(not(feature = "desktop"))]
    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let mut candidates = Vec::new();
        for cap in self.context_regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence: 0.9,
                });
            }
        }
        for m in self.suffix_regex.find_iter(text) {
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
        40
    }
}
