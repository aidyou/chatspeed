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
        "{}{}[{}] {}:{} {}{}",
        level_color,
        chrono::Local::now().format("%H:%M:%S.%3f "),
        get_level(level),
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
/// let message = "api_key=1234567890&password=secret123";
/// let sanitized_message = replace_sensitive_info(message);
/// assert_eq!(sanitized_message, "api_key=***&password=***");
/// ```
///
/// # Notes
/// - The list of sensitive keywords includes: `api_key`, `key`, `password`, `passwd`, `secret`, `token`, `api`.
/// - The replacement logic looks for the `=` sign after a keyword and replaces the value part with `***`.
/// - If the value part ends with `&`, it replaces up to `&`; otherwise, it replaces to the end of the string.
fn replace_sensitive_info(message: &str) -> String {
    let sensitive_keywords = [
        "api_key", "key", "password", "passwd", "secret", "token", "secret", "api",
    ];
    let mut sanitized_message = message.to_string();

    // replace sensitive keywords
    for keyword in sensitive_keywords {
        if let Some(start) = sanitized_message.find(keyword) {
            // find the '=' symbol after the keyword
            if let Some(equals_pos) = sanitized_message[start..].find('=') {
                // start of the value
                let value_start = start + equals_pos + 1;
                // find the end of the value (next '&' or end of string)
                let value_end = sanitized_message[value_start..]
                    .find('&')
                    .map(|pos| value_start + pos)
                    .unwrap_or(sanitized_message.len());
                // replace the value part with '***'
                sanitized_message.replace_range(value_start..value_end, "***");
            }
        }
    }

    sanitized_message
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
        .level(log::LevelFilter::Info)
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
        "Logger initialized successfully, log file path: {:?}",
        log_file_path
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
    }
}
