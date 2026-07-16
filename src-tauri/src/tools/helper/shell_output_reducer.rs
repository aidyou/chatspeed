pub(crate) trait ShellOutputReducer {
    fn matches(&self, normalized_command: &str) -> bool;

    fn reduce(&self, exit_code: i32, raw_content: &str) -> String;
}

const BUILD_FAILURE_TAIL_LINES: usize = 30;
const BUILD_SUCCESS_FALLBACK_TAIL_LINES: usize = 20;

pub(crate) struct FrontendBuildOutputReducer;

impl ShellOutputReducer for FrontendBuildOutputReducer {
    fn matches(&self, normalized_command: &str) -> bool {
        normalized_command
            .split(" && ")
            .any(is_frontend_build_command)
    }

    fn reduce(&self, exit_code: i32, raw_content: &str) -> String {
        if exit_code != 0 {
            return reduce_to_tail(raw_content, BUILD_FAILURE_TAIL_LINES);
        }

        let results = raw_content
            .lines()
            .filter(|line| is_build_result_line(line.trim()))
            .collect::<Vec<_>>();
        if results.is_empty() {
            return reduce_to_tail(raw_content, BUILD_SUCCESS_FALLBACK_TAIL_LINES);
        }

        format!(
            "Exit code: {}\n\nBuild result:\n{}",
            exit_code,
            results.join("\n")
        )
    }
}

pub(crate) fn reduce_with_command_reducers(
    normalized_command: &str,
    exit_code: i32,
    raw_content: &str,
) -> Option<String> {
    let reducers: [&dyn ShellOutputReducer; 1] = [&FrontendBuildOutputReducer];

    reducers
        .iter()
        .find(|reducer| reducer.matches(normalized_command))
        .map(|reducer| reducer.reduce(exit_code, raw_content))
}

fn is_frontend_build_command(command: &str) -> bool {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    matches!(
        tokens.as_slice(),
        ["pnpm", "build", ..]
            | ["npm", "build", ..]
            | ["npm", "run", "build", ..]
            | ["yarn", "build", ..]
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

fn is_build_result_line(line: &str) -> bool {
    line.starts_with("✓ built in ")
        || line.starts_with("Finished ")
        || line.starts_with("Bundling ")
        || line.starts_with("Bundled ")
        || line.starts_with("Done in ")
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
    use super::{reduce_with_command_reducers, FrontendBuildOutputReducer, ShellOutputReducer};

    #[test]
    fn matches_supported_frontend_build_commands() {
        let reducer = FrontendBuildOutputReducer;

        for command in [
            "pnpm build",
            "npm build",
            "npm run build",
            "yarn build",
            "pnpm tauri build",
            "pnpm run tauri build",
            "npm tauri build",
            "npm run tauri build",
            "yarn tauri build",
            "yarn run tauri build",
            "yarm tauri build",
            "yarm run tauri build",
            "cd app && pnpm build --mode production",
        ] {
            assert!(reducer.matches(command), "expected to match {command}");
        }
    }

    #[test]
    fn successful_build_keeps_only_result_lines() {
        let output = "Exit code: 0\n\nstdout:\nvite v6.0.0 building for production...\ndist/assets/index.js 3,303.40 kB\n(!) Some chunks are larger than 500 kB after minification. Consider:\n- Use dynamic import() to code-split the application\n✓ built in 15.99s\nFinished `release` profile [optimized] target(s) in 2m 30s\nBundling ChatSpeed.app\n";

        let reduced = reduce_with_command_reducers("pnpm tauri build", 0, output)
            .expect("frontend build reducer should match");

        assert_eq!(
            reduced,
            "Exit code: 0\n\nBuild result:\n✓ built in 15.99s\nFinished `release` profile [optimized] target(s) in 2m 30s\nBundling ChatSpeed.app"
        );
        assert!(!reduced.contains("dist/assets/index.js"));
        assert!(!reduced.contains("Some chunks are larger"));
    }

    #[test]
    fn failed_build_keeps_tail_for_diagnostics() {
        let output = (1..=40)
            .map(|line| format!("output line {line}"))
            .chain(["error: failed to build application".to_string()])
            .collect::<Vec<_>>()
            .join("\n");

        let reduced = reduce_with_command_reducers("yarn build", 1, &output)
            .expect("frontend build reducer should match");

        assert!(reduced.starts_with("[truncated previous output]"));
        assert!(reduced.contains("error: failed to build application"));
        assert!(!reduced.contains("output line 1\n"));
    }
}
