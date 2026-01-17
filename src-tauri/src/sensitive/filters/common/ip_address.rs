use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::Regex;

/// A filter for detecting IPv4 and IPv6 addresses.
pub struct IpAddressFilter;

// This regex combines patterns for both IPv4 and common IPv6 addresses.
// It's not exhaustive for all IPv6 variations (like compressed zeros `::`) but covers the most standard forms.
static IP_ADDRESS_REGEX: Lazy<Regex> = Lazy::new(|| {
    let ipv4_pattern = r#"\b((25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b"#;
    // A simplified but effective pattern for common IPv6 representations.
    let ipv6_pattern = r#"\b([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b"#;
    Regex::new(&format!("{}|{}", ipv4_pattern, ipv6_pattern)).unwrap()
});

impl SensitiveDataFilter for IpAddressFilter {
    fn filter_type(&self) -> &'static str {
        "IPAddress"
    }

    fn supported_languages(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn filter(
        &self,
        text: &str,
        _language: &str,
    ) -> std::result::Result<Vec<FilterCandidate>, SensitiveError> {
        let candidates = IP_ADDRESS_REGEX
            .find_iter(text)
            .map(|m| FilterCandidate {
                start: m.start(),
                end: m.end(),
                filter_type: self.filter_type(),
                confidence: 1.0,
            })
            .collect();
        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        // High priority, similar to email.
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_ipv4_address() {
        let filter = IpAddressFilter;
        let text = "The server is located at 192.168.1.1.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 25);
        assert_eq!(result[0].end, 36);
        assert_eq!(result[0].filter_type, "IPAddress");
    }

    #[test]
    fn test_filter_ipv6_address() {
        let filter = IpAddressFilter;
        let text = "Connect to 2001:0db8:85a3:0000:0000:8a2e:0370:7334 for service.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 11);
        assert_eq!(result[0].end, 50);
    }
    
    #[test]
    fn test_filter_multiple_ip_addresses() {
        let filter = IpAddressFilter;
        let text = "Allowed IPs: 8.8.8.8 and 1.1.1.1.";
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start, 13);
        assert_eq!(result[0].end, 20);
        assert_eq!(result[1].start, 25);
        assert_eq!(result[1].end, 32);
    }

    #[test]
    fn test_no_ip_addresses() {
        let filter = IpAddressFilter;
        let text = "This is a regular sentence without any IP addresses.";
        let result = filter.filter(text, "en").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_false_positives_version_numbers() {
        let filter = IpAddressFilter;
        let text = "This is version 1.2.3.4 of the software.";
        // Our regex is simple and might match this. A more complex regex could use negative lookarounds.
        // For this implementation, we accept this as a known limitation to keep the regex simple.
        let result = filter.filter(text, "en").unwrap();
        assert_eq!(result.len(), 1); // Acknowledging the match
    }
}
