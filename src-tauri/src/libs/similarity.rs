use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use humantime::parse_duration;
use jieba_rs::Jieba;
use phf::{phf_map, phf_set};
use tinysegmenter::tokenize as ja_tokenize;

lazy_static::lazy_static! {
    static ref JIEBA: Jieba = Jieba::new();
}
use whatlang::{Lang, Script};

use crate::http::crawler::SearchResult;

static STOP_WORDS: phf::Map<&'static str, phf::Set<&'static str>> = phf_map! {
    "zh-Hans" => phf_set! {
        "的", "和", "是", "在", "了", "有", "就", "这", "为", "与", "也", "而", "或", "要", "被",
        "我", "你", "他", "她", "它", "们", "个", "之", "于", "等", "以", "但", "对", "从", "如",
        "因", "所", "能", "那", "到", "得", "着", "给", "让", "地", "中", "上", "下", "前", "后",
        "来", "去", "过", "吗", "呢", "啊", "把", "很", "再", "都", "却", "只", "该", "谁", "什么"
    },
    "zh-Hant" => phf_set! {
        "的", "和", "是", "在", "了", "有", "就", "這", "為", "與", "也", "而", "或", "要", "被",
        "我", "你", "他", "她", "它", "們", "個", "之", "於", "等", "以", "但", "對", "從", "如",
        "因", "所", "能", "那", "到", "得", "著", "給", "讓", "地", "中", "上", "下", "前", "後",
        "來", "去", "過", "嗎", "呢", "啊", "把", "很", "再", "都", "卻", "只", "該", "誰", "什麼"
    },
    "en" => phf_set! {
        "the", "a", "an", "of", "and", "in", "to", "for", "that", "this", "is", "on", "with", "by", "at",
        "i", "you", "he", "she", "it", "we", "they", "my", "your", "his", "her", "its", "our", "their",
        "me", "him", "us", "them", "what", "who", "which", "when", "where", "why", "how", "all", "any",
        "both", "each", "few", "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own",
        "same", "so", "than", "too", "very", "as", "be", "been", "being", "have", "has", "had", "do", "does",
        "did", "but", "if", "or", "because", "from", "up", "down", "about", "into", "over", "after", "before"
    },
    "de" => phf_set! {
        "der", "die", "das", "und", "in", "zu", "den", "für", "mit", "sich", "des", "im", "nicht", "von", "ist", "ein", "eine", "einer", "einem", "einen", "eines", "auf", "als", "auch", "es", "an", "nach", "wie", "bei", "war", "wird", "sind", "sein", "keine", "hat", "nur", "um", "am", "dass", "noch", "so", "wenn", "hier", "bis", "da", "aber", "aus", "was", "wir", "ihr", "sie", "er", "ich", "kann", "mich", "mir", "mein", "dich", "dir", "dein", "wer", "warum", "wo", "wann", "alle", "oder", "vor", "durch"
    },
    "fr" => phf_set! {
        "le", "la", "les", "un", "une", "des", "et", "à", "dans", "pour", "en", "sur", "pas", "par", "vous",
        "je", "tu", "il", "elle", "nous", "ils", "elles", "mon", "ton", "son", "ma", "ta", "sa", "mes", "tes", "ses", "notre", "votre", "leur", "nos", "vos", "leurs", "ce", "cette", "ces", "que", "qui", "quoi", "dont", "où", "quand", "comment", "pourquoi", "avec", "sans", "mais", "ou", "donc", "car", "si", "au", "aux", "du", "de", "est", "sont", "être", "avoir", "fait", "faire", "dit", "dire", "plus", "moins", "très"
    },
    "ja" => phf_set! {
        "の", "に", "は", "を", "が", "と", "で", "も", "へ", "や", "です", "だ", "ます", "ない",
        "から", "まで", "より", "など", "して", "する", "れる", "られる", "なる", "ある", "いる", "この",
        "その", "あの", "どの", "これ", "それ", "あれ", "どれ", "わたし", "あなた", "かれ", "かのじょ",
        "わたしたち", "あなたたち", "かれら", "かのじょたち", "として", "について", "による", "において",
        "ので", "のに", "ところ", "もの", "こと", "ため", "さん", "くん", "ちゃん", "よう", "そう", "どう"
    },
    "ko" => phf_set! {
        "은", "는", "이", "가", "을", "를", "과", "와", "의", "에", "에서", "도", "로", "하다", "고",
        "나", "너", "그", "그녀", "우리", "그들", "저", "제", "당신", "저희", "너희", "그것", "이것", "저것",
        "그런", "이런", "저런", "그리고", "하지만", "또는", "또한", "그래서", "왜냐하면", "만약", "때문에",
        "에게", "부터", "까지", "처럼", "같이", "이나", "거나", "든지", "하고", "이고", "거의", "매우", "너무",
        "아주", "정말", "진짜", "모든", "어떤", "무슨", "언제", "어디", "어떻게", "왜", "누구", "없다", "있다"
    },
    "es" => phf_set! {
        "de", "la", "que", "el", "en", "y", "a", "los", "se", "del", "las", "un", "por", "con", "no",
        "una", "su", "para", "es", "al", "lo", "como", "más", "pero", "sus", "le", "ya", "o", "este",
        "sí", "porque", "esta", "entre", "cuando", "muy", "sin", "sobre", "también", "me", "hasta", "hay",
        "donde", "quien", "desde", "todo", "nos", "durante", "todos", "uno", "les", "ni", "contra", "otros",
        "ese", "eso", "ante", "ellos", "e", "esto", "mí", "antes", "algunos", "qué", "unos", "yo", "otro",
        "otras", "otra", "él", "tanto", "esa", "estos", "mucho", "quienes", "nada", "muchos", "cual", "poco"
    },
    "pt" => phf_set! {
        "o", "a", "de", "do", "da", "em", "um", "para", "com", "não", "uma", "os", "as", "pelo", "se",
        "na", "por", "mais", "das", "dos", "como", "mas", "foi", "ao", "ele", "ela", "são", "eu", "também",
        "só", "pela", "até", "isso",  "entre", "era", "depois", "sem", "mesmo", "aos", "ter",
        "seus", "quem", "nas", "me", "esse", "eles", "estão", "você", "tinha", "foram", "essa", "num", "nem",
        "suas", "meu", "às", "minha", "têm", "numa", "pelos", "elas", "havia", "seja", "qual", "será", "nós",
        "tenho", "lhe", "deles", "essas", "esses", "pelas", "este", "fosse", "dele", "tu", "te", "vocês", "vos"
    },
    "ru" => phf_set! {
        "и", "в", "на", "с", "по", "к", "но", "а", "о", "от", "для", "не", "что", "это", "так",
        "я", "ты", "он", "она", "оно", "мы", "вы", "они", "мой", "твой", "его", "её", "наш", "ваш", "их",
        "меня", "тебя", "нас", "вас", "мне", "тебе", "ему", "ей", "нам", "вам", "им",
        "как", "где", "когда", "почему", "зачем", "который", "какой", "чей", "кто", "тот", "этот",
        "весь", "всё", "все", "сам", "самый", "каждый", "любой", "другой", "такой", "только", "уже", "ещё",
        "был", "была", "были", "быть", "есть", "будет", "может", "из", "под", "над", "при", "про", "без", "до"
    },
};

/// Computes the relevance score between a search result and a query.
///
/// # Arguments
/// * `result` - The search result containing title and content.
/// * `query` - The search query string.
///
/// # Returns
/// A relevance score between 0.0 and 1.0, where 1.0 indicates a perfect match.
pub fn compute_relevance(result: &mut SearchResult, query: &str) -> f32 {
    // 计算标题相关性
    let title_relevance = compute_title_relevance(&result.title, query);

    // 计算内容相关性
    let content_relevance =
        compute_content_relevance(&result.summary.clone().unwrap_or_default(), query);

    // 计算基础分数
    let base_score =
        0.55 * title_relevance + 0.4 * content_relevance + 0.05 * url_quality_score(&result.url);

    // 时间衰减调整
    result.score = if let Some(pd) = &result.publish_date {
        if let Some(pt) = strtotime(pd) {
            let time_decay = time_score(pt);
            base_score * 0.9 + time_decay * 0.1
        } else {
            base_score
        }
    } else {
        base_score
    };

    result.score.clamp(0.2, 1.0)
}

fn url_quality_score(url: &str) -> f32 {
    let url = url.to_lowercase();

    // 协议评分
    let https_bonus = url.starts_with("https://") as u8 as f32 * 0.1;

    // 路径深度评分（1-5级路径）
    let path_depth = url.split('/').filter(|s| !s.is_empty()).count().min(5) as f32 * 0.05;

    // 顶级域名奖励
    let tld_bonus = if [".com", ".org", ".gov", ".edu"]
        .iter()
        .any(|tld| url.contains(tld))
    {
        0.05
    } else {
        0.0
    };

    (https_bonus + path_depth + tld_bonus).min(0.2)
}

/// Computes the relevance score between a title and a query.
///
/// # Arguments
/// * `title` - The title of the search result.
/// * `query` - The search query string.
///
/// # Returns
/// A relevance score between 0.0 and 1.0, where 1.0 indicates a perfect match.
pub fn compute_title_relevance(title: &str, query: &str) -> f32 {
    let title_tokens = tokenize(&title.to_lowercase()).ordered;
    let query_tokens = tokenize(&query.to_lowercase()).ordered;

    if query_tokens.is_empty() || title_tokens.is_empty() {
        return 0.0;
    }

    let query_set: HashSet<_> = query_tokens.iter().collect();
    let mut intersection = 0.0;
    let mut title_weight = 0.0;

    // 改进的位置权重：前5词保持高权重，后续缓慢衰减
    let total_words = title_tokens.len();
    for (idx, word) in title_tokens.iter().enumerate() {
        let position_weight = if idx < 5 {
            1.0 - (idx as f32 * 0.1)
        } else {
            0.6 - 0.4 * ((idx - 5) as f32 / (total_words - 5) as f32)
        }
        .max(0.2);

        title_weight += position_weight;
        if query_set.contains(word) {
            intersection += position_weight;
        }
    }

    // 改进的Jaccard公式
    let base_score =
        (intersection / (title_weight + query_tokens.len() as f32 + 1e-5)).clamp(0.0, 0.8); // 设置上限避免满分

    // 连续匹配奖励（最大0.2）
    let mut consecutive_bonus = 0.0;
    let mut current_streak = 0;
    for word in &title_tokens {
        current_streak = if query_set.contains(word) {
            current_streak + 1
        } else {
            0
        };
        consecutive_bonus += (current_streak as f32).sqrt() * 0.05;
    }

    (base_score + consecutive_bonus.min(0.2)).min(0.8)
}

/// Computes the relevance score between content and a query.
///
/// # Arguments
/// * `content` - The content of the search result.
/// * `query` - The search query string.
///
/// # Returns
/// A relevance score between 0.0 and 1.0, where 1.0 indicates a perfect match.
/// 计算查询词匹配分数
/// 基于查询词在内容中的出现比例计算
///
/// # Arguments
/// * `content` - 要搜索的内容
/// * `query` - 搜索查询词
///
/// # Returns
/// 分数在 0.0 到 1.0 之间，1.0 表示所有查询词都找到了
pub fn compute_query_match_score(content: &str, query: &str) -> f32 {
    if content.is_empty() || query.is_empty() {
        return 0.0;
    }

    let content_tokens = tokenize(content);
    let query_tokens = tokenize(query);

    if query_tokens.unique.is_empty() {
        return 0.0;
    }

    let matched_terms = query_tokens
        .unique
        .iter()
        .filter(|token| content_tokens.unique.contains(*token))
        .count() as f32;

    matched_terms / query_tokens.unique.len() as f32
}

/// 计算内容相关度分数
/// 考虑多个因素，包括词频、位置等
///
/// # Arguments
/// * `content` - 搜索结果的内容
/// * `query` - 搜索查询词
///
/// # Returns
/// 相关度分数在 0.0 到 1.0 之间，1.0 表示完全匹配
pub fn compute_content_relevance(content: &str, query: &str) -> f32 {
    // 边界条件处理
    if content.is_empty() || query.is_empty() {
        return 0.0;
    }

    let content_lc = content.to_lowercase();
    let query_lc = query.to_lowercase();

    // 计算查询词在内容中的出现次数
    let query_count = content_lc.matches(&query_lc).count() as f32;
    let content_len = content_lc.len() as f32;
    let direct_match_score = (query_count * query_lc.len() as f32) / content_len;
    if direct_match_score > 0.1 {
        // 降低直接匹配的阈值
        let base_score = 0.5 + direct_match_score * 0.5; // 调整权重
        return base_score.min(0.95);
    }

    // 分词处理
    let query_tokens = tokenize(&query_lc);
    let content_tokens = tokenize(&content_lc);

    // 空内容处理
    if content_tokens.ordered.is_empty() {
        return 0.0;
    }

    // 计算词频和位置信息
    let mut term_stats = HashMap::new();
    let mut first_positions = HashMap::new();
    for (pos, word) in content_tokens.ordered.iter().enumerate() {
        *term_stats.entry(word).or_insert(0) += 1;
        first_positions.entry(word).or_insert(pos);
    }

    let total_terms = content_tokens.ordered.len() as f32;
    let mut weighted_density = 0.0;
    let mut position_score = 0.0;

    // 计算 TF-IDF 和位置得分
    for word in &query_tokens.ordered {
        if let Some(count) = term_stats.get(word) {
            let tf = *count as f32 / total_terms;
            let idf = 1.0 + (total_terms / (*count as f32 + 1.0)).ln_1p();
            weighted_density += tf * idf;

            // 考虑词语的位置
            if let Some(pos) = first_positions.get(word) {
                position_score += 1.0 / (1.0 + (*pos as f32 / 10.0));
            }
        }
    }

    // 滑动窗口优化，使用可变窗口大小
    let mut max_window_score: f32 = 0.0;
    for window_size in 3..=7 {
        let window_score: f32 = content_tokens
            .ordered
            .windows(window_size)
            .map(|w| {
                let matches = w
                    .iter()
                    .filter(|word| query_tokens.unique.contains(*word))
                    .count() as f32;
                (matches / window_size as f32).powi(2)
            })
            .fold(0.0_f32, |acc, s| acc.max(s));
        max_window_score = max_window_score.max(window_score);
    }

    // 综合计算最终分数
    let normalized_density = weighted_density / query_tokens.ordered.len() as f32;
    let normalized_position = position_score / query_tokens.ordered.len() as f32;

    let final_score = (
        normalized_density * 0.4 +    // TF-IDF 得分
        max_window_score * 0.4 +      // 词语距离得分
        normalized_position * 0.2
        // 位置得分
    )
    .min(0.95);

    // 如果有至少一个查询词匹配，确保最小分数
    if final_score > 0.0 {
        final_score.max(0.1)
    } else {
        final_score
    }
}

fn strtotime(time_str: &str) -> Option<DateTime<Local>> {
    if let Ok(duration) = parse_duration(time_str) {
        let now = Local::now();
        Some(now - duration)
    } else {
        None
    }
}

fn time_score(publish_time: DateTime<Local>) -> f32 {
    let days = (Local::now() - publish_time).num_days() as f32;
    // 新曲线：7天0.8，30天0.6，180天0.3
    1.0 / (0.03 * days + 1.0).powf(0.4)
}

pub struct Tokenized {
    pub ordered: Vec<String>,
    pub unique: HashSet<String>,
}

/// Tokenizes the input text into a set of words, filtering out stop words.
///
/// # Arguments
/// * `text` - The input text to tokenize.
///
/// # Returns
/// A set of tokens (words) after filtering stop words.
pub fn tokenize(text: &str) -> Tokenized {
    // Pre-process text to normalize and remove punctuation
    let normalized = text
        .chars()
        .map(|c| {
            match c {
                // Convert full-width to half-width characters
                c if c >= '\u{ff01}' && c <= '\u{ff5e}' => {
                    // ASCII对应范围是 0x21~0x7e
                    let ascii = (c as u32 - 0xff01 + 0x21) as u8;
                    ascii as char
                }
                // Common CJK punctuation
                '\u{3001}' | '\u{3002}' | '\u{ff0c}' => ',', // 顶点符、句号、逗号
                '\u{3010}' | '\u{3011}' => '-',              // 方括号
                '\u{300c}' | '\u{300d}' => '"',              // 引号
                '\u{300e}' | '\u{300f}' => '\'',             // 双引号
                '\u{300a}' | '\u{300b}' => '<',              // 书名号
                '\u{2014}' | '\u{2015}' => '-',              // 破折号
                '\u{2018}' | '\u{2019}' => '\'',             // 单引号
                '\u{201c}' | '\u{201d}' => '"',              // 双引号
                '\u{2026}' => '.',                           // 省略号
                '\u{3000}' => ' ',                           // 全角空格
                // Keep useful symbols
                c if is_cjk(c) => c,
                c if c.is_alphanumeric() => c,
                c if c.is_whitespace() => ' ',
                // Special characters that might be meaningful
                '#' | '@' | '&' | '+' | '-' | '*' | '/' | '\\' | '_' => c,
                // Replace all other punctuation with space
                _ => ' ',
            }
        })
        .collect::<String>();

    // 检测语言和脚本
    let (lang, _) = detect_lang_and_script(&normalized);

    // 根据不同的语言选择分词策略
    let mut words = match lang {
        "zh" => chinese_tokenize(&normalized),
        "ja" => japanese_tokenize(&normalized),
        _ => space_based_tokenize(&normalized), // 包括韩语在内的其他语言都使用空格分词
    };

    // 过滤停用词
    words = filter_stop_words(words, lang);

    // 合并连续的数字和英文字符
    let mut merged = Vec::new();
    let mut current = String::new();
    let mut is_last_alnum = false;

    for word in words {
        let trimmed = word.trim();
        if trimmed.is_empty() {
            continue;
        }

        let is_current_alnum = trimmed.chars().all(|c| c.is_ascii_alphanumeric());
        if is_current_alnum && is_last_alnum {
            current.push_str(trimmed);
        } else {
            if !current.is_empty() {
                merged.push(current);
            }
            current = trimmed.to_string();
        }
        is_last_alnum = is_current_alnum;
    }

    if !current.is_empty() {
        merged.push(current);
    }

    // 去除重复并保持顺序
    let mut unique = HashSet::new();
    let ordered: Vec<String> = merged
        .into_iter()
        .filter(|s| s.len() > 1 && unique.insert(s.clone()))
        .collect();

    Tokenized { unique, ordered }
}

/// Detects the language and script of the input text.
///
/// # Arguments
/// * `text` - The input text to analyze.
///
/// # Returns
/// A tuple containing the language code and script.
pub fn detect_lang_and_script(text: &str) -> (&'static str, Script) {
    // 优先检查是否包含中日韩字符
    let mut has_han = false;
    let mut has_kana = false;
    let mut has_hangul = false;

    for c in text.chars() {
        let script = unicode_script::Script::from(c);
        match script {
            unicode_script::Script::Han => {
                has_han = true;
                break; // 只要发现汉字就可以确定是中文
            }
            unicode_script::Script::Hiragana | unicode_script::Script::Katakana => {
                if !has_han {
                    // 如果没有汉字，才考虑假名
                    has_kana = true;
                }
            }
            unicode_script::Script::Hangul => {
                if !has_han && !has_kana {
                    // 如果没有汉字和假名，才考虑谚文
                    has_hangul = true;
                }
            }
            _ => continue,
        }
    }

    // 优先识别中日韩文本
    if has_han {
        return ("zh", Script::Mandarin);
    } else if has_kana {
        return ("ja", Script::Hiragana);
    } else if has_hangul {
        return ("ko", Script::Hangul);
    }

    // 如果没有中日韩字符，使用 whatlang 进行语言检测
    let info = whatlang::detect(text)
        .unwrap_or_else(|| whatlang::Info::new(Script::Latin, Lang::Eng, 0.0));
    (info.lang().code(), info.script())
}

/// Tokenizes Chinese text, handling Simplified and Traditional characters.
///
/// # Arguments
/// * `text` - The Chinese text to tokenize.
/// * `script` - The script of the text (Simplified or Traditional).
///
/// # Returns
/// A vector of tokens.
fn chinese_tokenize(text: &str) -> Vec<String> {
    // 使用 jieba-rs 进行中文分词
    JIEBA
        .cut(text, false)
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn is_cjk(c: char) -> bool {
    // Get the Unicode Script of the character
    let script = unicode_script::Script::from(c);

    // Check for CJK scripts
    matches!(
        script,
        unicode_script::Script::Han
            | unicode_script::Script::Hiragana
            | unicode_script::Script::Katakana
            | unicode_script::Script::Hangul
    )
}

/// Tokenizes Japanese text using unicode word boundaries.
///
/// # Arguments
/// * `text` - The Japanese text to tokenize.
///
/// # Returns
/// A vector of tokens.
fn japanese_tokenize(text: &str) -> Vec<String> {
    // 使用 tiny-segmenter 进行日语分词
    ja_tokenize(text)
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Tokenizes text for languages that use spaces as word boundaries.
/// This includes Latin-based languages, Cyrillic scripts, Korean and modern Vietnamese.
///
/// # Arguments
/// * `text` - The text to tokenize.
///
/// # Returns
/// A vector of tokens.
fn space_based_tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .flat_map(|s| {
            // Split numbers and non-numbers
            let mut tokens = Vec::new();
            let mut current = String::new();
            let mut last_type = None;

            for c in s.chars() {
                let current_type = if c.is_numeric() {
                    TokenType::Number
                } else if c.is_alphabetic() {
                    TokenType::Letter
                } else {
                    TokenType::Other
                };

                match last_type {
                    Some(t) if t != current_type => {
                        if !current.is_empty() {
                            tokens.push(current);
                            current = String::new();
                        }
                    }
                    None => {}
                    _ => {}
                }

                current.push(c);
                last_type = Some(current_type);
            }

            if !current.is_empty() {
                tokens.push(current);
            }

            tokens
        })
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, PartialEq)]
enum TokenType {
    Number,
    Letter,
    Other,
}

/// Filters out stop words from a list of tokens based on the language.
///
/// # Arguments
/// * `words` - The list of tokens to filter.
/// * `lang_code` - The language code to select the appropriate stop word list.
///
/// # Returns
/// A set of tokens after removing stop words.
fn filter_stop_words(words: Vec<String>, lang_code: &str) -> Vec<String> {
    let stop_set = STOP_WORDS.get(lang_code).unwrap_or(&phf::phf_set! {});
    words
        .into_iter()
        .filter(|word| !stop_set.contains(word.as_str()))
        .collect()
}

pub fn simhash(text: &str) -> u64 {
    let tokens = tokenize(text);
    let mut counts = [0i32; 64];

    // 使用原始顺序词列表（包含重复）
    // Use the original ordered list of words (including duplicates)
    for token in &tokens.ordered {
        let hash = seahash::hash(token.as_bytes());
        for i in 0..64 {
            if (hash >> i) & 1 == 1 {
                counts[i] += 1;
            } else {
                counts[i] -= 1;
            }
        }
    }

    counts.iter().enumerate().fold(
        0u64,
        |acc, (i, &v)| {
            if v > 0 {
                acc | (1 << i)
            } else {
                acc
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_mapping() {
        let zh_text = "你好世界";
        let (zh_lang, zh_script) = detect_lang_and_script(zh_text);
        assert_eq!(zh_lang, "zh");
        assert_eq!(zh_script, Script::Mandarin);

        let ja_text = "こんにちは";
        let (ja_lang, ja_script) = detect_lang_and_script(ja_text);
        assert_eq!(ja_lang, "ja");
        assert!(ja_script == Script::Hiragana || ja_script == Script::Katakana);
    }

    #[test]
    fn test_relevance_calculation() {
        let mut result = SearchResult {
            title: "Rust异步编程指南".to_string(),
            url: "https://example.com/rust-async".to_string(),
            summary: Some("Rust异步编程指南：深入讲解异步编程原理...".to_string()),
            ..SearchResult::default()
        };

        let score1 = compute_relevance(&mut result, "Rust异步编程指南");
        assert!(score1 >= 0.95);
    }

    #[test]
    fn test_simhash() {
        let text1 = "This is a test";
        let text2 = "This is another test";
        let text3 = "This is a test";

        let hash1 = simhash(text1);
        let hash2 = simhash(text2);
        let hash3 = simhash(text3);

        assert_ne!(hash1, hash2); // 不同文本的哈希值应该不同
        assert_eq!(hash1, hash3); // 相同文本的哈希值应该相同
    }

    #[test]
    fn test_detect_lang_and_script() {
        // 纯英文
        let text1 = "This is a test";
        let (lang1, script1) = detect_lang_and_script(text1);
        assert_eq!(lang1, "eng");
        assert_eq!(script1, Script::Latin);

        // 纯中文
        let text2 = "你好世界";
        let (lang2, script2) = detect_lang_and_script(text2);
        assert_eq!(lang2, "zh");
        assert_eq!(script2, Script::Mandarin);

        // 纯日文
        let text3 = "こんにちは";
        let (lang3, script3) = detect_lang_and_script(text3);
        assert_eq!(lang3, "ja");
        assert_eq!(script3, Script::Hiragana);

        // 中英混合
        let text4 = "deepseek你好！";
        let (lang4, script4) = detect_lang_and_script(text4);
        assert_eq!(lang4, "zh");
        assert_eq!(script4, Script::Mandarin);

        // 日英混合
        let text5 = "helloこんにちは";
        let (lang5, script5) = detect_lang_and_script(text5);
        assert_eq!(lang5, "ja");
        assert_eq!(script5, Script::Hiragana);

        // 中日混合（优先中文）
        let text6 = "你好こんにちは";
        let (lang6, script6) = detect_lang_and_script(text6);
        assert_eq!(lang6, "zh");
        assert_eq!(script6, Script::Mandarin);
    }
}
