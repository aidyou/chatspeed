use crate::sensitive::{
    error::SensitiveError,
    traits::{FilterCandidate, SensitiveDataFilter},
};
use regex::{Regex, RegexSet};

/// A filter for detecting Chinese names based on common surnames and context.
pub struct NameFilter {
    name_pattern: Regex,
    context_keywords: RegexSet,
}

impl NameFilter {
    /// Creates a new `NameFilter` and pre-compiles its regexes.
    pub fn new() -> Result<Self, SensitiveError> {
        let surnames = vec![
            "王", "李", "张", "刘", "陈", "杨", "赵", "黄", "周", "吴", "徐", "孙", "胡", "朱",
            "高", "林", "何", "郭", "马", "罗", "梁", "宋", "郑", "谢", "韩", "唐", "冯", "于",
            "董", "萧", "程", "曹", "袁", "邓", "许", "傅", "沈", "曾", "彭", "吕", "苏", "卢",
            "蒋", "蔡", "贾", "丁", "魏", "薛", "叶", "阎", "余", "潘", "杜", "昭", "戴", "夏",
            "钟", "汪", "田", "任", "姜", "范", "方", "石", "姚", "谭", "廖", "邹", "熊", "金",
            "陆", "郝", "孔", "白", "崔", "康", "毛", "邱", "秦", "江", "史", "顾", "侯", "邵",
            "孟", "龙", "万", "段", "漕", "钱", "汤", "尹", "黎", "易", "常", "武", "乔", "贺",
            "赖", "龚", "文", "邢", "甘", "温", "詹", "庄", "骆", "陶", "包", "庞", "聂", "焦",
            "向", "管", "齐", "梅", "盛", "童", "霍", "柯", "阮", "纪", "舒", "屈", "项", "祝",
            "闵", "季", "麻", "强", "路", "娄", "危", "郗", "骈",
        ];

        // Use \p{Han} for robust Chinese character matching
        let pattern = format!(r#"\b(?:{})\p{{Han}}{{1,2}}\b"#, surnames.join("|"));
        let name_pattern =
            Regex::new(&pattern).map_err(|e| SensitiveError::RegexCompilationFailed {
                pattern: "zh_name_pattern".to_string(),
                message: e.to_string(),
            })?;

        let context_keywords = RegexSet::new(&[
            r"我叫",
            r"我是",
            r"姓名[:：]",
            r"联系人[:：]",
            r"负责人[:：]",
            r"代表人[:：]",
        ])
        .map_err(|e| SensitiveError::RegexCompilationFailed {
            pattern: "zh_name_context".to_string(),
            message: e.to_string(),
        })?;

        Ok(Self {
            name_pattern,
            context_keywords,
        })
    }
}

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

        for m in self.name_pattern.find_iter(text) {
            let mut confidence = 0.5; // Base confidence for just matching "Surname + 1-2 chars"

            // Check for surrounding context
            let start = m.start();
            let context_start = start.saturating_sub(15);
            let context_window = &text[context_start..start];

            if self.context_keywords.is_match(context_window) {
                confidence += 0.45; // High confidence with context
            }

            if confidence >= 0.8 {
                candidates.push(FilterCandidate {
                    start: m.start(),
                    end: m.end(),
                    filter_type: self.filter_type(),
                    confidence,
                });
            }
        }

        Ok(candidates)
    }

    fn priority(&self) -> u32 {
        60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_name_with_context() {
        let filter = NameFilter::new().unwrap();
        let text = "你好，我叫张伟。";
        let result = filter.filter(text, "zh").unwrap();
        assert_eq!(result.len(), 1);
    }
}
