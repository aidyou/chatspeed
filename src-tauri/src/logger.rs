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
        message,
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
        message
    ))
}

/// Sets up the application logger with console and file outputs
///
/// # Arguments
/// * `app` - A reference to the Tauri application
pub fn setup_logger(app: &tauri::App) {
    // 初始化日志目录和文件
    let log_dir = app
        .path()
        .app_log_dir()
        .expect(&t!("main.failed_to_retrieve_log_directory"));
    let log_file_path = log_dir.join("chatspeed.log");
    // 确保日志目录存在
    std::fs::create_dir_all(&log_dir).expect(&t!("main.failed_to_create_log_directory"));
    // 创建日志文件
    File::create(&log_file_path).expect(&t!("main.failed_to_create_log_file"));

    // 创建基础日志分发器
    let base_dispatcher = fern::Dispatch::new().level(log::LevelFilter::Debug);

    // 控制台日志分发器 - 使用简洁格式
    let stdout_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Debug
        })
        .format(console_log_formatter)
        .chain(std::io::stdout());

    // 文件日志分发器 - 使用详细格式
    let file_dispatcher = fern::Dispatch::new()
        .level(log::LevelFilter::Info)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Info
        })
        .format(file_log_formatter)
        .chain(fern::log_file(&log_file_path).expect(&t!("main.failed_to_create_log_file")));

    // 应用日志配置
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

/// 为测试设置日志记录器
///
/// 在测试环境中只输出到控制台，使用简洁格式
#[cfg(test)]
pub fn setup_test_logger() -> Result<(), SetLoggerError> {
    if log::logger().enabled(&log::Metadata::builder().level(log::Level::Debug).build()) {
        return Ok(()); // 日志器已经初始化
    }

    fern::Dispatch::new()
        .format(console_log_formatter)
        .level(log::LevelFilter::Debug)
        .filter(|record| {
            record.target().contains("chatspeed") || record.level() < log::LevelFilter::Debug
        })
        .chain(std::io::stdout())
        .apply()
        .map_err(|e| {
            log::error!("Failed to initialize logger: {:?}", e);
            e
        })?;

    log::debug!("Test logger initialized successfully");
    Ok(())
}
