use std::collections::{HashMap, HashSet};

use phf::{phf_map, phf_set};
use url::Url;

use super::similarity::{
    compute_content_relevance, compute_relevance, compute_title_relevance, detect_lang_and_script,
    simhash, tokenize,
};
use crate::http::crawler::SearchResult;

/// Common news title suffixes in different languages
static TITLE_SUFFIXES: phf::Map<&'static str, phf::Set<&'static str>> = phf_map! {
    "zh-Hans" => phf_set! {
        "新闻", "资讯", "来源", "官方", "最新", "热点", "详情", "专题"
    },
    "zh-Hant" => phf_set! {
        "新聞", "資訊", "來源", "官方", "最新", "熱點", "詳情", "專題"
    },
    "en" => phf_set! {
        "news", "source", "official", "latest", "update", "exclusive", "breaking", "story"
    },
    "ja" => phf_set! {
        "ニュース", "通信", "通信社", "通信社発", "通信社配信", "通信社配信記事"
    },
    "ko" => phf_set! {
        "뉴스", "소식", "공식", "최신", "업데이트", "전용"
    }
};

pub struct SimilarityChecker {
    seen_hashes: HashSet<u64>,
    query: String,
}

impl SimilarityChecker {
    pub fn new(query: &str) -> Self {
        Self {
            seen_hashes: HashSet::new(),
            query: query.to_lowercase(),
        }
    }

    pub fn is_duplicate(&mut self, title: &str, content: &Option<String>) -> bool {
        // 标题标准化
        let normalized_title = normalize_title(title);
        let title_hash = self.calculate_weighted_hash(&normalized_title);

        // 内容相似度检测
        let content_sim = content
            .as_ref()
            .map_or(0.0, |c| compute_content_relevance(c, &self.query));

        // 计算标题相似度
        let title_sim = compute_title_relevance(&normalized_title, &self.query);

        // 1. 如果标题完全相同（考虑标准化后），判定为重复
        println!("title: {}, content_sim: {:.2}", title, content_sim);
        if self.seen_hashes.contains(&title_hash) {
            // 对于完全相同的标题，如果内容也有一定相似度，更可能是重复
            if content_sim > 0.4 {
                println!(
                    "Exact title match with similar content - title:{}, content_sim:{:.2}",
                    title, content_sim
                );
                self.seen_hashes.insert(title_hash);
                return true;
            }
        }

        // 2. 如果内容和标题都相似，判定为重复
        if content_sim > 0.7 && title_sim > 0.8 {
            println!(
                "High similarity - title:{}, content_sim:{:.2}, title_sim:{:.2}",
                title, content_sim, title_sim
            );
            self.seen_hashes.insert(title_hash);
            return true;
        }

        // 3. SimHash 检测 - 只在内容相似度较高时进行
        if content_sim > 0.6 {
            if let Some(content) = content {
                let content_hash = self.calculate_weighted_hash(content);
                let min_hamming = self
                    .seen_hashes
                    .iter()
                    .map(|&h| (h ^ content_hash).count_ones())
                    .min()
                    .unwrap_or(64);

                // 如果汉明距离很小且标题也相似
                if min_hamming < 4 && title_sim > 0.6 {
                    println!(
                        "SimHash similarity - title:{}, hamming:{}, content_sim:{:.2}, title_sim:{:.2}",
                        title, min_hamming, content_sim, title_sim
                    );
                    self.seen_hashes.insert(content_hash);
                    return true;
                }
                self.seen_hashes.insert(content_hash);
            }
        }

        // 4. 记录新的标题哈希
        self.seen_hashes.insert(title_hash);
        false
    }

    fn calculate_weighted_hash(&self, text: &str) -> u64 {
        let normalized_text = normalize_title(text);
        let tokens = tokenize(&normalized_text);
        let mut counts = [0i32; 64];

        // 基于词频的归一化权重
        let total_tokens = tokens.ordered.len() as f32;
        let term_freq: HashMap<_, _> = tokens
            .ordered
            .iter()
            .map(|t| {
                (
                    t,
                    tokens.ordered.iter().filter(|&x| x == t).count() as f32 / total_tokens,
                )
            })
            .collect();

        for token in &tokens.ordered {
            let hash = seahash::hash(token.as_bytes());
            let weight = (term_freq[token] * 1000.0) as i32; // 放大词频影响
            for i in 0..64 {
                if (hash >> i) & 1 == 1 {
                    counts[i] += weight;
                } else {
                    counts[i] -= weight;
                }
            }
        }

        counts.iter().enumerate().fold(
            0u64,
            |acc, (i, &v)| if v > 0 { acc | (1 << i) } else { acc },
        )
    }
}

/// Normalizes a title by removing common suffixes, punctuation, and extra whitespace.
/// Also handles language-specific title patterns.
///
/// # Arguments
/// * `title` - The title to normalize
///
/// # Returns
/// A normalized version of the title
pub fn normalize_title(title: &str) -> String {
    // Detect text language
    let (lang_code, _) = detect_lang_and_script(title);

    // Normalize whitespace
    let mut normalized = title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Remove common separators and special characters
    normalized = normalized
        .trim_end_matches(|c| matches!(c, '_' | '-' | '|') || c.is_whitespace())
        .to_string();

    // Basic normalization
    normalized = normalized
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>();

    // Remove language-specific suffixes
    if let Some(suffixes) = TITLE_SUFFIXES.get(lang_code) {
        for suffix in suffixes.iter() {
            if normalized.ends_with(suffix) {
                normalized = normalized[..normalized.len() - suffix.len()]
                    .trim()
                    .to_string();
            }
        }
    }

    normalized
}

pub fn dedup_and_rank_results(mut results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    if results.is_empty() {
        return results;
    }

    // 第一阶段：智能URL去重
    // Phase 1: Intelligent URL Deduplication
    let mut url_map: HashMap<String, SearchResult> = HashMap::new();
    results.retain(|res| match normalize_url(&res.url) {
        Ok(norm_url) => {
            let res_clone = res.clone();
            url_map.insert(norm_url, res_clone).is_none()
        }
        Err(_) => true,
    });

    // Phase 2: Calculate relevance and sort
    for result in &mut results {
        compute_relevance(result, query);
    }
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Phase 3: Semantic Deduplication using simhash
    semantic_deduplicate(results)
}

static REMOVE_PARAMS: phf::Set<&'static str> = phf_set! {
    // UTM 相关参数
    "utm", "utm_source", "utm_medium", "utm_campaign", "utm_term",
    "utm_content", "utm_id", "utm_placement", "utm_network", "utm_device",
    "utm_adgroup", "utm_target", "utm_ad", "utm_adformat", "utm_adposition", "utm_adtype",

    // 社交平台追踪参数
    "fbclid", "igshid", "twclid", "twsrc", "twcamp", "twterm",

    // 广告平台追踪参数
    "gclid", "dclid", "msclkid", "yclid", "clickid", "click_id", "campaign", "adgroup", "creative",
    "label", "keywordid", "creativeid", "mediatype",

    // 电商平台追踪参数
    "spm", "ddclick_reco", "scm", "scid", "trackid", "refcode", "promo", "coupon", "voucher",

    // 其他常见追踪参数
    "ref", "referrer", "referer", "source",  "affiliate", "aff_id", "partner", "partner_id",
    "campid", "adid", "ad_id", "placement", "channel", "subid", "sub_id", "transaction_id", "session_id",

    // 全链路追踪参数
    "traceparent", "tracestate", "request_id", "correlation_id",

    // 短链服务参数
    "shortlink", "short_url", "tinyurl", "bitly", "goo.gl", "ow.ly", "t.co",

    // 自定义参数
    "custom_param", "custom_id", "client_id", "device_id", "browser_id",

    // 时间戳相关
    "timestamp", "time", "date", "expires", "ttl",

    "wfr", "for","from","fr",

    // 其他杂项
    "redirect", "return_url", "callback", "callback_url", "next", "next_url", "continue", "continue_url"
};

fn normalize_url(url_str: &str) -> Result<String, url::ParseError> {
    let mut url = Url::parse(url_str)?;

    // 统一协议和主机名大小写
    url.set_scheme("https").unwrap_or(());
    let host_str = url.host_str().unwrap_or("").to_lowercase();
    url.set_host(Some(&host_str))?;

    // 处理查询参数
    let query_pairs = url.query_pairs();
    let filtered: Vec<_> = query_pairs
        .filter(|(k, _)| !REMOVE_PARAMS.contains(k.as_ref()))
        .collect();

    // 按字母顺序排序参数以归一化
    let mut params: Vec<_> = filtered
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    params.sort_by(|a, b| a.0.cmp(&b.0));

    // 重建URL，不使用clone()来避免保留原始参数
    let mut new_url = Url::parse(&format!(
        "{}://{}",
        url.scheme(),
        url.host_str().unwrap_or("")
    ))?;
    new_url.set_path(url.path());

    // 添加过滤后的查询参数
    if !params.is_empty() {
        let new_query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        new_url.set_query(Some(&new_query));
    }

    // 移除锚点
    new_url.set_fragment(None);

    Ok(new_url.to_string())
}

/// Performs semantic deduplication using simhash for both title and content.
///
/// # Arguments
/// * `results` - Search results to deduplicate
/// * `query` - Search query string
///
/// # Returns
/// Deduplicated search results
/// Performs semantic deduplication using simhash for both title and content.
///
/// # Arguments
/// * `results` - Search results to deduplicate
///
/// # Returns
/// Deduplicated search results
fn semantic_deduplicate(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen_title_hashes = HashSet::new();
    let mut seen_content_hashes = HashSet::new();
    let mut deduped = Vec::new();

    for res in results {
        // Calculate title simhash
        let title_hash = simhash(&normalize_title(&res.title));
        let title_duplicate = seen_title_hashes.iter().any(|&h: &u64| {
            let hamming_distance = (h ^ title_hash).count_ones();
            hamming_distance <= 3 // Hamming distance threshold for titles
        });

        // Calculate content simhash if available
        let content_duplicate = if let Some(content) = &res.summary {
            let content_hash = simhash(content);
            seen_content_hashes.iter().any(|&h: &u64| {
                let hamming_distance = (h ^ content_hash).count_ones();
                hamming_distance <= 5 // Hamming distance threshold for content
            })
        } else {
            false
        };

        // Keep if neither title nor content is duplicate
        if !title_duplicate && !content_duplicate {
            seen_title_hashes.insert(title_hash);
            if let Some(content) = &res.summary {
                seen_content_hashes.insert(simhash(content));
            }
            deduped.push(res.clone());
        }

        #[cfg(debug_assertions)]
        {
            if title_duplicate || content_duplicate {
                println!("Duplicate found: {}", &res.title);
            }
        }
    }

    deduped
}

mod tests {
    use crate::{
        http::crawler::{Crawler, SearchProvider, SearchResult},
        libs::dedup::dedup_and_rank_results,
    };

    #[test]
    fn test_dynamic_urls() {
        let results = vec![
            // utm param will be removed
            SearchResult {
                url: "https://example.com/article?id=123&utm=track".into(),
                title: "Rust教程".into(),
                ..Default::default()
            },
            // ref param will be removed
            SearchResult {
                url: "https://example.com/article?id=123&ref=social".into(),
                title: "Rust教程".into(),
                ..Default::default()
            },
            // wfr param and for param will be removed
            SearchResult {
                url: "https://example.com/article?id=123&wfr=social&for=pc".into(),
                title: "Rust教程".into(),
                ..Default::default()
            },
            // 不同ID参数
            SearchResult {
                url: "https://example.com/article?id=456".into(),
                title: "Rust高级技巧".into(),
                ..Default::default()
            },
            // 相同标题不同内容
            SearchResult {
                url: "https://example.net/post?pid=789".into(),
                title: "Rust并发编程".into(),
                summary: Some("Mutex使用指南".into()),
                ..Default::default()
            },
            SearchResult {
                url: "https://example.net/post?pid=790".into(),
                title: "Rust并发编程".into(),
                summary: Some("Channel最佳实践".into()),
                ..Default::default()
            },
        ];

        let deduped = dedup_and_rank_results(results, "Rust编程");

        assert_eq!(deduped.len(), 3);
    }

    #[tokio::test]
    async fn test_dedup_and_rank_results() {
        // crawl news from baidu news and google news
        let keywords = "五粮液最近怎样，可以买吗？";
        let mut result = vec![];
        let providers = vec![SearchProvider::Baidu, SearchProvider::Google];
        for provider in providers {
            // The crawler is running in a separate process
            // Must be install chatspeedbot
            let crawler = Crawler::new("http://127.0.0.1:12321".to_string());
            let res = crawler
                .search(provider.clone(), &[keywords], Some(1), Some(30), true)
                .await
                .unwrap();
            let search_count = res.len();
            result.extend(res);
            println!(
                "{} search result count: {}",
                provider.to_string(),
                search_count
            );
        }
        println!("Total search result count: {}", result.len());
        // let json = serde_json::to_string_pretty(&result).expect("Failed to serialize results");
        // std::fs::write("search.json", json).expect("Failed to write results to file");

        // load from cache file
        // let result = std::fs::read_to_string("search.json").unwrap();
        // let result: Vec<SearchResult> = serde_json::from_str(&result).unwrap();
        // println!("Total search result count: {}", result.len());
        let deduped = dedup_and_rank_results(result, keywords);
        println!("Deduplicated result count: {}", deduped.len());
        assert!(deduped.len() > 0);

        // let json = serde_json::to_string_pretty(&deduped).expect("Failed to serialize results");
        // std::fs::write("search_deduped.json", json).expect("Failed to write results to file");
    }
}
