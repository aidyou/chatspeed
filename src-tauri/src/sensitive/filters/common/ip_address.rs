use crate::sensitive::{
    error::SensitiveError,
    traits::{adjust_to_char_boundary, FilterCandidate, SensitiveDataFilter},
};
use regex::Regex;

/// A filter for detecting IPv4 and IPv6 addresses.
pub struct IpAddressFilter {
    regex: Regex,
}

impl IpAddressFilter {
    /// Creates a new `IpAddressFilter` and pre-compiles its regex.
    pub fn new() -> Result<Self, SensitiveError> {
        let ipv4_pattern = r#"(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)"#;
        let ipv6_pattern = r#"(?:[A-Fa-f0-9]{1,4}:){7}[A-Fa-f0-9]{1,4}"#;
        let regex = Regex::new(&format!(r#"\b(?:{}|{})\b"#, ipv4_pattern, ipv6_pattern))
            .map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "ip_address_regex".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self { regex })
    }
}

impl SensitiveDataFilter for IpAddressFilter {
    fn filter_type(&self) -> &'static str {
        "IPAddress"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new() // Language-agnostic
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = self
            .regex
            .find_iter(text)
            .map(|m| {
                let start = adjust_to_char_boundary(text, m.start());
                let end = adjust_to_char_boundary(text, m.end());
                FilterCandidate {
                    start,
                    end,
                    filter_type: self.filter_type(),
                    confidence: 1.0,
                }
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_ipv4() {
        let filter = IpAddressFilter::new().unwrap();
        let text = "My local IP is 192.168.1.1.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].filter_type, "IPAddress");
    }

    #[test]
    fn test_filter_ipv6() {
        let filter = IpAddressFilter::new().unwrap();
        let text = "IPv6: 2001:0db8:85a3:0000:0000:8a2e:0370:7334 is sensitive.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
    }
}