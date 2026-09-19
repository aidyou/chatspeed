//! Clap command tree for the `cs` CLI.

use clap::{Parser, Subcommand, ValueEnum};
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
    name = "cs",
    version,
    about = "ChatSpeed workflow CLI",
    long_about = "ChatSpeed workflow CLI.\n\n\
        `cs` talks to a running ChatSpeed desktop app over its local loopback\n\
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
    /// Capture, inspect and replay workflow run artifacts.
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
}

/// Additive `cs doctor` sub-capabilities.
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

#[derive(Debug, Subcommand)]
pub enum ExperimentCommand {
    /// Submit one budgeted, single-attempt experiment run through the
    /// authenticated control plane (the CLI never runs an executor itself).
    Run {
        /// Stable agent ID to run the experiment with.
        #[arg(long)]
        agent: String,
        /// Path to the strict `experiment_run_spec.v1` JSON file.
        #[arg(long)]
        spec: PathBuf,
        /// Prompt text (mutually exclusive with --prompt-file).
        #[arg(long, conflicts_with = "prompt_file")]
        prompt: Option<String>,
        /// Read the prompt from a file ("-": stdin).
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        /// Follow live events after the run starts.
        #[arg(long)]
        follow: bool,
        /// Wait for a durable terminal state, then capture a 2A-compatible
        /// artifact into this directory (must not already exist).
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
    },
    /// Capture an existing workflow session into an artifact directory
    /// (read-only: never creates, starts, signals or stops a workflow).
    Capture {
        /// Workflow session ID to capture.
        session_id: String,
        /// Target directory for the artifact bundle (must not already exist).
        #[arg(long)]
        artifact_dir: PathBuf,
    },
    /// Verify an artifact directory offline (no main process, network or DB).
    Inspect {
        /// Artifact directory to verify.
        artifact_dir: PathBuf,
    },
    /// Project an artifact's timeline, status and usage offline.
    Replay {
        /// Artifact directory to replay.
        artifact_dir: PathBuf,
    },
    /// Evaluate a verified artifact offline with the deterministic 2D
    /// evaluator and publish an independent evaluation sidecar (no main
    /// process, network, DB or LLM; never modifies the source artifact).
    /// Note: the sidecar target flag is `--evaluation-dir` because the global
    /// `--output` flag already selects the output format.
    Evaluate {
        /// Artifact directory to evaluate (must pass the 2A verifier).
        artifact_dir: PathBuf,
        /// Target directory for the evaluation sidecar (must not already
        /// exist and must not overlap the artifact directory).
        #[arg(long)]
        evaluation_dir: PathBuf,
    },
    /// Run fixed benchmark tasks through the existing budgeted experiment
    /// control plane (single runtime owner; the CLI never executes locally).
    Benchmark {
        #[command(subcommand)]
        command: BenchmarkCommand,
    },
    /// Phase 2F Stage 0 campaign orchestration over one immutable plan.
    /// `create`/`run`/`close` talk to the control plane; `inspect` is fully
    /// offline and never loads discovery.
    Campaign {
        #[command(subcommand)]
        command: CampaignCommand,
    },
    /// Phase 2I promotion: apply a verified code candidate to a
    /// server-registered experiment target, run the paired canary gate and
    /// audit the result.
    ///
    /// `run`/`status`/`reconcile`/`audit` talk to the control plane; `inspect`
    /// is fully offline and never loads discovery.
    Promotion {
        #[command(subcommand)]
        command: PromotionCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CampaignCommand {
    /// Validate the immutable plan, create the shared backend campaign budget
    /// scope and publish the campaign/candidate sidecars under `--out`.
    Create {
        /// Path to the strict `campaign_plan.v1` JSON file.
        #[arg(long)]
        plan: PathBuf,
        /// Campaign output root; the sidecars are written under
        /// `<out>/campaign` and evidence under `<out>/evidence` (must not
        /// already contain a campaign).
        #[arg(long)]
        out: PathBuf,
    },
    /// Execute one declared candidate: submit exactly one backend-owned run
    /// under the campaign, wait for the durable terminal state, capture the
    /// artifact, evaluate and verify it offline, then independently re-verify
    /// and consume the verdict. Each declared arm may be run once.
    Run {
        /// Campaign output root created by `campaign create`.
        #[arg(long)]
        out: PathBuf,
        /// Candidate key declared by the campaign plan.
        #[arg(long)]
        candidate: String,
    },
    /// Re-verify the campaign sidecars offline and print the recomputed
    /// canonical facts (no discovery, network, DB or LLM).
    Inspect {
        /// Campaign output root created by `campaign create`.
        #[arg(long)]
        out: PathBuf,
    },
    /// Persist one durable campaign schedule: the frozen plan, its fixture
    /// refs, the server-registered execution profile and the bundle refs. The
    /// ordered jobs are created in one backend transaction (Phase 2G+2H).
    Schedule {
        /// Path to the strict `campaign_plan.v1` JSON file.
        #[arg(long)]
        plan: PathBuf,
        /// Server-registered execution profile reference of the target domain.
        #[arg(long)]
        profile: String,
        /// Allowlisted bundle reference the profile permits. Repeat for several.
        #[arg(long = "bundle-ref")]
        bundle_ref: Vec<String>,
    },
    /// List the durable jobs of a campaign, in candidate order.
    Jobs {
        /// Durable campaign id (backend-minted, `camp-...`).
        #[arg(long = "campaign-id")]
        campaign_id: String,
    },
    /// Read one durable job by its backend-minted id.
    Job {
        /// Durable job id (backend-minted, `job-...`).
        #[arg(long = "job-id")]
        job_id: String,
    },
    /// Cancel a campaign's pre-dispatch work and stop admitting new work.
    /// Already-dispatched jobs are reported, never cancelled.
    Cancel {
        /// Durable campaign id (backend-minted, `camp-...`).
        #[arg(long = "campaign-id")]
        campaign_id: String,
        /// Optional human-readable cancel reason.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Re-read the durable state plus the workflow authority and classify every
    /// non-terminal job. Evidence-only: it never requeues and never runs work.
    Reconcile {
        /// Durable campaign id (backend-minted, `camp-...`).
        #[arg(long = "campaign-id")]
        campaign_id: String,
    },
    /// Close the campaign budget scope so no further run or reservation is
    /// admitted, then publish the campaign summary sidecar.
    Close {
        /// Campaign output root created by `campaign create`.
        #[arg(long)]
        out: PathBuf,
        /// Optional human-readable close reason recorded on the scope.
        #[arg(long)]
        reason: Option<String>,
    },
}

/// Phase 2I promotion operations.
///
/// `run` and `audit` talk to the control plane; `inspect` is fully offline and
/// never loads discovery. The CLI is an evidence/HTTP adapter only: it never
/// opens the database, starts a runtime or touches Git.
#[derive(Debug, Subcommand)]
pub enum PromotionCommand {
    /// Submit one strict `promotion_request.v1` document, wait for the
    /// backend's terminal state and export the audit bundle under `--out`.
    Run {
        /// Path to the strict `promotion_request.v1` JSON file.
        #[arg(long)]
        evidence: PathBuf,
        /// Audit output root; the bundle is written under
        /// `<out>/promotion-audit` atomically.
        #[arg(long)]
        out: PathBuf,
    },
    /// Read the promotion status projection.
    Status {
        /// Backend-minted promotion id (`promo-...`).
        #[arg(long = "promotion-id")]
        promotion_id: String,
    },
    /// Evidence-only reconciliation: what the durable intents and recorded
    /// state imply, plus the ordered journal. It performs no effect.
    Reconcile {
        /// Backend-minted promotion id (`promo-...`).
        #[arg(long = "promotion-id")]
        promotion_id: String,
    },
    /// Export the offline-verifiable audit bundle of one promotion.
    Audit {
        /// Backend-minted promotion id (`promo-...`).
        #[arg(long = "promotion-id")]
        promotion_id: String,
        /// Audit output root.
        #[arg(long)]
        out: PathBuf,
    },
    /// Re-verify the exported audit bundle fully offline (no discovery,
    /// network, DB or Git) and print the recomputed canonical facts.
    Inspect {
        /// Audit output root created by `promotion run`/`promotion audit`.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum BenchmarkCommand {    /// Resolve a checked-in `chatspeed-smoke@1` fixture task and submit one
    /// budgeted, single-attempt experiment run (2C endpoint + admission).
    /// With --artifact-dir, waits for a durable terminal state and captures
    /// a 2A-compatible artifact.
    Run {
        /// Benchmark suite id (only `chatspeed-smoke` is registered).
        #[arg(long)]
        suite: String,
        /// Task id declared in the checked-in suite manifest.
        #[arg(long)]
        task: String,
        /// Stable agent ID to run the task with.
        #[arg(long)]
        agent: String,
        /// Optional act-phase model override as "group@model"
        /// (e.g. cs@free:ds-v4-flash); forwarded as the 2C spec workflow
        /// override. Without it the agent's default model is used.
        #[arg(long)]
        model: Option<String>,
        /// Wait for a durable terminal state, then capture a 2A-compatible
        /// artifact into this directory (must not already exist).
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
    },
    /// Verify a captured artifact against the fixed fixture task offline and
    /// publish an independent, digest-bound verdict sidecar (no main process,
    /// network, DB or LLM; model self-reports are never score inputs).
    Verify {
        /// Benchmark suite id (only `chatspeed-smoke` is registered).
        #[arg(long)]
        suite: String,
        /// Task id declared in the checked-in suite manifest.
        #[arg(long)]
        task: String,
        /// Artifact directory to verify (must pass the 2A verifier).
        artifact_dir: PathBuf,
        /// Target directory for the verdict sidecar (must not already exist
        /// and must not overlap the artifact directory).
        #[arg(long)]
        verdict_dir: PathBuf,
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
    if let Some(text) = prompt {
        return Ok(text.clone());
    }
    if let Some(path) = prompt_file {
        if path == &PathBuf::from("-") {
            use std::io::Read;
            let mut buffer = String::new();
            std::io::stdin()
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

    fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(argv)
    }

    /// The whole command tree must be internally consistent: duplicate or
    /// conflicting definitions are a usage-contract regression.
    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    /// Bare `cs doctor` keeps its previous behaviour; the capabilities report
    /// is strictly additive.
    #[test]
    fn doctor_is_optional_subcommand_additive() {
        let bare = parse(&["cs", "doctor"]).expect("bare doctor");
        match bare.command {
            Command::Doctor { command } => assert!(command.is_none()),
            _ => panic!("expected the doctor command"),
        }

        let capabilities = parse(&["cs", "doctor", "capabilities"]).expect("doctor capabilities");
        match capabilities.command {
            Command::Doctor {
                command: Some(DoctorCommand::Capabilities),
            } => {}
            _ => panic!("expected doctor capabilities"),
        }

        // The reconcile subcommand carries an optional idempotency key and stays
        // additive; bare `cs doctor` is untouched.
        let reconcile = parse(&["cs", "doctor", "reconcile"]).expect("doctor reconcile");
        match reconcile.command {
            Command::Doctor {
                command: Some(DoctorCommand::Reconcile { idempotency_key }),
            } => assert!(idempotency_key.is_none()),
            _ => panic!("expected doctor reconcile"),
        }
        let keyed = parse(&[
            "cs",
            "doctor",
            "reconcile",
            "--idempotency-key",
            "k-1",
        ])
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
        match parse(&["cs", "skill", "targets"]).expect("skill targets").command {
            Command::Skill {
                command: SkillCommand::Targets,
            } => {}
            _ => panic!("expected skill targets"),
        }
        match parse(&["cs", "skill", "list"]).expect("skill list").command {
            Command::Skill {
                command: SkillCommand::List,
            } => {}
            _ => panic!("expected skill list"),
        }
        match parse(&["cs", "mcp", "list"]).expect("mcp list").command {
            Command::Mcp {
                command: McpCommand::List,
            } => {}
            _ => panic!("expected mcp list"),
        }
        match parse(&["cs", "mcp", "status", "weather"])
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
        assert!(parse(&["cs", "skill", "verify"]).is_err());
        assert!(parse(&["cs", "mcp", "call"]).is_err());
    }

    #[test]
    fn skill_mutations_parse_a_source_and_repeatable_targets() {
        match parse(&[
            "cs",
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
            "cs",
            "skill",
            "check",
            "--source-json",
            "{}",
            "--source-file",
            "source.json",
        ])
        .is_err());

        match parse(&["cs", "skill", "uninstall", "demo"])
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
}
