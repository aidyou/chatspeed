use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};

/// A heuristic-based filter for detecting Chinese names.
pub struct NameFilter;

// A small list of the most common Chinese surnames.
// In a real-world application, this list would be much larger.
static COMMON_SURNAMES: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "王", "李", "张", "刘", "陈", "杨", "黄", "赵", "吴", "周", "徐", "孙", "马", "朱", "胡",
    ]
});

// Regex to find potential names (a surname followed by 1 or 2 characters).
// This is a simplified approach.
static NAME_PATTERN_REGEX: Lazy<Regex> = Lazy::new(|| {
    let surnames_pattern = COMMON_SURNAMES.join("|");
    // Match a surname followed by 1 or 2 Chinese characters.
    // \p{Han} is a Unicode property for Chinese characters.
    let pattern = format!(r"({})(?:\p{{Han}}{{1,2}})", surnames_pattern);
    Regex::new(&pattern).unwrap()
});

// RegexSet to find strong contextual keywords appearing before a name.
static NAME_CONTEXT_KEYWORDS: Lazy<RegexSet> = Lazy::new(|| {
    RegexSet::new(&[
        r"我叫",
        r"名叫",
        r"姓名是",
        r"姓名：",
        r"Name:",
    ])
    .unwrap()
});

impl SensitiveDataFilter for NameFilter {
    fn filter_type(&self) -> &'static str {
        "ChineseName"
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

        for mat in NAME_PATTERN_REGEX.find_iter(text) {
            let start = mat.start();
            let end = mat.end();
            let mut confidence = 0.6; // Base confidence for a pattern match.

            // Check for context keywords within a reasonable distance before the match.
            // Use char indices to avoid splitting multi-byte characters
            let char_indices: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
            let char_pos = char_indices.binary_search(&start).unwrap_or(0);
            let context_char_start = if char_pos >= 5 { char_pos - 5 } else { 0 };
            let context_byte_start = char_indices[context_char_start];
            
            let context_slice = &text[context_byte_start..start];

            if NAME_CONTEXT_KEYWORDS.is_match(context_slice) {
                confidence = 0.95; // High confidence if context is found.
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
        50 // Lower priority than regex-certain filters.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_with_strong_context() {
        let filter = NameFilter;
        let text = "你好，我叫张伟，请多指教。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(candidate.start, 15); // Byte position of "张"
        assert_eq!(candidate.end, 21);   // Byte position after "伟"
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "张伟");
        assert_eq!(candidate.confidence, 0.95);
    }

    #[test]
    fn test_name_without_context() {
        let filter = NameFilter;
        let text = "在会议上，李娜发表了讲话。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
        let candidate = &result[0];
        assert_eq!(candidate.start, 5);
        assert_eq!(candidate.end, 8);
        assert_eq!(text.get(candidate.start..candidate.end).unwrap(), "李娜");
        assert_eq!(candidate.confidence, 0.6);
    }

    #[test]
    fn test_multiple_names() {
        let filter = NameFilter;
        let text = "项目负责人是王伟，助理是陈静。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].confidence, 0.6);
        assert_eq!(result[1].confidence, 0.6);
    }

    #[test]
    fn test_no_names() {
        let filter = NameFilter;
        let text = "这是一个普通的句子。";
        let result = filter.filter(text, "zh").unwrap();
        assert!(result.is_empty());
    }
}
