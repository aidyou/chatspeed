use base64::{engine::general_purpose, Engine as _};
use log::debug;
use regex::Regex;
use reqwest::get;
use url::Url;

use crate::libs::util::urldecode;
pub async fn get_meta_refresh_url(url: &str) -> Option<String> {
    if !url.contains("so.com/link") && !url.contains("sogou.com/link") {
        return None;
    }

    match get(url).await {
        Ok(response) if response.status().is_success() => {
            if let Ok(body) = response.text().await {
                let re = match regex::Regex::new(r"URL='([^']+)'") {
                    Ok(r) => r,
                    Err(_) => return None,
                };

                if let Some(captures) = re.captures(&body) {
                    let extracted = &captures[1];
                    if url::Url::parse(extracted).is_ok() {
                        return Some(extracted.to_string());
                    }
                }

                // parse from window.location.replace
                let re = match regex::Regex::new(r#"window\.location\.replace\("([^"]+)"\)"#) {
                    Ok(r) => r,
                    Err(_) => return None,
                };

                if let Some(captures) = re.captures(&body) {
                    let extracted = &captures[1];
                    if url::Url::parse(extracted).is_ok() {
                        return Some(extracted.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    None
}

pub fn decode_bing_url(bing_url: &str) -> Option<String> {
    debug!("Processing URL: {}", bing_url);

    if let Some(url) = decode_bing_base64_url(bing_url) {
        return Some(url);
    }

    if let Some(url) = decode_bing_direct_url(bing_url) {
        return Some(url);
    }

    if let Some(url) = decode_bing_redirect_url(bing_url) {
        return Some(url);
    }

    extract_url_from_query_params(bing_url)
}

/// Handles Base64 encoded URLs
fn decode_bing_base64_url(bing_url: &str) -> Option<String> {
    let patterns = [r"u=([^&]+)", r"r=([^&]+)", r"url=([^&]+)"];

    for pattern in patterns.iter() {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(bing_url) {
                let encoded = &captures[1];

                let base64_str = if encoded.starts_with("a1a") {
                    &encoded[3..]
                } else if encoded.starts_with("a2a") {
                    &encoded[3..]
                } else if encoded.starts_with("a3") {
                    &encoded[2..]
                } else {
                    encoded
                };

                if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(base64_str) {
                    if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                        if is_valid_url(&original_url) {
                            return Some(original_url);
                        }
                    }
                } else {
                    let padded = add_base64_padding(base64_str);
                    if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(&padded) {
                        if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                            if is_valid_url(&original_url) {
                                return Some(original_url);
                            }
                        }
                    } else {
                        if let Ok(decoded_bytes) = general_purpose::URL_SAFE.decode(base64_str) {
                            if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                                if is_valid_url(&original_url) {
                                    return Some(original_url);
                                }
                            }
                        } else {
                            if let Ok(decoded_bytes) = general_purpose::URL_SAFE.decode(&padded) {
                                if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                                    if is_valid_url(&original_url) {
                                        return Some(original_url);
                                    }
                                }
                            }
                        }
                    }
                }

                if let Ok(decoded_param) = urldecode(encoded) {
                    if let Ok(decoded_bytes) =
                        general_purpose::STANDARD.decode(decoded_param.as_bytes())
                    {
                        if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                            if is_valid_url(&original_url) {
                                return Some(original_url);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Handles direct URL encoding
fn decode_bing_direct_url(bing_url: &str) -> Option<String> {
    let patterns = [
        r"u=([^&]+)",
        r"r=([^&]+)",
        r"url=([^&]+)",
        r"redirect_url=([^&]+)",
        r"link=([^&]+)",
    ];

    for pattern in patterns.iter() {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(bing_url) {
                let encoded = &captures[1];

                if let Ok(decoded) = urldecode(encoded) {
                    let url_str = decoded.to_string();
                    if is_valid_url(&url_str) {
                        return Some(url_str);
                    }
                }

                let cleaned = encoded
                    .trim_start_matches("a1")
                    .trim_start_matches("a2")
                    .trim_start_matches("a3");

                if cleaned != encoded {
                    if let Ok(decoded) = urldecode(cleaned) {
                        let url_str = decoded.to_string();
                        if is_valid_url(&url_str) {
                            return Some(url_str);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Handles the redirect URL pattern
fn decode_bing_redirect_url(bing_url: &str) -> Option<String> {
    let redirect_patterns = [
        r"https://www\.bing\.com/ck/a\?!.*&u=([^&]+)",
        r"https://www\.bing\.com/ck/a\?!.*&r=([^&]+)",
        r"https://www\.bing\.com/ck/a\?!.*&url=([^&]+)",
    ];

    for pattern in redirect_patterns.iter() {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(bing_url) {
                let encoded = &captures[1];
                if let Some(url) = try_decode_encoded_url(encoded) {
                    return Some(url);
                }
            }
        }
    }

    None
}

/// Extracts URL from query parameters
fn extract_url_from_query_params(bing_url: &str) -> Option<String> {
    if let Ok(url) = Url::parse(bing_url) {
        for (key, value) in url.query_pairs() {
            if key == "u" || key == "r" || key == "url" || key == "redirect_url" || key == "link" {
                if let Ok(decoded) = urldecode(&value) {
                    let url_str = decoded.to_string();
                    if is_valid_url(&url_str) {
                        return Some(url_str);
                    }
                }

                if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(value.as_bytes()) {
                    if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                        if is_valid_url(&original_url) {
                            return Some(original_url);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Tries multiple ways to decode an encoded URL
fn try_decode_encoded_url(encoded: &str) -> Option<String> {
    if let Ok(decoded) = urldecode(encoded) {
        let url_str = decoded.to_string();
        if is_valid_url(&url_str) {
            return Some(url_str);
        }
    }

    if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(encoded) {
        if let Ok(original_url) = String::from_utf8(decoded_bytes) {
            if is_valid_url(&original_url) {
                return Some(original_url);
            }
        }
    } else {
        let padded = add_base64_padding(encoded);
        if let Ok(decoded_bytes) = general_purpose::STANDARD.decode(&padded) {
            if let Ok(original_url) = String::from_utf8(decoded_bytes) {
                if is_valid_url(&original_url) {
                    return Some(original_url);
                }
            }
        }
    }

    let cleaned = encoded
        .trim_start_matches("a1")
        .trim_start_matches("a2")
        .trim_start_matches("a3")
        .trim_start_matches("url=");

    if cleaned != encoded {
        return try_decode_encoded_url(cleaned);
    }

    None
}

/// Validates if a string is a well-formed http/https URL
fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Adds necessary padding to a Base64 string
fn add_base64_padding(s: &str) -> String {
    let remainder = s.len() % 4;
    if remainder == 0 {
        s.to_string()
    } else {
        let padding = 4 - remainder;
        format!("{}{}", s, "=".repeat(padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_meta_refresh_url() {
        let so_url = "https://www.so.com/link?m=wAM%2FjzKW%2Bnnw6e3G4F0MXfYDRJvNtWCUpXYMiUw0FDpXb79HTy2kfMzzju8vCF5K7QLW7ITIj%2Bbq380rBeDroiT5XILuEJ97ZO5sieoCdT9btVEWXf7ZuDRPh6p29lOcPKgN7a2anxnD6jURTLVux2XGREuk%3D";
        assert_eq!(
            get_meta_refresh_url(so_url).await,
            Some("https://www.cnblogs.com/Grewer/p/12789261.html".to_string())
        );

        let so_url = "https://www.so.com/link?m=bFILiTJwx0FodHNKTKOr00CwO5c%2FNNT4Ds0opmnHxviV6gE9M23FZNgsW7u0sNah3zQv7GIoB228sukyzrgmZa1Mt6ahUmkPhf1%2F26pH4RAln0Uwx9I7lwGMOUyEXmSGd8R4d0tCmmUGgR2wdpgNCSoaAMGQ5BIpuG%2FaX%2F9yoIZA%3D";
        assert_eq!(
            get_meta_refresh_url(so_url).await,
            Some("https://blog.csdn.net/wan212000/article/details/125432957".to_string())
        );

        let so_url = "https://www.so.com/link?m=zoKVCHxt%2B%2FagaIAOXmQVJ56X8uFEF%2B71lI49Ijl7hnCtKUVLNMo%2FMu%2FH%2F3GZA3cwNhPu2XsDrGJD8jO4gUgD3LT1s51IsyGR4w%2BMRaB%2FwyqiWx1Q3s1wQ%2Fug6ox%2BwaxiUAwo3PZ%2Fun879a6YXNARZbU9j5NGKv5BpP6rxgZoebKs%3D";
        assert_eq!(
            get_meta_refresh_url(so_url).await,
            Some("https://blog.csdn.net/zdsx1104/article/details/125026526".to_string())
        );

        let so_url = "https://www.so.com/link?m=ukSfUyUUpJ4eAFggN88e8Q5XPNFAdhUT8yiCNzNuHzCLOWU2HsMRNed0JpuciYfTXSgep1NbX6ZIi%2Bw9vckVNRewaaTKRSQgFXuBuzFiPXNSzvxvp93Sqcr7zoOn9Z8B7rS1ipP%2FYhi5tkv7I";
        assert_eq!(
            get_meta_refresh_url(so_url).await,
            Some("https://www.51cto.com/article/720608.html".to_string())
        );

        let sougou_url = "https://www.sogou.com/link?url=hedJjaC291OhctwIJyMURczUaPImU8YR";
        assert_eq!(
            get_meta_refresh_url(sougou_url).await,
            Some("https://tauri.studio/".to_string())
        );

        let sougou_url = "https://www.sogou.com/link?url=hedJjaC291OHSfRZxx--pdfZ45aIPvhNrynoH4S1IZp3dsjpqTIyDdJ1cUoC0dHFEWmyEg-B1pwKm75UZhhKwXWJulowaO7Lp9uOj4z9auQ.";
        assert_eq!(
            get_meta_refresh_url(sougou_url).await,
            Some(
                "https://www.sogou.com/web?ie=utf8&query=tauri%20framework site:blog.csdn.net"
                    .to_string()
            )
        );

        let sougou_url =
            "https://www.sogou.com/link?url=hedJjaC291MuovqUW6cN1tNvFnZxSx9T6xKCqZIjYRU.";
        assert_eq!(
            get_meta_refresh_url(sougou_url).await,
            Some("https://github.com/hube12/tauri".to_string())
        );
    }

    #[test]
    fn test_decode_bing_url() {
        // Test traditional Base64 encoding
        let bing_url1 = "https://www.bing.com/ck/a?!&&p=4cb9e15ea20592d8b145d86e52d06be5a0f49d1f2b9e3f72f9e984fd76df93aaJmltdHM9MTc1OTAxNzYwMA&ptn=3&ver=2&hsh=4&fclid=07d9c9a5-2d21-61b6-2e86-dff42c8260f1&u=a1aHR0cHM6Ly90YXVyaS5hcHAv&ntb=1";
        assert_eq!(
            decode_bing_url(bing_url1),
            Some("https://tauri.app/".to_string())
        );

        // Test another Base64 URL
        let bing_url2 = "https://www.bing.com/ck/a?!&&p=5b5e694ee637a930ece6e75c4ecb3d57cda7f07475e3e4ef945c95bdcf704861JmltdHM9MTc1OTAxNzYwMA&ptn=3&ver=2&hsh=4&fclid=07d9c9a5-2d21-61b6-2e86-dff42c8260f1&u=a1aHR0cHM6Ly9naXRodWIuY29tL3RhdXJpLWFwcHMvdGF1cmk&ntb=1";
        assert_eq!(
            decode_bing_url(bing_url2),
            Some("https://github.com/tauri-apps/tauri".to_string())
        );

        // Test direct URL encoding
        let bing_url3 = "https://www.bing.com/ck/a?!&&u=https%3A%2F%2Fexample.com%2Ftest";
        assert_eq!(
            decode_bing_url(bing_url3),
            Some("https://example.com/test".to_string())
        );

        // Test invalid URL
        let invalid_url = "https://example.com/no-u-param";
        assert_eq!(decode_bing_url(invalid_url), None);
    }
}
