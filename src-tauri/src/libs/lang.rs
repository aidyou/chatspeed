use locale_config::Locale;
use phf::phf_map;
use serde_json::Value;
use std::collections::HashMap;

/// Returns a supported interface locale or falls back to English.
pub fn normalize_interface_locale(locale: &str) -> &str {
    match locale {
        "en" | "ja" | "zh-Hans" | "zh-Hant" => locale,
        _ => "en",
    }
}

/// Retrieves the system locale and maps it to a supported interface locale.
pub fn get_system_locale() -> String {
    let locale = Locale::current().to_string();
    let normalized = if locale.starts_with("en-") || locale == "en" {
        "en"
    } else if locale.starts_with("ja-") || locale == "ja" {
        "ja"
    } else {
        match locale.as_str() {
            "zh-CN" | "zh-SG" => "zh-Hans",
            "zh-TW" | "zh-HK" | "zh-MO" => "zh-Hant",
            _ => "en",
        }
    };

    normalized.to_string()
}

/// Loads available languages from the i18n configuration file
///
/// Returns a serde_json::Value containing only the "languages" object
/// Error handling is added to provide better diagnostics
pub fn get_available_lang() -> Result<HashMap<String, String>, String> {
    let languages = include_str!("../../i18n/available_language.json");
    let json: Value = serde_json::from_str(languages).map_err(|e| e.to_string())?;
    Ok(serde_json::from_value(json["languages"].clone()).map_err(|e| e.to_string())?)
}

// whatlang language code to ISO 639-1 language code map
static LANG_MAP: phf::Map<&'static str, &'static str> = phf_map! {
    "afr" => "af",
    "aka" => "ak",
    "amh" => "am",
    "ara" => "ar",
    "aze" => "az",
    "bel" => "be",
    "ben" => "bn",
    "bul" => "bg",
    "cat" => "ca",
    "ces" => "cs",
    "cmn" => "zh",
    "dan" => "da",
    "deu" => "de",
    "ell" => "el",
    "eng" => "en",
    "epo" => "eo",
    "est" => "et",
    "fin" => "fi",
    "fra" => "fr",
    "guj" => "gu",
    "heb" => "he",
    "hin" => "hi",
    "hrv" => "hr",
    "hun" => "hu",
    "hye" => "hy",
    "ind" => "id",
    "ita" => "it",
    "jav" => "jv",
    "jpn" => "ja",
    "kan" => "kn",
    "kat" => "ka",
    "khm" => "km",
    "kor" => "ko",
    "lat" => "la",
    "lav" => "lv",
    "lit" => "lt",
    "mal" => "ml",
    "mar" => "mr",
    "mkd" => "mk",
    "mya" => "my",
    "nep" => "ne",
    "nld" => "nl",
    "nob" => "nb",
    "ori" => "or",
    "pan" => "pa",
    "pes" => "fa",
    "pol" => "pl",
    "por" => "pt",
    "ron" => "ro",
    "rus" => "ru",
    "sin" => "si",
    "slk" => "sk",
    "slv" => "sl",
    "sna" => "sn",
    "spa" => "es",
    "srp" => "sr",
    "swe" => "sv",
    "tam" => "ta",
    "tel" => "te",
    "tgl" => "tl",
    "tha" => "th",
    "tuk" => "tk",
    "tur" => "tr",
    "ukr" => "uk",
    "urd" => "ur",
    "uzb" => "uz",
    "vie" => "vi",
    "yid" => "yi",
    "zul" => "zu",
};

/// Converts whatlang language code string to the ISO 639-1 format
///
/// # Arguments
/// - `lang`: The whatlang language code to convert.
///
/// # Returns
/// - The ISO 639-1 language code.
pub fn lang_to_iso_639_1(lang: &str) -> Result<&'static str, String> {
    LANG_MAP
        .get(lang)
        .copied()
        .ok_or(format!("Language not supported: {}", lang))
}

#[cfg(test)]
mod tests {
    use super::normalize_interface_locale;

    #[test]
    fn normalizes_supported_interface_locales() {
        for locale in ["en", "ja", "zh-Hans", "zh-Hant"] {
            assert_eq!(normalize_interface_locale(locale), locale);
        }
    }

    #[test]
    fn falls_back_to_english_for_unsupported_interface_locales() {
        for locale in ["de", "fr", "es", "pt", "ru", "ko", "", "en-US"] {
            assert_eq!(normalize_interface_locale(locale), "en");
        }
    }
}
