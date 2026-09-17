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
    Doctor,
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
    /// Capture, inspect and replay workflow run artifacts.
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
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

#[derive(Debug, Subcommand)]
pub enum BenchmarkCommand {
    /// Resolve a checked-in `chatspeed-smoke@1` fixture task and submit one
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
