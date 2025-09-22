//! A simple token estimator.

/// A rough heuristic for token estimation.
///
/// This function provides a rough estimation of the number of tokens in a given text.
/// The estimation is based on the character type:
/// - For ASCII characters (common English, punctuation, numbers), it assumes approximately
///   3-4 characters per token, so each character contributes 0.3 to the count.
/// - For non-ASCII characters (assuming CJK and other languages), it uses a heuristic
///   of 1.5 tokens per character.
///
/// The final count is rounded up to the nearest whole number.
///
/// # Arguments
///
/// * `text` - A string slice to estimate the token count for.
///
/// # Returns
///
/// An estimated token count as a `f64`.
pub fn estimate_tokens(text: &str) -> f64 {
    let mut token_count: f64 = 0.0;
    for c in text.chars() {
        if c.is_ascii() {
            // Rough approximation for English text, punctuation, and numbers
            // 1 token ~ 3.3 chars, so 1 char ~ 0.3 tokens
            token_count += 0.3;
        } else {
            // For non-ASCII characters, assume they are mostly CJK.
            // 1.5 tokens per character is a reasonable estimate.
            token_count += 1.5;
        }
    }
    token_count
}
