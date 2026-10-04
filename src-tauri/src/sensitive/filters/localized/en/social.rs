use crate::sensitive::error::SensitiveError;
use crate::sensitive::traits::SensitiveDataFilter;
#[cfg(not(feature = "desktop"))]
use crate::sensitive::traits::FilterCandidate;
#[cfg(not(feature = "desktop"))]
use regex::Regex;

pub struct SocialFilter {
    #[cfg(not(feature = "desktop"))]
    regex: Regex,
}

impl SocialFilter {
    pub fn new() -> Result<Self, SensitiveError> {
        #[cfg(not(feature = "desktop"))]
        {
            let regex = Regex::new(r#"(?i)(?:WeChat|WhatsApp|Telegram|Skype|Twitter|Instagram|ID)[:：]?\s*([a-zA-Z0-9_-]{4,32})"#)
                .map_err(|e| SensitiveError::RegexCompilationFailed { pattern: "en_social_regex".to_string(), message: e.to_string() })?;
            Ok(Self { regex })
        }
        #[cfg(feature = "desktop")]
        {
            Ok(Self {})
        }
    }
}

impl SensitiveDataFilter for SocialFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishSocial"
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
            .captures_iter(text)
            .filter_map(|cap| cap.get(1))
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 0.85,
            })
            .collect();
        Ok(candidates)
    }
    fn priority(&self) -> u32 {
        15
    }
}
