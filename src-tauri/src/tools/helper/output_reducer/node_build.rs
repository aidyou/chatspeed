use super::CommandOutputReducer;
use crate::tools::helper::{shell_tokens, split_shell_command_segments};

const BUILD_FAILURE_TAIL_LINES: usize = 30;
const KILOBYTE_BYTES: f64 = 1024.0;

pub(crate) struct NodeBuildReducer;

impl CommandOutputReducer for NodeBuildReducer {
    fn matches(&self, normalized_command: &str) -> bool {
        is_node_build_command(normalized_command)
    }

    fn reduce(&self, _normalized_command: &str, exit_code: i32, raw_content: &str) -> String {
        if exit_code != 0 {
            return format!(
                "Exit code: {exit_code}\n\nBuild failed. Diagnostic tail:\n{}",
                reduce_to_tail(raw_content, BUILD_FAILURE_TAIL_LINES)
            );
        }

        let mut results = raw_content
            .lines()
            .filter(|line| is_build_result_line(line.trim()))
            .map(str::trim)
            .map(str::to_string)
            .collect::<Vec<_>>();
        if let Some(summary) = summarize_build_output(raw_content) {
            results.insert(0, summary.format());
        }
        if results.is_empty() {
            results.push("Build completed. Full output was reduced.".to_string());
        }

        format!(
            "Exit code: {exit_code}\n\nBuild result:\n{}",
            results.join("\n")
        )
    }

    fn persist_complete_output(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct BuildOutputSummary {
    file_count: usize,
    size_bytes: f64,
    size_file_count: usize,
    gzip_file_count: usize,
    gzip_size_bytes: f64,
}

impl BuildOutputSummary {
    fn format(&self) -> String {
        let file_label = if self.file_count == 1 {
            "file"
        } else {
            "files"
        };
        let mut result = format!("Build output: {} {}", self.file_count, file_label);
        if self.size_file_count > 0 {
            result.push_str(&format!(", {}", format_size(self.size_bytes)));
        }
        if self.gzip_file_count > 0 {
            let gzip_file_label = if self.gzip_file_count == 1 {
                "file"
            } else {
                "files"
            };
            result.push_str(&format!(
                " (gzip: {} across {} {})",
                format_size(self.gzip_size_bytes),
                self.gzip_file_count,
                gzip_file_label
            ));
        }
        result
    }
}

fn summarize_build_output(raw_content: &str) -> Option<BuildOutputSummary> {
    let mut summary = BuildOutputSummary::default();
    let mut in_gzip_size_section = false;

    for line in raw_content.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("file sizes after gzip:") {
            in_gzip_size_section = true;
            continue;
        }
        if let Some((size_bytes, gzip_size_bytes)) =
            parse_build_output_line(line, in_gzip_size_section)
        {
            summary.file_count += 1;
            if in_gzip_size_section {
                summary.gzip_file_count += 1;
                summary.gzip_size_bytes += size_bytes;
            } else {
                summary.size_file_count += 1;
                summary.size_bytes += size_bytes;
                if let Some(gzip_size_bytes) = gzip_size_bytes {
                    summary.gzip_file_count += 1;
                    summary.gzip_size_bytes += gzip_size_bytes;
                }
            }
        } else if !line.is_empty() {
            in_gzip_size_section = false;
        }
    }

    (summary.file_count > 0).then_some(summary)
}

fn parse_build_output_line(line: &str, in_gzip_size_section: bool) -> Option<(f64, Option<f64>)> {
    if in_gzip_size_section {
        return parse_cra_gzip_output_line(line).map(|size_bytes| (size_bytes, None));
    }

    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let (path_end, size_index, gzip_size_bytes) = if tokens.len() >= 7
        && tokens[tokens.len() - 3] == "gzip:"
        && tokens[tokens.len() - 4] == "│"
    {
        let gzip_size_bytes = parse_size(tokens[tokens.len() - 2], tokens[tokens.len() - 1])?;
        (tokens.len() - 6, tokens.len() - 6, Some(gzip_size_bytes))
    } else if let Some((size_index, path_index)) = webpack_asset_indices(&tokens) {
        (path_index + 1, size_index, None)
    } else if tokens.len() >= 3 {
        (tokens.len() - 2, tokens.len() - 2, None)
    } else {
        return None;
    };
    let path = tokens[..path_end].join(" ");
    if !looks_like_build_output_path(&path) {
        return None;
    }

    parse_size(tokens[size_index], tokens[size_index + 1])
        .map(|size_bytes| (size_bytes, gzip_size_bytes))
}

fn webpack_asset_indices(tokens: &[&str]) -> Option<(usize, usize)> {
    if tokens.first() != Some(&"asset") || tokens.len() < 4 {
        return None;
    }

    tokens[2..]
        .windows(2)
        .position(|pair| parse_size(pair[0], pair[1]).is_some())
        .map(|offset| (offset + 2, 1))
}

fn parse_cra_gzip_output_line(line: &str) -> Option<f64> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.len() < 3 || !looks_like_build_output_path(&tokens[2..].join(" ")) {
        return None;
    }

    parse_size(tokens[0], tokens[1])
}

fn looks_like_build_output_path(path: &str) -> bool {
    path.contains(&['/', '\\'][..])
        || path
            .rsplit_once(char::is_whitespace)
            .map_or(path, |(_, file_name)| file_name)
            .contains('.')
}

fn parse_size(value: &str, unit: &str) -> Option<f64> {
    let value = value.replace(',', "").parse::<f64>().ok()?;
    let multiplier = match unit.to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" | "kib" => KILOBYTE_BYTES,
        "mb" | "mib" => KILOBYTE_BYTES.powi(2),
        "gb" | "gib" => KILOBYTE_BYTES.powi(3),
        _ => return None,
    };
    Some(value * multiplier)
}

fn format_size(bytes: f64) -> String {
    let (value, unit) = if bytes >= KILOBYTE_BYTES.powi(3) {
        (bytes / KILOBYTE_BYTES.powi(3), "GB")
    } else if bytes >= KILOBYTE_BYTES.powi(2) {
        (bytes / KILOBYTE_BYTES.powi(2), "MB")
    } else if bytes >= KILOBYTE_BYTES {
        (bytes / KILOBYTE_BYTES, "kB")
    } else {
        (bytes, "B")
    };
    format!("{value:.2} {unit}")
}

pub(crate) fn is_node_build_command(normalized_command: &str) -> bool {
    split_shell_command_segments(normalized_command)
        .iter()
        .any(|segment| is_node_build_command_segment(segment))
}

fn is_node_build_command_segment(command: &str) -> bool {
    let Some(tokens) = shell_tokens(command) else {
        return false;
    };
    let tokens = tokens
        .iter()
        .map(String::as_str)
        .skip_while(|token| is_environment_assignment(token))
        .collect::<Vec<_>>();
    matches!(
        tokens.as_slice(),
        ["pnpm", "build", ..]
            | ["pnpm", "run", "build", ..]
            | ["npm", "build", ..]
            | ["npm", "run", "build", ..]
            | ["yarn", "build", ..]
            | ["yarn", "run", "build", ..]
            | ["pnpm", "tauri", "build", ..]
            | ["pnpm", "run", "tauri", "build", ..]
            | ["npm", "tauri", "build", ..]
            | ["npm", "run", "tauri", "build", ..]
            | ["yarn", "tauri", "build", ..]
            | ["yarn", "run", "tauri", "build", ..]
            | ["yarm", "tauri", "build", ..]
            | ["yarm", "run", "tauri", "build", ..]
    )
}

fn is_environment_assignment(token: &&str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn is_build_result_line(line: &str) -> bool {
    line.starts_with("✓ built in ")
        || line.starts_with("Finished ")
        || line.starts_with("Bundling ")
        || line.starts_with("Bundled ")
        || line.starts_with("Done in ")
        || line.starts_with("Compiled successfully")
        || line.contains("compiled successfully")
        || line.starts_with("The build folder is ready")
}

fn reduce_to_tail(raw_content: &str, tail_lines: usize) -> String {
    let lines = raw_content.lines().collect::<Vec<_>>();
    if lines.len() <= tail_lines {
        return raw_content.to_string();
    }

    format!(
        "[truncated previous output]\n{}",
        lines[lines.len().saturating_sub(tail_lines)..].join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::{is_node_build_command, NodeBuildReducer};
    use crate::tools::helper::{reduce_command_output, CommandOutputReducer};

    #[test]
    fn matches_supported_node_build_commands() {
        let reducer = NodeBuildReducer;

        for command in [
            "pnpm build",
            "pnpm run build",
            "npm build",
            "npm run build",
            "yarn build",
            "yarn run build",
            "pnpm tauri build",
            "pnpm run tauri build",
            "npm tauri build",
            "npm run tauri build",
            "yarn tauri build",
            "yarn run tauri build",
            "yarm tauri build",
            "yarm run tauri build",
            "cd app && pnpm build --mode production",
            "cd app; pnpm build --mode production",
            "CI=1 pnpm build",
            "BUILD_LABEL=\"release candidate\" pnpm build",
            "cd app; BUILD_LABEL=\"release candidate\" pnpm build",
        ] {
            assert!(reducer.matches(command), "expected to match {command}");
            assert!(is_node_build_command(command));
        }
    }

    #[test]
    fn successful_build_summarizes_output_from_any_directory() {
        let output = "Exit code: 0\n\nstdout:\nvite v6.0.0 building for production...\nbin/index.html 0.76 kB\nbin/assets/index.js 3,303.40 kB │ gzip: 1,023.37 kB\n(!) Some chunks are larger than 500 kB after minification. Consider:\n- Use dynamic import() to code-split the application\n✓ built in 15.99s\nFinished `release` profile [optimized] target(s) in 2m 30s\nBundling ChatSpeed.app\n";

        let reduction = reduce_command_output("pnpm tauri build", 0, output)
            .expect("node build reducer should match");

        assert_eq!(
            reduction.content,
            "Exit code: 0\n\nBuild result:\nBuild output: 2 files, 3.23 MB (gzip: 1023.37 kB across 1 file)\n✓ built in 15.99s\nFinished `release` profile [optimized] target(s) in 2m 30s\nBundling ChatSpeed.app"
        );
        assert!(reduction.persist_complete_output);
        assert!(!reduction.content.contains("bin/assets/index.js"));
        assert!(!reduction.content.contains("Some chunks are larger"));
    }

    #[test]
    fn successful_webpack_and_cra_builds_summarize_assets() {
        let webpack_output = "asset main.js 244 KiB [emitted] [minimized] (name: main)\nasset main.css 12.5 KiB [emitted] [minimized]\nWARNING in asset size limit: The following asset(s) exceed the recommended size limit (244 KiB).\nwebpack 5.95.0 compiled successfully in 8432 ms\n";
        let webpack_reduction = reduce_command_output("npm run build", 0, webpack_output)
            .expect("node build reducer should match");
        assert_eq!(
            webpack_reduction.content,
            "Exit code: 0\n\nBuild result:\nBuild output: 2 files, 256.50 kB\nwebpack 5.95.0 compiled successfully in 8432 ms"
        );
        assert!(!webpack_reduction.content.contains("asset main.js"));
        assert!(!webpack_reduction
            .content
            .contains("WARNING in asset size limit"));

        let cra_output = "Creating an optimized production build...\nCompiled successfully.\n\nFile sizes after gzip:\n\n  46.6 kB  build/static/js/main.abc.js\n  1.77 kB  build/static/css/main.def.css\n\nThe build folder is ready to be deployed.\n";
        let cra_reduction = reduce_command_output("yarn build", 0, cra_output)
            .expect("node build reducer should match");
        assert_eq!(
            cra_reduction.content,
            "Exit code: 0\n\nBuild result:\nBuild output: 2 files (gzip: 48.37 kB across 2 files)\nCompiled successfully.\nThe build folder is ready to be deployed."
        );
        assert!(!cra_reduction
            .content
            .contains("build/static/js/main.abc.js"));
    }

    #[test]
    fn failed_build_uses_uniform_diagnostic_format() {
        let output = (1..=40)
            .map(|line| format!("output line {line}"))
            .chain(["error: failed to build application".to_string()])
            .collect::<Vec<_>>()
            .join("\n");

        let reduction = reduce_command_output("yarn build", 1, &output)
            .expect("node build reducer should match");

        assert!(reduction.content.starts_with(
            "Exit code: 1\n\nBuild failed. Diagnostic tail:\n[truncated previous output]"
        ));
        assert!(reduction
            .content
            .contains("error: failed to build application"));
        assert!(!reduction.content.contains("output line 1\n"));
        assert!(reduction.persist_complete_output);
    }
}
