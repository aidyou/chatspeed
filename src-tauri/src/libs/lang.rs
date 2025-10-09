use locale_config::Locale;
use phf::phf_map;
use serde_json::Value;
use std::collections::HashMap;

/// Retrieves the system's locale and standardizes it.
///
/// This function obtains the current system locale string and converts it
/// to a standardized language code based on predefined rules. If the locale
/// is not within the predefined range, the original locale string is returned.
pub fn get_system_locale() -> String {
    // Get the current system locale
    let locale = Locale::current().to_string();

    if locale.starts_with("en-") || locale == "en" {
        return "en".to_string();
    }
    if locale.starts_with("fr-") || locale == "fr" {
        return "fr".to_string();
    }
    if locale.starts_with("de-") || locale == "de" {
        return "de".to_string();
    }
    if locale.starts_with("es-") || locale == "es" {
        return "es".to_string();
    }
    if locale.starts_with("ja-") || locale == "ja" {
        return "ja".to_string();
    }
    if locale.starts_with("ko-") || locale == "ko" {
        return "ko".to_string();
    }

    // If not one of the above, use the more specific matching from before.
    match locale.as_str() {
        // Simplified Chinese retains zh-CN
        "zh-CN" => "zh-Hans".to_string(),
        // Traditional Chinese is unified to zh-HK
        "zh-TW" | "zh-HK" | "zh-MO" | "zh-SG" => "zh-Hant".to_string(),
        // Other languages retain their original form
        _ => locale,
    }
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
