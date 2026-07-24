use super::CommandOutputReducer;
use crate::tools::helper::{contains_unquoted_shell_operator, shell_tokens};
use serde_json::Value;

const MAX_FAILURES: usize = 10;
const MAX_FAILURE_LINES: usize = 12;

pub(crate) struct TestOutputReducer;

impl CommandOutputReducer for TestOutputReducer {
    fn matches(&self, normalized_command: &str) -> bool {
        test_framework(normalized_command).is_some()
    }

    fn reduce(&self, normalized_command: &str, exit_code: i32, raw_content: &str) -> String {
        match test_framework(normalized_command) {
            Some(TestFramework::Pytest) => reduce_pytest_output(exit_code, raw_content),
            Some(TestFramework::Vitest) => reduce_vitest_output("Vitest", exit_code, raw_content),
            Some(TestFramework::Jest) => reduce_vitest_output("Jest", exit_code, raw_content),
            Some(TestFramework::JavaScript) => {
                reduce_vitest_output("JavaScript test", exit_code, raw_content)
            }
            None => raw_content.to_string(),
        }
    }

    fn persist_complete_output(&self) -> bool {
        true
    }
}

enum TestFramework {
    Pytest,
    Vitest,
    Jest,
    JavaScript,
}

fn test_framework(command: &str) -> Option<TestFramework> {
    if contains_unquoted_shell_operator(command) {
        return None;
    }

    let tokens = shell_tokens(command)?
        .into_iter()
        .skip_while(is_environment_assignment)
        .collect::<Vec<_>>();
    match tokens.as_slice() {
        [python, module, framework, ..]
            if matches!(python.as_str(), "python" | "python3")
                && module == "-m"
                && framework == "pytest" =>
        {
            Some(TestFramework::Pytest)
        }
        [framework, ..] if framework == "pytest" => Some(TestFramework::Pytest),
        [runner, subcommand, framework, ..]
            if runner == "uv" && subcommand == "run" && framework == "pytest" =>
        {
            Some(TestFramework::Pytest)
        }
        [runner, subcommand, python, module, framework, ..]
            if runner == "uv"
                && subcommand == "run"
                && matches!(python.as_str(), "python" | "python3")
                && module == "-m"
                && framework == "pytest" =>
        {
            Some(TestFramework::Pytest)
        }
        [runner, framework, ..]
            if matches!(runner.as_str(), "npx" | "pnpm" | "yarn") && framework == "vitest" =>
        {
            Some(TestFramework::Vitest)
        }
        [runner, executable, framework, ..]
            if matches!(runner.as_str(), "pnpm" | "yarn")
                && executable == "exec"
                && framework == "vitest" =>
        {
            Some(TestFramework::Vitest)
        }
        [framework, ..] if framework == "vitest" => Some(TestFramework::Vitest),
        [runner, framework, ..]
            if matches!(runner.as_str(), "npx" | "pnpm" | "yarn") && framework == "jest" =>
        {
            Some(TestFramework::Jest)
        }
        [runner, executable, framework, ..]
            if matches!(runner.as_str(), "pnpm" | "yarn")
                && executable == "exec"
                && framework == "jest" =>
        {
            Some(TestFramework::Jest)
        }
        [framework, ..] if framework == "jest" => Some(TestFramework::Jest),
        [runner, script, ..]
            if matches!(runner.as_str(), "npm" | "pnpm" | "yarn") && script == "test" =>
        {
            Some(TestFramework::JavaScript)
        }
        [runner, subcommand, script, ..]
            if matches!(runner.as_str(), "npm" | "pnpm" | "yarn")
                && subcommand == "run"
                && script == "test" =>
        {
            Some(TestFramework::JavaScript)
        }
        _ => None,
    }
}

fn is_environment_assignment(token: &String) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn reduce_pytest_output(exit_code: i32, raw_content: &str) -> String {
    let lines = raw_content.lines().collect::<Vec<_>>();
    let summary = lines
        .iter()
        .rev()
        .find(|line| is_pytest_summary(line.trim()))
        .map(|line| line.trim().trim_matches('=').trim());
    let failure_lines = collect_pytest_failure_lines(&lines);

    if summary.is_none() && failure_lines.is_empty() {
        return raw_content.to_string();
    }

    let mut output = format!("Exit code: {exit_code}\n\nPytest result:");
    if let Some(summary) = summary {
        output.push_str("\n");
        output.push_str(summary);
    }
    append_failure_list(&mut output, &failure_lines);
    output
}

fn collect_pytest_failure_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut failures = Vec::new();
    let mut in_failures = false;

    for line in lines {
        let trimmed = line.trim();
        if is_pytest_failures_header(trimmed) {
            in_failures = true;
            continue;
        }
        if in_failures && is_pytest_short_summary_header(trimmed) {
            break;
        }
        if in_failures && !trimmed.is_empty() {
            failures.push(trimmed);
        }
    }

    let summary_failures = lines
        .iter()
        .filter_map(|line| {
            let line = line.trim();
            (line.starts_with("FAILED ") || line.starts_with("ERROR ")).then_some(line)
        })
        .collect::<Vec<_>>();

    if failures.is_empty() {
        return summary_failures;
    }

    for failure in summary_failures.into_iter().rev() {
        failures.insert(0, failure);
    }

    failures
}

fn is_pytest_failures_header(line: &str) -> bool {
    line.contains("FAILURES") && line.chars().any(|character| character == '=')
}

fn is_pytest_short_summary_header(line: &str) -> bool {
    line.contains("short test summary") && line.chars().any(|character| character == '=')
}

fn is_pytest_summary(line: &str) -> bool {
    (line.contains(" passed")
        || line.contains(" failed")
        || line.contains(" skipped")
        || line.contains(" xfailed")
        || line.contains(" xpassed")
        || line.contains("no tests ran"))
        && (line.contains(" in ") || line.contains("no tests ran"))
}

fn reduce_vitest_output(framework: &str, exit_code: i32, raw_content: &str) -> String {
    if let Some(output) = reduce_test_json_output(framework, exit_code, raw_content) {
        return output;
    }

    let lines = raw_content.lines().collect::<Vec<_>>();
    let summary_lines = lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| {
            line.starts_with("Test Files")
                || line.starts_with("Tests")
                || line.starts_with("Duration")
                || line.starts_with("Snapshots")
        })
        .collect::<Vec<_>>();
    let failure_lines = collect_vitest_failure_lines(&lines);

    if summary_lines.is_empty() && failure_lines.is_empty() {
        return raw_content.to_string();
    }

    let mut output = format!("Exit code: {exit_code}\n\n{framework} result:");
    for line in summary_lines {
        output.push('\n');
        output.push_str(line);
    }
    append_failure_list(&mut output, &failure_lines);
    output
}

fn reduce_test_json_output(framework: &str, exit_code: i32, raw_content: &str) -> Option<String> {
    let json = raw_content.split_once("\n\nstdout:\n")?.1.trim();
    let value = serde_json::from_str::<Value>(json).ok()?;
    let total = value.get("numTotalTests")?.as_u64()?;
    let passed = value
        .get("numPassedTests")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let failed = value
        .get("numFailedTests")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let failed_suites = value
        .get("numFailedTestSuites")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let skipped = value
        .get("numPendingTests")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let failures = extract_json_failures(&value);

    let mut output = format!(
        "Exit code: {exit_code}\n\n{framework} result:\nTests {total} total | {passed} passed | {failed} failed | {skipped} skipped"
    );
    if failed_suites > 0 {
        output.push_str(&format!("\nTest suites {failed_suites} failed"));
    }
    append_json_failures(&mut output, &failures);
    Some(output)
}

fn extract_json_failures(value: &Value) -> Vec<String> {
    value
        .get("testResults")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|file| {
            let file_name = file
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown test file");
            let suite_error = file
                .get("testExecError")
                .and_then(json_error_message)
                .or_else(|| file.get("message").and_then(Value::as_str))
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(|message| format!("{file_name}: {message}"));
            let assertion_failures = file
                .get("assertionResults")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(move |test| {
                    (test.get("status").and_then(Value::as_str) == Some("failed")).then(|| {
                        let name = test
                            .get("fullName")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown test");
                        let message = test
                            .get("failureMessages")
                            .and_then(Value::as_array)
                            .and_then(|messages| messages.first())
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|message| !message.is_empty())
                            .unwrap_or("failure details unavailable");
                        format!("{file_name} > {name}: {message}")
                    })
                });
            suite_error.into_iter().chain(assertion_failures)
        })
        .collect()
}

fn json_error_message(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("message").and_then(Value::as_str))
}

fn append_json_failures(output: &mut String, failures: &[String]) {
    if failures.is_empty() {
        return;
    }

    output.push_str("\n\nFailures:\n");
    for failure in failures.iter().take(MAX_FAILURES) {
        output.push_str(failure);
        output.push('\n');
    }
    let omitted = failures.len().saturating_sub(MAX_FAILURES);
    if omitted > 0 {
        output.push_str(&format!("... {omitted} additional failures omitted\n"));
    }
}

fn collect_vitest_failure_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut failures = Vec::new();
    let mut in_failures = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.contains("Failed Tests") || trimmed.starts_with("FAIL ") {
            in_failures = true;
        }
        if in_failures && !trimmed.is_empty() {
            failures.push(trimmed);
        }
        if in_failures
            && (trimmed.starts_with("Test Files")
                || trimmed.starts_with("Tests")
                || trimmed.starts_with("Duration"))
        {
            failures.pop();
            break;
        }
    }

    failures
}

fn append_failure_list(output: &mut String, failures: &[&str]) {
    if failures.is_empty() {
        return;
    }

    output.push_str("\n\nFailures:\n");
    for line in failures.iter().take(MAX_FAILURES * MAX_FAILURE_LINES) {
        output.push_str(line);
        output.push('\n');
    }
    let omitted = failures
        .len()
        .saturating_sub(MAX_FAILURES * MAX_FAILURE_LINES);
    if omitted > 0 {
        output.push_str(&format!(
            "... {omitted} additional failure-detail lines omitted\n"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{test_framework, TestOutputReducer};
    use crate::tools::helper::CommandOutputReducer;

    #[test]
    fn recognizes_supported_test_commands() {
        for command in [
            "pytest",
            "python -m pytest tests",
            "python3 -m pytest -q",
            "uv run pytest",
            "uv run python -m pytest tests",
            "uv run python3 -m pytest tests",
            "pnpm vitest run",
            "pnpm exec vitest run",
            "npx vitest run",
            "yarn jest",
            "npx jest --runInBand",
            "npm test",
            "pnpm run test",
        ] {
            assert!(test_framework(command).is_some(), "expected {command}");
        }
        assert!(test_framework("echo pytest").is_none());
        for command in [
            "pytest && git status",
            "pytest; git status",
            "pytest || git status",
            "pytest | cat",
            "pnpm vitest run |& cat",
        ] {
            assert!(test_framework(command).is_none(), "unexpected {command}");
        }
    }

    #[test]
    fn compacts_pytest_progress_while_retaining_the_summary_and_failures() {
        let output = "============================= test session starts =============================\ncollected 3 items\n\ntests/example.py .F. [100%]\n\n=================================== FAILURES ===================================\n_______________________________ test_value _______________________________\n>       assert actual == 2\nE       AssertionError: expected 2\ntests/example.py:12: AssertionError\n\n=========================== short test summary info ============================\nFAILED tests/example.py::test_value - AssertionError: expected 2\n========================= 2 passed, 1 failed in 0.12s =========================";
        let reduced = TestOutputReducer.reduce("uv run pytest", 1, output);

        assert!(reduced.contains("2 passed, 1 failed in 0.12s"));
        assert!(reduced.contains("FAILED tests/example.py::test_value"));
        assert!(reduced.contains("assert actual == 2"));
        assert!(reduced.contains("tests/example.py:12"));
        assert!(!reduced.contains("test session starts"));
    }

    #[test]
    fn compacts_vitest_progress_while_retaining_failures_and_counts() {
        let output = " RUN  v3.0.0 /workspace\n\n ✓ src/ok.test.ts (2 tests)\n ⨯ src/fail.test.ts (1 test)\n\n⎯⎯⎯ Failed Tests 1 ⎯⎯⎯\n\n FAIL  src/fail.test.ts > fails\nAssertionError: expected true to be false\n\n Test Files  1 failed | 1 passed (2)\n      Tests  1 failed | 2 passed (3)\n   Duration  1.20s";
        let reduced = TestOutputReducer.reduce("pnpm vitest run", 1, output);

        assert!(reduced.contains("Test Files  1 failed | 1 passed (2)"));
        assert!(reduced.contains("Tests  1 failed | 2 passed (3)"));
        assert!(reduced.contains("FAIL  src/fail.test.ts > fails"));
        assert!(!reduced.contains("✓ src/ok.test.ts"));
    }

    #[test]
    fn compacts_test_reporter_json_and_retains_failure_details() {
        let output = "Exit code: 1\n\nstdout:\n{\"numTotalTests\":3,\"numPassedTests\":2,\"numFailedTests\":1,\"numPendingTests\":0,\"testResults\":[{\"name\":\"src/example.test.ts\",\"assertionResults\":[{\"fullName\":\"example fails\",\"status\":\"failed\",\"failureMessages\":[\"expected true to be false\"]}]}]}";
        let reduced = TestOutputReducer.reduce("pnpm vitest run --reporter=json", 1, output);

        assert!(reduced.contains("Tests 3 total | 2 passed | 1 failed | 0 skipped"));
        assert!(reduced.contains("src/example.test.ts > example fails"));
        assert!(reduced.contains("expected true to be false"));
    }

    #[test]
    fn compacts_failed_test_suites_in_json_reports() {
        let output = "Exit code: 1\n\nstdout:\n{\"numTotalTests\":0,\"numPassedTests\":0,\"numFailedTests\":0,\"numPendingTests\":0,\"numFailedTestSuites\":1,\"testResults\":[{\"name\":\"src/setup.test.ts\",\"testExecError\":{\"message\":\"Cannot find module './setup'\"},\"assertionResults\":[]}]}";
        let reduced = TestOutputReducer.reduce("npx jest --json", 1, output);

        assert!(reduced.contains("Test suites 1 failed"));
        assert!(reduced.contains("src/setup.test.ts: Cannot find module './setup'"));
    }

    #[test]
    fn preserves_unknown_test_output() {
        let output = "custom test runner output";
        assert_eq!(TestOutputReducer.reduce("pytest", 0, output), output);
    }
}
