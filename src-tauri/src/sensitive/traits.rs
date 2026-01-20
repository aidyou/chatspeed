use super::error::SensitiveError;

/// Helper function to adjust byte index to nearest character boundary.
/// This ensures that indices from regex matches don't fall inside multi-byte Unicode characters.
pub fn adjust_to_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx > text.len() {
        idx = text.len();
    }
    // Ensure we don't decrement past 0
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    // idx is now either 0 or a character boundary
    idx
}

/// Represents a candidate for a sensitive data match.
#[derive(Debug)]
pub struct FilterCandidate {
    pub start: usize,
    pub end: usize,
    pub filter_type: &'static str,
    /// Confidence score for the match, from 0.0 to 1.0.
    pub confidence: f32,
}

/// The core trait for all sensitive data filters.
pub trait SensitiveDataFilter {
    /// Identifier for the filter.
    fn filter_type(&self) -> &'static str;

    /// Returns a list of all candidate types this filter can produce.
    /// Default implementation returns just the [filter_type].
    fn produced_types(&self) -> Vec<&'static str> {
        vec![self.filter_type()]
    }

    /// List of supported languages. An empty vector means it's a common filter.
    fn supported_languages(&self) -> Vec<&'static str>;

    /// Executes the filter on the given text.
    ///
    /// # Arguments
    /// * `text` - The input text to scan.
    /// * `language` - The language of the text.
    ///
    /// # Returns
    /// A `Result` containing a vector of `FilterCandidate`s if successful,
    /// or a `SensitiveError` if an error occurs.
    fn filter(&self, text: &str, language: &str)
        -> std::result::Result<Vec<FilterCandidate>, SensitiveError>;

    /// Returns the priority of the filter.
    /// Lower values have higher priority.
    fn priority(&self) -> u32 {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjust_to_char_boundary_empty_string() {
        // Test with empty string
        let text = "";
        assert_eq!(adjust_to_char_boundary(text, 0), 0);
        assert_eq!(adjust_to_char_boundary(text, 5), 0); // Index beyond length
    }

    #[test]
    fn test_adjust_to_char_boundary_ascii() {
        // Test with ASCII text (1 byte per char)
        let text = "Hello World";
        assert_eq!(adjust_to_char_boundary(text, 0), 0);
        assert_eq!(adjust_to_char_boundary(text, 5), 5); // Between 'o' and ' '
        assert_eq!(adjust_to_char_boundary(text, 11), 11); // End of string
        assert_eq!(adjust_to_char_boundary(text, 20), 11); // Beyond end
    }

    #[test]
    fn test_adjust_to_char_boundary_unicode() {
        // Test with Unicode characters (multi-byte)
        let text = "Hello 世界 🌍";
        // "Hello " = 6 bytes, "世" = 3 bytes, "界" = 3 bytes, " 🌍" = 4 bytes (space + emoji)
        
        // Start of string
        assert_eq!(adjust_to_char_boundary(text, 0), 0);
        
        // Middle of ASCII part
        assert_eq!(adjust_to_char_boundary(text, 3), 3);
        
        // Boundary at start of Chinese character
        assert_eq!(adjust_to_char_boundary(text, 6), 6); // Before "世"
        
        // Middle of Chinese character (should adjust to start)
        assert_eq!(adjust_to_char_boundary(text, 7), 6); // Middle of "世"
        assert_eq!(adjust_to_char_boundary(text, 8), 6); // Middle of "世"
        
        // Boundary between Chinese characters
        assert_eq!(adjust_to_char_boundary(text, 9), 9); // Between "世" and "界"
        
        // Middle of emoji (4 bytes, starts at byte 13)
        assert_eq!(adjust_to_char_boundary(text, 13), 13); // Start of 🌍
        assert_eq!(adjust_to_char_boundary(text, 14), 13); // Middle of 🌍
        assert_eq!(adjust_to_char_boundary(text, 15), 13); // Middle of 🌍
        assert_eq!(adjust_to_char_boundary(text, 16), 13); // Middle of 🌍
        
        // End of string
        assert_eq!(adjust_to_char_boundary(text, 17), 17); // End
        assert_eq!(adjust_to_char_boundary(text, 20), 17); // Beyond end
    }

    #[test]
    fn test_adjust_to_char_boundary_mixed_unicode() {
        // Test with mixed Unicode characters
        let text = "a©b😀c";
        // a=1, ©=2, b=1, 😀=4, c=1
        
        assert_eq!(adjust_to_char_boundary(text, 0), 0); // a
        assert_eq!(adjust_to_char_boundary(text, 1), 1); // Between a and ©
        assert_eq!(adjust_to_char_boundary(text, 2), 1); // Middle of ©
        assert_eq!(adjust_to_char_boundary(text, 3), 3); // b
        assert_eq!(adjust_to_char_boundary(text, 4), 4); // Between b and 😀
        assert_eq!(adjust_to_char_boundary(text, 5), 4); // Middle of 😀
        assert_eq!(adjust_to_char_boundary(text, 6), 4); // Middle of 😀
        assert_eq!(adjust_to_char_boundary(text, 7), 4); // Middle of 😀
        assert_eq!(adjust_to_char_boundary(text, 8), 8); // c
        assert_eq!(adjust_to_char_boundary(text, 9), 9); // End
    }

    #[test]
    fn test_adjust_to_char_boundary_edge_cases() {
        // Test various edge cases
        let text = "test";
        
        // Index 0
        assert_eq!(adjust_to_char_boundary(text, 0), 0);
        
        // Index at string length
        assert_eq!(adjust_to_char_boundary(text, 4), 4);
        
        // Index beyond string length
        assert_eq!(adjust_to_char_boundary(text, 10), 4);
        
        // Very large index
        assert_eq!(adjust_to_char_boundary(text, usize::MAX), 4);
    }

    #[test]
    fn test_adjust_to_char_boundary_surrogate_pairs() {
        // Test with characters that might have surrogate pairs
        let text = "𐍈test"; // 𐍈 is 4 bytes in UTF-8
        
        assert_eq!(adjust_to_char_boundary(text, 0), 0);
        assert_eq!(adjust_to_char_boundary(text, 1), 0); // Middle of 𐍈
        assert_eq!(adjust_to_char_boundary(text, 2), 0); // Middle of 𐍈
        assert_eq!(adjust_to_char_boundary(text, 3), 0); // Middle of 𐍈
        assert_eq!(adjust_to_char_boundary(text, 4), 4); // After 𐍈
        assert_eq!(adjust_to_char_boundary(text, 5), 5); // t
    }

    #[test]
    fn test_filter_candidate_debug() {
        // Test FilterCandidate Debug implementation
        let candidate = FilterCandidate {
            start: 10,
            end: 20,
            filter_type: "TestFilter",
            confidence: 0.85,
        };
        
        // Just ensure it doesn't panic when formatting
        let debug_output = format!("{:?}", candidate);
        assert!(debug_output.contains("TestFilter"));
        assert!(debug_output.contains("0.85"));
    }

    #[test]
    fn test_trait_default_implementations() {
        // Test default implementations of SensitiveDataFilter trait
        
        struct TestFilter;
        
        impl SensitiveDataFilter for TestFilter {
            fn filter_type(&self) -> &'static str {
                "TestFilter"
            }
            
            fn supported_languages(&self) -> Vec<&'static str> {
                vec!["en"]
            }
            
            fn filter(&self, _text: &str, _language: &str) -> Result<Vec<FilterCandidate>, SensitiveError> {
                Ok(vec![])
            }
        }
        
        let filter = TestFilter;
        
        // Test default produced_types
        assert_eq!(filter.produced_types(), vec!["TestFilter"]);
        
        // Test default priority
        assert_eq!(filter.priority(), 100);
    }
}
