use crate::sensitive::error::SensitiveError;
use crate::sensitive::traits::SensitiveDataFilter;
#[cfg(not(feature = "desktop"))]
use crate::sensitive::traits::FilterCandidate;
#[cfg(not(feature = "desktop"))]
use regex::Regex;

pub struct MobileFilter {
    #[cfg(not(feature = "desktop"))]
    regex: Regex,
}

impl MobileFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        #[cfg(not(feature = "desktop"))]
        {
            let regex = Regex::new(r#"\b(?:\+?1[-\s.]?)?\(?\d{3}\)?[-\s.]?\d{3}[-\s.]?\d{4}\b"#)
                .map_err(|e| SensitiveError::RegexCompilationFailed {
                    pattern: "en_mobile_regex".to_string(),
                    message: e.to_string(),
                })?;
            Ok(Self { regex })
        }
        #[cfg(feature = "desktop")]
        {
            Ok(Self {})
        }
    }
}

impl SensitiveDataFilter for MobileFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishMobile"
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
        let candidates = self
            .regex
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.9,
            })
            .collect();
        Ok(candidates)
    }
    fn priority(&self) -> u32 {
        2
    }
}
