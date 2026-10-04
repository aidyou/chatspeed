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
            let context_regex = Regex::new(r#"(?:甲方|乙方|丙方|丁方|委托方|受托方|承揽方|定作方|发包方|承包方|出租方|承租方|转让方|受让方|出卖方|买受方|贷款方|借款方|保证人|抵押人|质押人|开户企业|收款方|受票方|付款方)[:：]\s*([\p{Han}a-zA-Z0-9()（）]{4,})"#).map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "zh_company_context".to_string(), message: e.to_string() })?;
            let suffix_regex = Regex::new(
                r#"[\p{Han}()（）]{2,20}(?:有限公司|责任公司|集团|总厂|分公司)"#,
            )
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "zh_company_suffix".to_string(),
                message: e.to_string(),
            })?;
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
        "ChineseCompany"
    }
    #[cfg(not(feature = "desktop"))]
    fn supported_languages(&self) -> Vec<&'static str> {
        vec!["zh", "zh-Hans", "zh-Hant"]
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
                    confidence: 0.95,
                });
            }
        }
        for m in self.suffix_regex.find_iter(text) {
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
        40
    }
}
