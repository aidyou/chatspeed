use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};

/// A heuristic-based filter for detecting English names.
pub struct NameFilter;

// Regex to find potential names (one or more capitalized words).
static NAME_PATTERN_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\b[A-Z][a-z]+(?:\s[A-Z][a-z]+)*\b"#).unwrap());

// RegexSet to find strong contextual keywords or titles appearing before a name.
static NAME_CONTEXT_KEYWORDS: Lazy<RegexSet> = Lazy::new(|| {
    RegexSet::new(&[
        r"name is",
        r"I'm",
        r"I am",
        r"called",
        r"Mr\.",
        r"Mrs\.",
        r"Ms\.",
        r"Dr\.",
    ])
    .unwrap()
});

impl SensitiveDataFilter for NameFilter {
    fn filter_type(&self) -> &'static str {
        "EnglishName"
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

        for mat in NAME_PATTERN_REGEX.find_iter(text) {
            let start = mat.start();
            let end = mat.end();
            let name_candidate = mat.as_str();

            let mut confidence = 0.4; // Base confidence for any capitalized word sequence.

            // A single capitalized word is less likely to be a name unless it's at the start of a sentence.
            // But a sequence of two or more is more likely.
            if name_candidate.contains(' ') {
                confidence = 0.65; // Higher base for multi-word names like "John Smith".
            }

            // Check for context keywords within a reasonable distance before the match.
            let context_window_start = start.saturating_sub(15);
            let context_slice = &text[context_window_start..start];

            if NAME_CONTEXT_KEYWORDS.is_match(context_slice) {
                confidence = 0.95; // High confidence if context is found.
            }

            // Simple heuristic to down-weigh words at the beginning of a sentence.
            if start > 0 {
                // Check if the character before the word is sentence-ending punctuation.
                if let Some(prev_char) = text[..start].chars().last().and_then(|c| c.to_string().trim().chars().last()) {
                     if ".?!".contains(prev_char) {
                        confidence = 0.3; // Lower confidence if it's likely the start of a sentence.
                    }
                }
            } else if start == 0 {
                 confidence = 0.3; // Lower confidence if it's the very first word.
            }


            candidates.push(FilterCandidate {
                start,
                end,
                filter_type: self.filter_type(),
                confidence,
            });
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        50 // Same as ChineseName
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_with_strong_context_keyword() {
        let filter = NameFilter;
        let text = "As I said, my name is John Smith.";
        let result = filter.filter(text, "en").unwrap();
        let candidate = result.iter().find(|c| c.start == 21).unwrap();
        assert_eq!(candidate.confidence, 0.95);
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "John Smith");
    }
    
    #[test]
    fn test_name_with_title() {
        let filter = NameFilter;
        let text = "Please see Mr. Robert Paulson.";
        let result = filter.filter(text, "en").unwrap();
        let candidate = result.iter().find(|c| c.start == 12).unwrap();
        assert_eq!(candidate.confidence, 0.95);
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "Robert Paulson");
    }

    #[test]
    fn test_name_without_context() {
        let filter = NameFilter;
        let text = "We saw Jane Doe at the store.";
        let result = filter.filter(text, "en").unwrap();
        let candidate = result.iter().find(|c| c.start == 7).unwrap();
        // Base confidence for a two-word name
        assert_eq!(candidate.confidence, 0.65);
    }

    #[test]
    fn test_name_at_sentence_start_low_confidence() {
        let filter = NameFilter;
        let text = "The quick brown fox. Peter Piper picked a peck.";
        let result = filter.filter(text, "en").unwrap();
        // Find "Peter Piper"
        let candidate = result.iter().find(|c| c.start == 21).unwrap();
        assert_eq!(candidate.confidence, 0.3);
    }
    
    #[test]
    fn test_first_word_in_text_low_confidence() {
        let filter = NameFilter;
        let text = "Hello world, this is a test.";
        let result = filter.filter(text, "en").unwrap();
        let candidate = result.iter().find(|c| c.start == 0).unwrap();
        assert_eq!(candidate.confidence, 0.3);
    }
}
