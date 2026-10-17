//! Clap command tree for the `cscli` CLI.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Long help for `--agent-config`: the full camelCase inheritedAgentConfig
/// contract, so callers do not need to read ChatSpeed source to use it.
const AGENT_CONFIG_LONG_HELP: &str = "\
Raw inherited agent config JSON (camelCase, same contract as the Tauri \
create_workflow inheritedAgentConfig). Supported top-level keys:

- personality: execution/communication style preset id
- allowedPaths: [string] authorized directory paths
- approvalLevel: \"default\" | \"smart\" | \"full\"
- autoApprove: [tool] extra auto-approved tools (bash is always excluded)
- autoApprovePlan: bool, approve generated plans without confirmation
- autoCompress: bool, task-boundary rollup compression
- availableTools: [tool] tool subset; intersected with the agent's own tools
- finalAudit: bool (legacy flag, kept in sync with finalReviewMode)
- finalReviewMode: \"off\" | \"sub_agent_review\"
- skillEnabled: bool; selectedSkills: [string]
- mcpTools: MCP tool exposure config
- phase: default workflow phase
- models: {plan|act|vision|utility|lite: {id, model, temperature?, \
contextSize?, maxTokens?, functionCall?}}
- shellPolicy: [{pattern, decision: \"Allow\" | \"Review\" | \"Deny\"}]
- sandboxExecutionMode / sandboxSchemeId / sandboxConfig / sandboxOverride

Notes: maxContexts is not inherited; unknown keys are ignored; --model is a \
shortcut for models.act.";

/// Every friendly automation field flag, so the raw document inputs
/// (`--spec-json`/`--spec-file`/`--intent`) can declare themselves mutually
/// exclusive with the friendly path from one place.
const FRIENDLY_ARG_IDS: [&str; 19] = [
    "title",
    "agent",
    "prompt",
    "prompt_file",
    "every",
    "daily",
    "once",
    "weekdays",
    "start_date",
    "end_date",
    "anchor_time",
    "allowed_paths",
    "clear_allowed_paths",
    "enabled",
    "disabled",
    "continuous_context",
    "no_continuous_context",
    "self_review",
    "no_self_review",
];

/// Machine-stable output formats. `human` is localized and may change;
/// `json`/`jsonl` are versioned machine contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    Jsonl,
}

#[derive(Debug, Parser)]
#[command(
    name = "cscli",
    disable_help_subcommand = true,
    version,
    about = "ChatSpeed workflow CLI",
    long_about = "ChatSpeed workflow CLI.\n\n\
        `cscli` talks to the running ChatSpeed app over its local loopback\n\
        control plane. It never opens the database or runs agents itself.\n\n\
        Exit codes:\n  \
        0  success\n  \
        1  server-side or I/O error\n  \
        2  CLI usage error\n  \
        3  ChatSpeed not running / discovery missing or stale / connection failure\n  \
        4  authentication failure\n  \
        5  incompatible control-plane protocol version"
)]
pub struct Cli {
    /// Output format: human, json or jsonl.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    pub output: OutputFormat,

    /// Explicit discovery file path (default:
    /// ${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json).
    #[arg(long, global = true)]
    pub discovery_file: Option<PathBuf>,

    /// Locale for human output: en, zh-Hans or zh-Hant.
    #[arg(long, global = true)]
    pub lang: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the bundled Help Skill and capability/documentation index offline.
    Help,
    /// Check connectivity, authentication and protocol compatibility.
    Doctor {
        #[command(subcommand)]
        command: Option<DoctorCommand>,
    },
    /// Discover agents known to the running ChatSpeed app.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Create, run, inspect and control workflows.
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    /// Inspect and manage Agent Skills (file-based capabilities).
    ///
    /// Read-only in this phase: it never opens the database or a capability
    /// directory and never installs anything.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Install, remove, start, stop and inspect the MCP servers managed by the
    /// running ChatSpeed app.
    ///
    /// Every mutation goes through the app's single capability service over the
    /// authenticated control plane; the CLI never touches the database, the
    /// config cache or a server process itself. Listing tools never invokes a
    /// tool.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Create, inspect, draft, apply, schedule and run local workflow
    /// Automations managed by the running ChatSpeed app.
    ///
    /// Every mutation goes through the app's single automation facade over the
    /// authenticated control plane; the CLI never opens the database, starts a
    /// workflow, or runs a shell command itself. `draft` is side-effect-free and
    /// can never grant a new shell/path/network/MCP/Skill permission.
    Automation {
        #[command(subcommand)]
        command: Box<AutomationCommand>,
    },
}

/// Additive `cscli doctor` sub-capabilities.
#[derive(Debug, Subcommand)]
pub enum DoctorCommand {
    /// Report the capability journal, Skill ownership, MCP desired/runtime
    /// drift and private staging residue (report-only; never mutates).
    Capabilities,
    /// Converge capability drift the durable evidence proves, then report what
    /// changed. Anything whose effect state cannot be proven is left in
    /// `needs_reconcile`; nothing is blind-retried or deleted on a guess.
    Reconcile {
        /// Idempotency key for the reconcile mutation. One is minted when the
        /// flag is omitted so a retry cannot run twice.
        #[arg(long = "idempotency-key")]
        idempotency_key: Option<String>,
    },
}

/// Read-only Agent Skill commands.
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List the registered Skill install targets and whether each is usable.
    Targets,
    /// List installed Agent Skills with their ownership and drift state.
    List,
    /// Check a Skill source with the non-LLM checker, without installing.
    Check {
        /// The structured source document (`{"kind": ...}`), inline.
        #[arg(long, conflicts_with = "source_file")]
        source_json: Option<String>,
        /// The structured source document, read from a file (`-` for stdin).
        #[arg(long)]
        source_file: Option<PathBuf>,
    },
    /// Install a Skill that passed the checker into the selected targets.
    Install {
        /// The structured source document (`{"kind": ...}`), inline.
        #[arg(long, conflicts_with = "source_file")]
        source_json: Option<String>,
        /// The structured source document, read from a file (`-` for stdin).
        #[arg(long)]
        source_file: Option<PathBuf>,
        /// Install target id; repeatable. Defaults to the ChatSpeed directory.
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Idempotency key. Generated when omitted, so a retry never doubles an effect.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Uninstall a Skill that ChatSpeed installed and still owns.
    Uninstall {
        /// The Skill name as installed.
        name: String,
        /// Install target id; repeatable. Defaults to the ChatSpeed directory.
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

/// MCP capability commands: reads plus the durable mutations.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// List MCP servers with their desired, observed runtime and tools state.
    List,
    /// Check one MCP server's status with a fresh bounded runtime observation.
    Status {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
    },
    /// Install one MCP server from a strict descriptor. It is registered
    /// disabled: no process is started and no network is contacted by install.
    Install {
        /// The descriptor document (`{"name": ..., "type": ...}`), inline.
        #[arg(long, conflicts_with = "descriptor_file")]
        descriptor_json: Option<String>,
        /// The descriptor document, read from a file (`-` for stdin).
        #[arg(long)]
        descriptor_file: Option<PathBuf>,
        /// Enable and start the server as a second, separately recorded
        /// operation after the install completes.
        #[arg(long)]
        enable: bool,
        /// Idempotency key. Generated when omitted, so a retry never doubles an effect.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Uninstall a server: disable, confirm the runtime stopped, then delete.
    Uninstall {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Set the desired state to enabled and start the server.
    Enable {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Set the desired state to disabled and stop the server.
    Disable {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Stop and start one enabled server.
    Restart {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// List the tools a server published, without invoking any of them.
    Tools {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
    },
    /// Re-read one server's tool list. It lists tools; it never invokes one.
    Refresh {
        /// The MCP server name as registered in ChatSpeed.
        name: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

/// Friendly automation fields shared by `create`, `update` and `draft`.
///
/// These flags are the validated alternative to a raw `--spec-json`/`--spec-file`
/// document: they are translated into the canonical `snake_case`
/// `AutomationSpec` the control plane deserializes. The two inputs are mutually
/// exclusive, and a combination that the schedule cannot express (for example a
/// weekday window on a one-shot schedule) is rejected rather than silently
/// dropped.
#[derive(Debug, Clone, Args)]
pub struct AutomationFriendlyArgs {
    /// Automation title.
    #[arg(long)]
    pub title: Option<String>,
    /// Existing agent id the automation runs with.
    #[arg(long)]
    pub agent: Option<String>,
    /// Inline prompt text; `-` reads it from stdin.
    #[arg(long, conflicts_with = "prompt_file")]
    pub prompt: Option<String>,
    /// Read the prompt text from a file (`-` for stdin).
    #[arg(long)]
    pub prompt_file: Option<PathBuf>,
    /// Run every interval, e.g. `5m` or `1h` (minimum 5 minutes).
    #[arg(long)]
    pub every: Option<String>,
    /// Run at a local `HH:MM` time; repeat the flag for several times a day.
    #[arg(long = "daily")]
    pub daily: Vec<String>,
    /// Run once at an RFC3339 timestamp or a local `YYYY-MM-DD HH:MM[:SS]`.
    #[arg(long)]
    pub once: Option<String>,
    /// Allowed weekdays as `1,2,3,4,5` (1 = Monday .. 7 = Sunday).
    #[arg(long)]
    pub weekdays: Option<String>,
    /// First effective date as `YYYY-MM-DD`.
    #[arg(long)]
    pub start_date: Option<String>,
    /// Last effective date as `YYYY-MM-DD`.
    #[arg(long)]
    pub end_date: Option<String>,
    /// Interval anchor time as `HH:MM[:SS]` (interval schedules only).
    #[arg(long)]
    pub anchor_time: Option<String>,
    /// Authorized directory path; repeat for several. Replaces the stored set.
    #[arg(long = "allowed-path")]
    pub allowed_paths: Vec<String>,
    /// Clear the stored allowed paths (existing automations only).
    #[arg(long)]
    pub clear_allowed_paths: bool,
    /// Enable the automation.
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the automation.
    #[arg(long)]
    pub disabled: bool,
    /// Keep one workflow context across scheduled runs.
    #[arg(long, conflicts_with = "no_continuous_context")]
    pub continuous_context: bool,
    /// Start a fresh workflow context for every scheduled run.
    #[arg(long)]
    pub no_continuous_context: bool,
    /// Ask the final review sub-agent to audit every run.
    #[arg(long, conflicts_with = "no_self_review")]
    pub self_review: bool,
    /// Skip the final review sub-agent.
    #[arg(long)]
    pub no_self_review: bool,
}

/// Local workflow-automation commands (Phase 3D).
#[derive(Debug, Subcommand)]
pub enum AutomationCommand {
    /// List automations with their schedule, enabled state and revision.
    List,
    /// Show one automation by id.
    Get {
        /// Automation id.
        automation_id: String,
    },
    /// List an automation's projected run lifecycle.
    Runs {
        /// Automation id.
        automation_id: String,
    },
    /// Produce a side-effect-free plan from a raw spec, a constrained intent or
    /// friendly field flags. It never mutates and never grants new permissions.
    Draft {
        /// Friendly field flags (alternative to `--spec-json`/`--spec-file`).
        #[command(flatten)]
        friendly: AutomationFriendlyArgs,
        /// The structured spec document, inline.
        #[arg(
            long,
            conflicts_with = "spec_file",
            conflicts_with_all = FRIENDLY_ARG_IDS
        )]
        spec_json: Option<String>,
        /// The structured spec document, read from a file ("-" for stdin).
        #[arg(long, conflicts_with_all = FRIENDLY_ARG_IDS)]
        spec_file: Option<PathBuf>,
        /// A constrained natural-language intent (alternative to a spec).
        #[arg(long, conflicts_with_all = ["spec_json", "spec_file"])]
        intent: Option<String>,
        /// Existing automation id to draft an update against.
        #[arg(long)]
        automation_id: Option<String>,
    },
    /// Apply a previously drafted plan after verifying its hash and base revision.
    Apply {
        /// The plan document (a `draft` output), inline.
        #[arg(long, conflicts_with = "plan_file")]
        plan_json: Option<String>,
        /// The plan document, read from a file ("-" for stdin).
        #[arg(long)]
        plan_file: Option<PathBuf>,
        /// The exact plan hash the caller reviewed. Defaults to the plan's own hash.
        #[arg(long)]
        expected_plan_hash: Option<String>,
        /// Acknowledge that the plan changes shell/path/agent permissions.
        #[arg(long)]
        acknowledge_permission_changes: bool,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Create a new automation from a raw spec or friendly field flags. The id is
    /// backend-minted.
    Create {
        /// Friendly field flags (alternative to `--spec-json`/`--spec-file`).
        #[command(flatten)]
        friendly: AutomationFriendlyArgs,
        /// The structured spec document, inline.
        #[arg(
            long,
            conflicts_with = "spec_file",
            conflicts_with_all = FRIENDLY_ARG_IDS
        )]
        spec_json: Option<String>,
        /// The structured spec document, read from a file ("-" for stdin).
        #[arg(long, conflicts_with_all = FRIENDLY_ARG_IDS)]
        spec_file: Option<PathBuf>,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Update an existing automation by compare-and-set on its revision.
    ///
    /// With friendly fields the current revision is read first and used unless
    /// `--expected-revision` pins the same value; with a raw spec the revision
    /// stays required. A same-kind update may restate only schedule parameters
    /// (`--weekdays`/`--start-date`/`--end-date`/`--anchor-time`), which keeps the
    /// stored kind and its other parameters.
    Update {
        /// Automation id.
        automation_id: String,
        /// Friendly field flags (alternative to `--spec-json`/`--spec-file`).
        #[command(flatten)]
        friendly: AutomationFriendlyArgs,
        /// The structured spec document, inline.
        #[arg(
            long,
            conflicts_with = "spec_file",
            conflicts_with_all = FRIENDLY_ARG_IDS
        )]
        spec_json: Option<String>,
        /// The structured spec document, read from a file ("-" for stdin).
        #[arg(long, conflicts_with_all = FRIENDLY_ARG_IDS)]
        spec_file: Option<PathBuf>,
        /// The revision the caller read; required with a raw spec and must match
        /// the current revision when friendly flags are used.
        #[arg(long)]
        expected_revision: Option<i64>,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Enable an automation so the scheduler may dispatch it.
    Enable {
        /// Automation id.
        automation_id: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Disable an automation so no further scheduled run is dispatched.
    Disable {
        /// Automation id.
        automation_id: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Trigger a manual run now. An accepted start is never reported as a
    /// completion; observe `runs` for the durable terminal state.
    Run {
        /// Automation id.
        automation_id: String,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Permanently delete an automation and its associated workflow tree.
    /// Destructive: requires `--confirm` and refuses while a run is active.
    Delete {
        /// Automation id.
        automation_id: String,
        /// Confirm the destructive delete. Without it the server refuses.
        #[arg(long)]
        confirm: bool,
        /// Idempotency key. Generated when omitted.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List top-level, non-disabled agents.
    List,
    /// Show one agent by stable ID.
    Get { agent_id: String },
}

#[derive(Debug, Subcommand)]
pub enum WorkflowCommand {
    /// List workflows.
    List,
    /// Create a workflow session (does not start it).
    Create {
        /// Stable agent ID to run the workflow with.
        #[arg(long)]
        agent: String,
        /// Initial prompt text.
        #[arg(long, conflicts_with = "prompt_file")]
        prompt: Option<String>,
        /// Read the initial prompt from a file ("-": stdin).
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        /// Authorized directory paths for the workflow.
        #[arg(long = "allowed-path")]
        allowed_paths: Vec<String>,
        /// Model override for the act (execution) phase, as "group@model"
        /// (e.g. cs@free:ds-v4-flash; shortcut for agent-config models.act).
        #[arg(long, conflicts_with = "agent_config")]
        model: Option<String>,
        /// Raw inherited agent config JSON (use --help to list supported keys).
        #[arg(long, long_help = AGENT_CONFIG_LONG_HELP)]
        agent_config: Option<String>,
        /// Enable final audit for the workflow.
        #[arg(long)]
        final_audit: bool,
    },
    /// Start (or resume) a created workflow session.
    Start {
        session_id: String,
        #[arg(long, conflicts_with = "prompt_file")]
        prompt: Option<String>,
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        /// Start in planning mode.
        #[arg(long)]
        plan: bool,
        /// Follow live events after starting.
        #[arg(long)]
        follow: bool,
    },
    /// Create and then start a workflow in one step.
    Run {
        #[arg(long)]
        agent: String,
        #[arg(long, conflicts_with = "prompt_file")]
        prompt: Option<String>,
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        #[arg(long = "allowed-path")]
        allowed_paths: Vec<String>,
        /// Model override for the act (execution) phase, as "group@model"
        /// (e.g. cs@free:ds-v4-flash; shortcut for agent-config models.act).
        #[arg(long, conflicts_with = "agent_config")]
        model: Option<String>,
        /// Raw inherited agent config JSON (use --help to list supported keys).
        #[arg(long, long_help = AGENT_CONFIG_LONG_HELP)]
        agent_config: Option<String>,
        /// Enable final audit for the workflow.
        #[arg(long)]
        final_audit: bool,
        /// Start in planning mode.
        #[arg(long)]
        plan: bool,
        #[arg(long)]
        follow: bool,
    },
    /// Show the authoritative workflow snapshot.
    Get { session_id: String },
    /// List durable events; with --follow, also stream live events.
    Events {
        session_id: String,
        /// Only return durable events after this durable event ID.
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        follow: bool,
    },
    /// Submit a raw typed signal (JSON) to a workflow session.
    Signal {
        session_id: String,
        /// Signal JSON, e.g. {"type":"user_message","content":"hi"}
        #[arg(long, required_unless_present = "file", conflicts_with = "file")]
        json: Option<String>,
        /// Read the signal JSON from a file ("-": stdin).
        #[arg(long, required_unless_present = "json")]
        file: Option<PathBuf>,
    },
    /// Send a user message to a workflow session.
    Message {
        session_id: String,
        #[arg(long, required_unless_present = "file", conflicts_with = "file")]
        text: Option<String>,
        #[arg(long, required_unless_present = "text")]
        file: Option<PathBuf>,
    },
    /// Approve a pending tool call.
    Approve {
        session_id: String,
        #[arg(long, required_unless_present = "all")]
        tool_call_id: Option<String>,
        /// Approve all pending tool calls.
        #[arg(long)]
        all: bool,
    },
    /// Reject a pending tool call.
    Reject {
        session_id: String,
        #[arg(long, required_unless_present = "all")]
        tool_call_id: Option<String>,
        /// Reject all pending tool calls.
        #[arg(long)]
        all: bool,
        /// Optional rejection message.
        #[arg(long)]
        message: Option<String>,
    },
    /// Ask a waiting workflow to continue.
    Continue { session_id: String },
    /// Stop a workflow session (active, waiting or retrying).
    Stop { session_id: String },
}

/// Resolves the initial prompt text from `--prompt` / `--prompt-file`.
pub fn resolve_prompt(
    prompt: &Option<String>,
    prompt_file: &Option<PathBuf>,
) -> Result<String, String> {
    resolve_prompt_with(prompt, prompt_file, &mut std::io::stdin().lock())
}

/// Resolves the initial prompt, reading the `-` convention from the given stdin
/// reader.
///
/// Split out from [`resolve_prompt`] so the stdin branch is verifiable without a
/// real terminal or runtime.
fn resolve_prompt_with(
    prompt: &Option<String>,
    prompt_file: &Option<PathBuf>,
    stdin: &mut impl std::io::Read,
) -> Result<String, String> {
    if let Some(text) = prompt {
        return Ok(text.clone());
    }
    if let Some(path) = prompt_file {
        if path == &PathBuf::from("-") {
            let mut buffer = String::new();
            stdin
                .read_to_string(&mut buffer)
                .map_err(|e| format!("Failed to read prompt from stdin: {}", e))?;
            return Ok(buffer);
        }
        return std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read prompt file {}: {}", path.display(), e));
    }
    Ok(String::new())
}

/// Validates mutually required flag groups that Clap cannot express directly.
pub fn validate_signal_source(
    json: &Option<String>,
    file: &Option<PathBuf>,
) -> Result<String, String> {
    if let Some(json) = json {
        return Ok(json.clone());
    }
    if let Some(path) = file {
        if path == &PathBuf::from("-") {
            use std::io::Read;
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|e| format!("Failed to read signal from stdin: {}", e))?;
            return Ok(buffer);
        }
        return std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read signal file {}: {}", path.display(), e));
    }
    Err("Either --json or --file is required".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The raw `--spec-json` fixture documented in `docs/cscli.md`, kept in one
    /// place so the parser example and the spec-completeness check cannot drift.
    const DOCUMENTED_SPEC_FIXTURE: &str = "{\"title\":\"Raw\",\"prompt\":\"p\",\"prompt_file_path\":null,\"agent_id\":\"agent-1\",\"agent_config\":null,\"allowed_paths\":[],\"shell_config\":null,\"schedule_kind\":\"daily\",\"schedule_config\":{\"times\":[\"09:00\"]},\"continuous_context\":false,\"self_review\":false,\"enabled\":true}";

    fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(argv)
    }

    /// The whole command tree must be internally consistent: duplicate or
    /// conflicting definitions are a usage-contract regression.
    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    /// Bare `cscli doctor` keeps its previous behaviour; the capabilities report
    /// is strictly additive.
    #[test]
    fn doctor_is_optional_subcommand_additive() {
        let bare = parse(&["cscli", "doctor"]).expect("bare doctor");
        match bare.command {
            Command::Doctor { command } => assert!(command.is_none()),
            _ => panic!("expected the doctor command"),
        }

        let capabilities =
            parse(&["cscli", "doctor", "capabilities"]).expect("doctor capabilities");
        match capabilities.command {
            Command::Doctor {
                command: Some(DoctorCommand::Capabilities),
            } => {}
            _ => panic!("expected doctor capabilities"),
        }

        // The reconcile subcommand carries an optional idempotency key and stays
        // additive; bare `cscli doctor` is untouched.
        let reconcile = parse(&["cscli", "doctor", "reconcile"]).expect("doctor reconcile");
        match reconcile.command {
            Command::Doctor {
                command: Some(DoctorCommand::Reconcile { idempotency_key }),
            } => assert!(idempotency_key.is_none()),
            _ => panic!("expected doctor reconcile"),
        }
        let keyed = parse(&["cscli", "doctor", "reconcile", "--idempotency-key", "k-1"])
            .expect("doctor reconcile with key");
        match keyed.command {
            Command::Doctor {
                command: Some(DoctorCommand::Reconcile { idempotency_key }),
            } => assert_eq!(idempotency_key.as_deref(), Some("k-1")),
            _ => panic!("expected doctor reconcile with key"),
        }
    }

    #[test]
    fn skill_and_mcp_read_commands_parse() {
        match parse(&["cscli", "skill", "targets"])
            .expect("skill targets")
            .command
        {
            Command::Skill {
                command: SkillCommand::Targets,
            } => {}
            _ => panic!("expected skill targets"),
        }
        match parse(&["cscli", "skill", "list"])
            .expect("skill list")
            .command
        {
            Command::Skill {
                command: SkillCommand::List,
            } => {}
            _ => panic!("expected skill list"),
        }
        match parse(&["cscli", "mcp", "list"]).expect("mcp list").command {
            Command::Mcp {
                command: McpCommand::List,
            } => {}
            _ => panic!("expected mcp list"),
        }
        match parse(&["cscli", "mcp", "status", "weather"])
            .expect("mcp status")
            .command
        {
            Command::Mcp {
                command: McpCommand::Status { name },
            } => assert_eq!(name, "weather"),
            _ => panic!("expected mcp status"),
        }
    }

    #[test]
    fn an_unknown_capability_subcommand_is_a_usage_error() {
        // `skill check/install/uninstall` are the Phase 3 mutations; a name
        // that does not exist stays a parse error.
        assert!(parse(&["cscli", "skill", "verify"]).is_err());
        assert!(parse(&["cscli", "mcp", "call"]).is_err());
    }

    #[test]
    fn skill_mutations_parse_a_source_and_repeatable_targets() {
        match parse(&[
            "cscli",
            "skill",
            "install",
            "--source-json",
            "{\"kind\":\"local_directory\",\"path\":\"/tmp/demo\"}",
            "--target",
            "chatspeed",
            "--target",
            "agents",
            "--idempotency-key",
            "k1",
        ])
        .expect("skill install")
        .command
        {
            Command::Skill {
                command:
                    SkillCommand::Install {
                        source_json,
                        targets,
                        idempotency_key,
                        ..
                    },
            } => {
                assert!(source_json.is_some());
                assert_eq!(targets, vec!["chatspeed".to_string(), "agents".to_string()]);
                assert_eq!(idempotency_key.as_deref(), Some("k1"));
            }
            _ => panic!("expected skill install"),
        }

        // An inline document and a file are mutually exclusive.
        assert!(parse(&[
            "cscli",
            "skill",
            "check",
            "--source-json",
            "{}",
            "--source-file",
            "source.json",
        ])
        .is_err());

        match parse(&["cscli", "skill", "uninstall", "demo"])
            .expect("skill uninstall")
            .command
        {
            Command::Skill {
                command:
                    SkillCommand::Uninstall {
                        name,
                        targets,
                        idempotency_key,
                    },
            } => {
                assert_eq!(name, "demo");
                // No target means the ChatSpeed directory only.
                assert!(targets.is_empty());
                assert!(idempotency_key.is_none());
            }
            _ => panic!("expected skill uninstall"),
        }
    }

    /// Friendly automation flags parse into every subcommand that accepts them,
    /// and the raw document path stays available beside them.
    #[test]
    fn automation_friendly_flags_parse_for_create_update_and_draft() {
        let create = parse(&[
            "cscli",
            "automation",
            "create",
            "--title",
            "Nightly",
            "--agent",
            "agent-1",
            "--prompt",
            "hello",
            "--every",
            "1h",
            "--allowed-path",
            "/tmp/a",
            "--allowed-path",
            "/tmp/b",
            "--enabled",
        ])
        .expect("friendly create");
        match create.command {
            Command::Automation { command } => match *command {
                AutomationCommand::Create {
                    friendly,
                    spec_json,
                    ..
                } => {
                    assert_eq!(friendly.title.as_deref(), Some("Nightly"));
                    assert_eq!(friendly.agent.as_deref(), Some("agent-1"));
                    assert_eq!(friendly.every.as_deref(), Some("1h"));
                    assert_eq!(friendly.allowed_paths, vec!["/tmp/a", "/tmp/b"]);
                    assert!(friendly.enabled);
                    assert!(spec_json.is_none());
                }
                _ => panic!("expected automation create"),
            },
            _ => panic!("expected an automation command"),
        }

        // A raw spec and friendly flags are mutually exclusive at the CLI edge.
        assert!(parse(&[
            "cscli",
            "automation",
            "create",
            "--title",
            "T",
            "--spec-json",
            "{}",
        ])
        .is_err());

        let draft = parse(&[
            "cscli",
            "automation",
            "draft",
            "--daily",
            "09:00",
            "--daily",
            "18:00",
            "--weekdays",
            "1,2,3,4,5",
        ])
        .expect("friendly draft");
        match draft.command {
            Command::Automation { command } => match *command {
                AutomationCommand::Draft { friendly, .. } => {
                    assert_eq!(friendly.daily, vec!["09:00", "18:00"]);
                }
                _ => panic!("expected automation draft"),
            },
            _ => panic!("expected an automation command"),
        }

        let update = parse(&["cscli", "automation", "update", "auto-1", "--self-review"])
            .expect("friendly update");
        match update.command {
            Command::Automation { command } => match *command {
                AutomationCommand::Update {
                    automation_id,
                    friendly,
                    expected_revision,
                    ..
                } => {
                    assert_eq!(automation_id, "auto-1");
                    assert!(friendly.self_review);
                    // The revision is optional for the friendly path and validated later.
                    assert!(expected_revision.is_none());
                }
                _ => panic!("expected automation update"),
            },
            _ => panic!("expected an automation command"),
        }
        let pinned = parse(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--disabled",
            "--expected-revision",
            "7",
        ])
        .expect("friendly update with revision");
        match pinned.command {
            Command::Automation { command } => match *command {
                AutomationCommand::Update {
                    friendly,
                    expected_revision,
                    ..
                } => {
                    assert!(friendly.disabled);
                    assert_eq!(expected_revision, Some(7));
                }
                _ => panic!("expected automation update"),
            },
            _ => panic!("expected an automation command"),
        }
    }

    /// Every three-state pair and the prompt source are rejected by clap when
    /// both halves are supplied, so an intent can never be ambiguous.
    #[test]
    fn automation_friendly_flag_pairs_are_mutually_exclusive() {
        for flags in [
            vec!["--enabled", "--disabled"],
            vec!["--continuous-context", "--no-continuous-context"],
            vec!["--self-review", "--no-self-review"],
            vec!["--prompt", "x", "--prompt-file", "p.txt"],
            vec!["--spec-json", "{}", "--spec-file", "p.json"],
        ] {
            let mut argv = vec!["cscli", "automation", "create"];
            argv.extend(flags.iter().copied());
            assert!(
                parse(&argv).is_err(),
                "expected a conflict for {flags:?}"
            );
        }
    }

    /// A draft accepts exactly one of a raw spec, an intent or friendly flags for
    /// its structured inputs. Clap rejects a spec next to an intent; the friendly
    /// flags are rejected next to an intent when the request is shaped (see the
    /// automation command tests), so an ambiguous draft can never be sent.
    #[test]
    fn automation_draft_inputs_are_mutually_exclusive() {
        assert!(parse(&[
            "cscli",
            "automation",
            "draft",
            "--intent",
            "run daily",
            "--spec-json",
            "{}",
        ])
        .is_err());
        assert!(parse(&["cscli", "automation", "draft", "--intent", "run daily"]).is_ok());
        assert!(parse(&["cscli", "automation", "draft", "--title", "T"]).is_ok());
    }

    /// The `-` prompt-file convention reads the prompt text from stdin, and the
    /// named-file and inline forms keep working beside it.
    #[test]
    fn resolve_prompt_reads_stdin_for_the_dash_convention() {
        let mut stdin: &[u8] = b"from stdin\n";
        assert_eq!(
            resolve_prompt_with(&None, &Some(PathBuf::from("-")), &mut stdin).expect("stdin"),
            "from stdin\n"
        );

        let inline = Some("inline text".to_string());
        assert_eq!(
            resolve_prompt_with(&inline, &None, &mut std::io::empty()).expect("inline"),
            "inline text"
        );
        assert_eq!(
            resolve_prompt_with(&None, &None, &mut std::io::empty()).expect("none"),
            ""
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prompt.txt");
        std::fs::write(&path, "from file").expect("write");
        assert_eq!(
            resolve_prompt_with(&None, &Some(path), &mut std::io::empty()).expect("file"),
            "from file"
        );
    }

    /// Every automation command shape documented in `docs/cscli.md` must parse,
    /// so the developer reference cannot drift from the CLI contract.
    #[test]
    fn documented_automation_examples_parse() {
        let cases: &[&[&str]] = &[
            &["cscli", "automation", "list"],
            &["cscli", "automation", "get", "auto-1"],
            &["cscli", "automation", "runs", "auto-1"],
            &[
                "cscli",
                "automation",
                "create",
                "--title",
                "Nightly",
                "--agent",
                "agent-1",
                "--prompt",
                "Summarize new issues",
                "--every",
                "1h",
                "--allowed-path",
                "/home/me/project",
            ],
            &[
                "cscli",
                "automation",
                "create",
                "--title",
                "Morning",
                "--agent",
                "agent-1",
                "--prompt-file",
                "./prompt.txt",
                "--daily",
                "09:00",
                "--daily",
                "18:00",
                "--weekdays",
                "1,2,3,4,5",
                "--enabled",
            ],
            &[
                "cscli",
                "automation",
                "create",
                "--spec-json",
                DOCUMENTED_SPEC_FIXTURE,
            ],
            &[
                "cscli",
                "automation",
                "draft",
                "--title",
                "Nightly",
                "--agent",
                "agent-1",
                "--prompt",
                "hi",
                "--every",
                "1h",
            ],
            &[
                "cscli",
                "automation",
                "draft",
                "--intent",
                "run every day at 09:00",
            ],
            &[
                "cscli",
                "automation",
                "draft",
                "--automation-id",
                "auto-1",
                "--title",
                "Renamed",
            ],
            &[
                "cscli",
                "automation",
                "apply",
                "--plan-file",
                "plan.json",
                "--acknowledge-permission-changes",
            ],
            &["cscli", "automation", "update", "auto-1", "--title", "Renamed"],
            &[
                "cscli",
                "automation",
                "update",
                "auto-1",
                "--spec-file",
                "spec.json",
                "--expected-revision",
                "7",
            ],
            &["cscli", "automation", "enable", "auto-1"],
            &["cscli", "automation", "disable", "auto-1"],
            &["cscli", "automation", "run", "auto-1"],
            &["cscli", "automation", "delete", "auto-1", "--confirm"],
        ];
        for argv in cases {
            parse(argv).unwrap_or_else(|error| {
                panic!("documented example {argv:?} failed to parse: {error}")
            });
        }
    }

    /// The raw `--spec-json` fixture in `docs/cscli.md` must be a complete
    /// `AutomationSpec` document: every canonical field is present and no key the
    /// backend would reject as unknown is carried.
    #[test]
    fn documented_raw_spec_fixture_is_a_complete_automation_spec() {
        let spec: serde_json::Value =
            serde_json::from_str(DOCUMENTED_SPEC_FIXTURE).expect("valid JSON");
        let object = spec.as_object().expect("a JSON object");
        for field in [
            "title",
            "prompt",
            "prompt_file_path",
            "agent_id",
            "agent_config",
            "allowed_paths",
            "shell_config",
            "schedule_kind",
            "schedule_config",
            "continuous_context",
            "self_review",
            "enabled",
        ] {
            assert!(
                object.contains_key(field),
                "missing AutomationSpec field {field}"
            );
        }
        assert_eq!(object.len(), 12, "the fixture must not carry unknown fields");
        assert_eq!(object["schedule_kind"], "daily");
    }
}
