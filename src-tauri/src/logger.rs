use lazy_static::*;
use regex::Regex;
use rust_i18n::t;
use std::fs::File;
use tauri::Manager;

/// Simplifies file paths by extracting relevant parts from cargo registry paths
///
/// # Arguments
/// * `file_path` - The file path to simplify
///
/// # Returns
/// A simplified version of the file path
fn simplify_file_path(file_path: &str) -> String {
    if file_path.contains("chatspeed") {
        if let Some(pos) = file_path.rfind("/src/") {
            return file_path[(pos + 1)..].to_string();
        }
    }

    if let Some((_, suffix)) = file_path.split_once(".cargo/registry/src/") {
        if let Some(first_slash) = suffix.find('/') {
            suffix[(first_slash + 1)..].to_string()
        } else {
            suffix.to_string()
        }
    } else {
        file_path.to_string()
    }
}

/// Formats log messages for console output with a simplified format
///
/// # Arguments
/// * `out` - The format callback to write the formatted message
/// * `message` - The log message to format
/// * `record` - The log record containing metadata
///
/// # Features
/// * Simplified time format (HH:MM:SS)
/// * Concise log format for console viewing
/// * Filtering for non-project related low-level logs
pub fn console_log_formatter(
    out: fern::FormatCallback,
    message: &std::fmt::Arguments,
    record: &log::Record,
) {
    let level = record.level();
    let level_color = match level {
        log::Level::Error => "\x1B[31m", // red
        log::Level::Warn => "\x1B[33m",  // yellow
        log::Level::Info => "\x1B[32m",  // green
        log::Level::Debug => "\x1B[0m",  // normal
        log::Level::Trace => "\x1B[35m", // purple
    };
    let reset = "\x1B[0m";

    out.finish(format_args!(
        "{}{}[{}] {} - {}:{} {}{}",
        level_color,
        chrono::Local::now().format("%H:%M:%S.%3f "),
        get_level(level),
        record.target(),
        simplify_file_path(record.file().unwrap_or("")),
        record.line().unwrap_or(0),
        replace_sensitive_info(message.to_string().as_str()),
        reset,
    ))
}

/// Formats log messages for file output with detailed information
///
/// # Arguments
/// * `out` - The format callback to write the formatted message
/// * `message` - The log message to format
/// * `record` - The log record containing metadata
///
/// # Features
/// * Complete date-time format (YYYY-MM-DD HH:MM:SS)
/// * Includes thread ID, target module, and file location
/// * Preserves sufficient context for all logs for troubleshooting
pub fn file_log_formatter(
    out: fern::FormatCallback,
    message: &std::fmt::Arguments,
    record: &log::Record,
) {
    out.finish(format_args!(
        "{}[{}] {}:{} {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S.%3f"),
        get_level(record.level()),
        simplify_file_path(record.file().unwrap_or("")),
        record.line().unwrap_or(0),
        replace_sensitive_info(message.to_string().as_str()),
    ))
}

lazy_static! {
    /// Regex for sanitizing sensitive information in logs.
    ///
    /// This regex is defined as a compile-time constant and `unwrap()` is intentionally used for the following reasons:
    /// 1. If the regex contains a syntax error, this represents a **coding bug** that should be caught during development or CI, not at runtime in production.
    /// 2. Log sanitization is a security-critical feature. If the regex fails to compile, the safest response is to panic and stop execution rather than silently skip or output unsanitized logs.
    /// 3. As this expression is never loaded from user input or configuration, runtime failure is never expected after passing compile and tests. Using `unwrap()` ensures any issue is quickly and explicitly detected.
    ///
    /// If you change this regex, ensure you have thorough tests to prevent breaking production log sanitization.
     pub static ref SENSITIVE_REGEX: Regex = Regex::new(
        &format!(
            r#"(?ix) # Ignore case and allow comments/whitespace. Ensure < > are actual angle brackets.
                (?P<key_str> # Capture the full key string (e.g., "api_key" or api_key)
                    ["']? # Optional opening quote for the key
                    (?:{keyword_alternatives}) # Non-capturing group for keyword list
                    ["']? # Optional closing quote for the key
                )
                (?P<s1>\s*)(?P<sep>[:=])(?P<s2>\s*) # Separator and spaces
                (?: # Value alternatives. We don't capture the whole value string into one group,
                    # but rather check sub-groups to determine quoting for replacement.
                    (?P<val_q_double_open>")(?P<val_double_quoted_content>(?:\\.|[^\\"])*?)(?P<val_q_double_close>") # Double quoted string
                    |
                    (?P<val_q_single_open>')(?P<val_single_quoted_content>(?:\\.|[^\\'])*?)(?P<val_q_single_close>') # Single quoted string
                    |
                    (?P<val_unquoted_content>[^"'\s,}}&]*) # Unquoted value
                )
            "#,
            keyword_alternatives = "api_key|apikey|access_token|refresh_token|card_number|bank_account|client_secret|app_secret|proxyPassword|proxyUsername|user_id|sessionid|set-cookie|credit_card|password|passwd|secret|token|user|username|account|email|mail|mobile|phone|telephone|id_card|idnumber|session|cookie|device_token|authentication|credentials|auth|api|jwt|key|otp|pin|pwd|ssn|tk"
        )
    ).unwrap();
}

/// Replaces sensitive information in log messages with asterisks (`***`).
///
/// This function scans the log message for sensitive keywords (e.g., `api_key`, `password`),
/// and replaces their corresponding values with `***` to prevent sensitive data leakage.
///
/// # Arguments
/// * `message` - The log message to sanitize.
///
/// # Returns
/// A sanitized version of the log message, where sensitive values are replaced with `***`.
///
/// # Examples
/// ```
/// let message = "api_key=1234567890&password=1234567890&secret=1234567890";
/// let sanitized_message = replace_sensitive_info(message);
/// assert_eq!(sanitized_message, "api_key=***&password=***&secret=***");
/// ```
///
/// # Notes
/// - The list of sensitive keywords includes: `api_key`, `key`, `password`, `passwd`, `secret`, `token`, `api`.
/// - The replacement logic looks for the `=` sign after a keyword and replaces the value part with `***`.
/// - If the value part ends with `&`, it replaces up to `&`; otherwise, it replaces to the end of the string.
fn replace_sensitive_info(message: &str) -> String {
    SENSITIVE_REGEX
        .replace_all(message, |caps: &regex::Captures| {
            // Safely access capture groups.
            // If the regex matched, these named groups should be present.
            // Defaulting to empty string if a group is unexpectedly missing,
            // though this would indicate an issue with the regex itself.
            let key_str = caps.name("key_str").map_or("", |m| m.as_str());
            let s1 = caps.name("s1").map_or("", |m| m.as_str());
            let sep_char = caps.name("sep").map_or("", |m| m.as_str());
            let s2 = caps.name("s2").map_or("", |m| m.as_str());

            // Determine the replacement string for the value part based on original quoting
            let value_replacement_str = if caps.name("val_q_double_open").is_some() {
                // Value was double-quoted, e.g., "secret"
                // The groups val_q_double_open, val_double_quoted_content, val_q_double_close
                // would have matched the original quoted value. We replace the content.
                "\"***\""
            } else if caps.name("val_q_single_open").is_some() {
                // Value was single-quoted, e.g., 'secret'
                "'***'"
            } else {
                // Value was unquoted (the val_unquoted_content group matched)
                "***"
            };

            format!(
                "{}{}{}{}{}",
                key_str, s1, sep_char, s2, value_replacement_str
            )
        })
        .to_string()
}

/// Sets up the application logger with console and file outputs
///
/// # Arguments
/// * `app` - A reference to the Tauri application
pub fn setup_logger(app: &tauri::App) {
    // Initialize log directory and file
    let log_dir = app
        .path()
        .app_log_dir()
        .expect(&t!("main.failed_to_retrieve_log_directory"));

    *LOG_DIR.write() = log_dir.clone();

    let log_file_path = log_dir.join("chatspeed.log");
    let ccproxy_log_path = log_dir.join("ccproxy.log");

    // Ensure log directory exists
    std::fs::create_dir_all(&log_dir).expect(&t!("main.failed_to_create_log_directory"));
    // Create log file
    File::create(&log_file_path).expect(&t!("main.failed_to_create_log_file"));
    File::create(&ccproxy_log_path).expect("Failed to create ccproxy.log");

    // Create base dispatcher
    let base_dispatcher = fern::Dispatch::new().level(log::LevelFilter::Debug);

    // Console dispatcher - concise format
    let stdout_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Debug
        })
        .format(console_log_formatter)
        .chain(std::io::stdout());

    // File dispatcher - detailed format
    let file_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed")
                || (record.level() < log::LevelFilter::Debug && record.target() != "ccproxy_logger")
        })
        .format(file_log_formatter)
        .chain(fern::log_file(&log_file_path).expect(&t!("main.failed_to_create_log_file")));

    // ccproxy dispatcher - raw format
    let ccproxy_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Info)
        .filter(|record| record.target() == "ccproxy_logger")
        .format(|out, message, _| out.finish(format_args!("{}", message)))
        .chain(fern::log_file(&ccproxy_log_path).expect("Failed to create ccproxy.log"));

    // Apply logger configuration
    base_dispatcher
        .chain(stdout_dispatcher)
        .chain(file_dispatcher)
        .chain(ccproxy_dispatcher)
        .apply()
        .expect(&t!("main.failed_to_initialize_logger"));

    log::info!(
        "Logger initialized successfully, log file path: {:?}",
        log_file_path
    );
    log::info!(
        "Logger initialized successfully, ccproxy log file path: {:?}",
        ccproxy_log_path
    );
}

fn get_level(level: log::Level) -> String {
    match level {
        log::Level::Error => "E",
        log::Level::Warn => "W",
        log::Level::Info => "I",
        log::Level::Debug => "D",
        log::Level::Trace => "T",
    }
    .to_string()
}

#[cfg(test)]
use log::SetLoggerError;

use crate::constants::LOG_DIR;

/// Set up logger for tests
///
/// Only output to console in test environment, using concise format
#[cfg(test)]
pub fn setup_test_logger() -> Result<(), SetLoggerError> {
    use std::env;
    use std::path::PathBuf;

    if log::logger().enabled(&log::Metadata::builder().level(log::Level::Debug).build()) {
        return Ok(()); // Logger already initialized
    }
    let dev_root: PathBuf = env::var("PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("Failed to get current directory"))
        .parent()
        .unwrap()
        .into();
    let log_dir = dev_root.join("dev_data").join("logs");
    let log_file_path = log_dir.join("chatspeed.log");
    // Ensure log directory exists
    std::fs::create_dir_all(&log_dir).expect(&t!("main.failed_to_create_log_directory"));
    // Create log file
    File::create(&log_file_path).expect(&t!("main.failed_to_create_log_file"));

    // Create base dispatcher
    let base_dispatcher = fern::Dispatch::new().level(log::LevelFilter::Debug);

    // Console dispatcher - concise format
    let stdout_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Debug
        })
        .format(console_log_formatter)
        .chain(std::io::stdout());

    // File dispatcher - detailed format
    let file_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Info
        })
        .format(file_log_formatter)
        .chain(fern::log_file(&log_file_path).expect(&t!("main.failed_to_create_log_file")));

    // Apply logger configuration
    base_dispatcher
        .chain(stdout_dispatcher)
        .chain(file_dispatcher)
        .apply()
        .expect(&t!("main.failed_to_initialize_logger"));

    log::debug!(
        "Test logger initialized successfully, log file path: {:?}",
        log_file_path
    );
    Ok(())
}

mod tests {

    #[test]
    fn test_replace_sensitive_info() {
        let message = "api_key=1234567890&password=1234567890&secret=1234567890";
        let sanitized_message = crate::logger::replace_sensitive_info(message);
        assert_eq!(sanitized_message, "api_key=***&password=***&secret=***");

        let logs = r#"
        {
            "chat_param": {
                "api_key": "sk-2SAc-fB43a2-dCdB8F1",
                "api_url": "https://abc.com/v1",
                "proxyPassword": "a1232323232",
                "proxyServer": "http://127.0.0.1:15154?passwd=<PASSWORD>",
                "proxyUsername": "abcdefg",
                },
            },
            "is_internal_tool_result": true,
            "windowLabel": "main"
        }"#;
        let sanitized_message1 = crate::logger::replace_sensitive_info(logs);
        print!("{}", sanitized_message1);
        assert!(sanitized_message1.contains(r#""api_key": "***""#));
    }
}
