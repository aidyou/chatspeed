//! Offline Help Skill output for the `cscli` CLI.
//!
//! The bundled Help Skill is the manually maintained source of truth for the
//! capability and documentation index. Keeping the CLI output sourced from the
//! same file prevents the interactive Skill and CLI help from drifting.

use crate::args::{Cli, OutputFormat};
use crate::error::CliError;
use crate::output::{print_json, print_jsonl};
use serde_json::json;
use std::io::Write;

const HELP_SKILL: &str = include_str!("../../assets/skills/help/SKILL.md");

/// Prints the bundled Help Skill without contacting the control plane.
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let document = json!({
        "name": "help",
        "source": "builtin",
        "path": "assets/skills/help/SKILL.md",
        "content": HELP_SKILL,
        "maintenance": "manual",
    });

    match cli.output {
        OutputFormat::Json => print_json(&document),
        OutputFormat::Jsonl => print_jsonl(&document),
        OutputFormat::Human => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = write!(handle, "{HELP_SKILL}");
            if !HELP_SKILL.ends_with('\n') {
                let _ = writeln!(handle);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_help_contains_the_capability_index_and_manual_maintenance_note() {
        assert!(HELP_SKILL.contains("## Capability Operations"));
        assert!(HELP_SKILL.contains("cscli skill install"));
        assert!(HELP_SKILL.contains("cscli mcp install"));
        assert!(HELP_SKILL.contains("## Built-in CLI Invocation"));
        assert!(HELP_SKILL.contains("which cscli"));
        assert!(HELP_SKILL.contains("2>&1"));
        assert!(HELP_SKILL.contains("## Maintenance"));
        assert!(HELP_SKILL.contains("manually maintained"));
    }
}
