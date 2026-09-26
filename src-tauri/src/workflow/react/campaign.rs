//! Phase 2F campaign, candidate and proposer-boundary contracts.
//!
//! Phase 2F adds an immutable, plan-driven campaign orchestration boundary on
//! top of the 2B budget ledger, the 2C run kernel and the 2D/2E offline
//! evaluation/verdict sidecars. This module defines the *contract* only; the
//! backend application service and the CLI are the two adapters.
//!
//! Design rules enforced here (AC-2/AC-3/INV-4/INV-6):
//!
//! - Every external document is strict, versioned, snake_case and rejects
//!   unknown fields. Unknown or forbidden fields fail closed *before* any
//!   effect with a stable machine code.
//! - A Stage 0 candidate may declare exactly one allowlisted declarative
//!   surface: a checked-in `agent_prompt_ref`/`prompt_hash` pair. The raw
//!   prompt text lives only in the checked-in, versioned catalog; it is never
//!   accepted from the caller and never stored in a candidate document.
//! - All identity that binds a run to its plan (campaign id, candidate scope,
//!   trial scope) is derived by the backend from a domain-separated canonical
//!   hash of the frozen plan. A caller can never mint or supply a scope id.
//! - Canonical hashes exclude volatile fields (timestamps), so the same plan
//!   re-parsed offline always yields the same candidate order and hashes.

use crate::budget::types::{BudgetEnvelope, BudgetVector, ResourceCaps, ScopeKind};
use crate::workflow::react::experiment::{
    ExperimentBudgetSpec, ExperimentRunSpecV1, ExperimentWorkflowOverride, EXPERIMENT_RUN_SPEC_V1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Fixed schema version literal for the stage 0 campaign plan document.
pub const CAMPAIGN_PLAN_V1: &str = "campaign_plan.v1";
/// Fixed schema version literal for a candidate manifest document.
pub const CANDIDATE_MANIFEST_V1: &str = "candidate_manifest.v1";
/// Fixed schema version literal for a campaign run intent document.
pub const CAMPAIGN_RUN_REQUEST_V1: &str = "campaign_run_request.v1";
/// Fixed schema version literal for the campaign consumption summary.
pub const CAMPAIGN_SUMMARY_V1: &str = "campaign_summary.v1";

/// The only Stage 0 campaign stage.
pub const STAGE_0_MANUAL: &str = "stage_0_manual";
/// The only Stage 0 proposer kind (2F ships no LLM proposer).
pub const PROPOSER_KIND_MANUAL: &str = "manual";
/// The only Stage 0 proposer version.
pub const PROPOSER_VERSION: &str = "1";

/// The single allowlisted Stage 0 declarative surface.
pub const CANDIDATE_SURFACE_AGENT_PROMPT_REF: &str = "agent_prompt_ref";
/// Reserved candidate key for the unmodified baseline arm.
pub const BASELINE_CANDIDATE_KEY: &str = "baseline";

/// Stage 0 fixes concurrency at one ordered run at a time.
pub const STAGE_0_CONCURRENCY: u32 = 1;
/// Maximum length of an operator-chosen campaign/candidate key.
pub const MAX_KEY_LEN: usize = 64;

/// Canonical hash domains. Each identity is domain-separated so a hash minted
/// for one purpose can never be replayed as another.
pub const PLAN_HASH_DOMAIN: &str = "cs-campaign:plan";
pub const CANDIDATE_HASH_DOMAIN: &str = "cs-campaign:candidate";
pub const CANDIDATE_SURFACE_HASH_DOMAIN: &str = "cs-campaign:candidate-surface";
pub const ENVELOPE_HASH_DOMAIN: &str = "cs-campaign:envelope";
pub const PROMPT_CATALOG_HASH_DOMAIN: &str = "cs-campaign:prompt-catalog";
pub const CANDIDATE_PROMPT_HASH_DOMAIN: &str = "cs-campaign:candidate-prompt";
pub const CAMPAIGN_ID_DOMAIN: &str = "cs-campaign:campaign-id";
pub const CANDIDATE_SCOPE_ID_DOMAIN: &str = "cs-campaign:candidate-scope-id";
pub const TRIAL_SCOPE_ID_DOMAIN: &str = "cs-campaign:trial-scope-id";

/// Hash domain of a checked-in benchmark task instruction. It must stay equal
/// to the 2E benchmark adapter's `INSTRUCTION_HASH_DOMAIN`, so the run intent
/// and the offline verifier bind the same prompt bytes (asserted by a test in
/// the CLI adapter).
pub const FIXTURE_INSTRUCTION_HASH_DOMAIN: &str = "cs-benchmark:instruction";

/// Stable machine codes for campaign contract rejections. They describe a
/// request rejected *before* any effect and are part of the CLI/HTTP contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignSpecErrorCode {
    /// The document `schema_version` is missing or unsupported.
    UnsupportedVersion,
    /// The campaign stage is not the Stage 0 literal.
    InvalidStage,
    /// The campaign key is empty or malformed.
    InvalidCampaignKey,
    /// The candidate key is empty or malformed.
    InvalidCandidateKey,
    /// The candidate key collides with another candidate.
    DuplicateCandidate,
    /// No candidate is declared, or the baseline is not first.
    InvalidCandidateOrder,
    /// The baseline arm carries a mutable surface or prompt reference.
    BaselineNotImmutable,
    /// The declared mutable surface is not the single allowlisted surface.
    InvalidSurface,
    /// A caller-supplied field is forbidden for this document.
    ForbiddenField,
    /// A field is not part of the strict document schema.
    UnknownField,
    /// The prompt ref is not allowlisted by the checked-in catalog.
    PromptRefNotAllowlisted,
    /// The prompt hash does not match the checked-in catalog entry.
    PromptHashMismatch,
    /// The checked-in prompt catalog itself is malformed.
    PromptCatalogInvalid,
    /// The frozen budget envelope is structurally invalid.
    InvalidBudget,
    /// Stage 0 requires concurrency 1.
    ConcurrencyNotOne,
    /// The proposer reference is not the Stage 0 manual proposer.
    UnsupportedProposer,
    /// The agent id is missing.
    InvalidAgentId,
    /// The fixture (suite/task/model) identity is missing or malformed.
    InvalidFixture,
    /// A candidate references a key that is not declared by the plan.
    UnknownCandidate,
    /// The supplied plan does not match the campaign's frozen plan hash.
    CampaignPlanMismatch,
    /// The campaign id is empty or is not a backend-minted campaign scope id.
    InvalidCampaignId,
    /// A candidate declared a prompt ref but no prompt hash (or the reverse).
    IncompletePromptRef,
    /// The declared fixture instruction does not match its digest.
    FixtureDigestMismatch,
}

impl CampaignSpecErrorCode {
    /// Stable snake_case machine code.
    pub fn as_str(&self) -> &'static str {
        match self {
            CampaignSpecErrorCode::UnsupportedVersion => "unsupported_campaign_schema",
            CampaignSpecErrorCode::InvalidStage => "invalid_campaign_stage",
            CampaignSpecErrorCode::InvalidCampaignKey => "invalid_campaign_key",
            CampaignSpecErrorCode::InvalidCandidateKey => "invalid_candidate_key",
            CampaignSpecErrorCode::DuplicateCandidate => "duplicate_candidate",
            CampaignSpecErrorCode::InvalidCandidateOrder => "invalid_candidate_order",
            CampaignSpecErrorCode::BaselineNotImmutable => "baseline_not_immutable",
            CampaignSpecErrorCode::InvalidSurface => "invalid_candidate_surface",
            CampaignSpecErrorCode::ForbiddenField => "forbidden_field",
            CampaignSpecErrorCode::UnknownField => "unknown_field",
            CampaignSpecErrorCode::PromptRefNotAllowlisted => "prompt_ref_not_allowlisted",
            CampaignSpecErrorCode::PromptHashMismatch => "prompt_hash_mismatch",
            CampaignSpecErrorCode::PromptCatalogInvalid => "prompt_catalog_invalid",
            CampaignSpecErrorCode::InvalidBudget => "invalid_budget",
            CampaignSpecErrorCode::ConcurrencyNotOne => "concurrency_not_one",
            CampaignSpecErrorCode::UnsupportedProposer => "unsupported_proposer",
            CampaignSpecErrorCode::InvalidAgentId => "invalid_agent_id",
            CampaignSpecErrorCode::InvalidFixture => "invalid_fixture",
            CampaignSpecErrorCode::UnknownCandidate => "unknown_candidate",
            CampaignSpecErrorCode::CampaignPlanMismatch => "campaign_plan_mismatch",
            CampaignSpecErrorCode::InvalidCampaignId => "invalid_campaign_id",
            CampaignSpecErrorCode::IncompletePromptRef => "incomplete_prompt_ref",
            CampaignSpecErrorCode::FixtureDigestMismatch => "fixture_digest_mismatch",
        }
    }
}

/// A campaign contract rejection carrying a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignSpecError {
    pub code: CampaignSpecErrorCode,
    pub message: String,
}

impl CampaignSpecError {
    pub fn new(code: CampaignSpecErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CampaignSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CampaignSpecError {}

// ---------------------------------------------------------------------------
// Canonical hashing (domain-separated, key-sorted, timestamp-free)
// ---------------------------------------------------------------------------

fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                canonical_json(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// Domain-separated SHA-256 over raw bytes.
pub fn domain_hash(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Canonical-JSON hash of a value within a domain. Key order never affects the
/// result, so the same logical document always hashes identically.
pub fn canonical_hash(domain: &str, value: &Value) -> String {
    let mut buffer = String::new();
    canonical_json(value, &mut buffer);
    domain_hash(domain, buffer.as_bytes())
}

/// Hash of the frozen budget envelope (the 2B admission contract).
pub fn envelope_hash(envelope: &BudgetEnvelope) -> String {
    let value = serde_json::to_value(envelope).unwrap_or(Value::Null);
    canonical_hash(ENVELOPE_HASH_DOMAIN, &value)
}

/// Hash of a checked-in candidate prompt body.
pub fn candidate_prompt_hash(system_prompt: &str) -> String {
    domain_hash(CANDIDATE_PROMPT_HASH_DOMAIN, system_prompt.as_bytes())
}

// ---------------------------------------------------------------------------
// Checked-in candidate prompt catalog
// ---------------------------------------------------------------------------

/// Strict checked-in catalog document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CandidatePromptCatalogV1 {
    pub schema_version: u32,
    pub catalog_id: String,
    pub catalog_version: u32,
    pub surfaces: Vec<CandidatePromptSurfaceV1>,
}

/// One allowlisted prompt surface. The prompt body is checked in, versioned
/// and auditable; it is never accepted from a caller and never written to a
/// candidate sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CandidatePromptSurfaceV1 {
    pub agent_prompt_ref: String,
    pub surface_version: u32,
    /// `candidate_prompt_hash(system_prompt)`; validated at load time.
    pub prompt_hash: String,
    pub system_prompt: String,
    #[serde(default)]
    pub planning_prompt: Option<String>,
}

/// A loaded, validated catalog with its canonical digest.
#[derive(Debug, Clone)]
pub struct CandidatePromptCatalog {
    catalog_id: String,
    catalog_version: u32,
    digest: String,
    surfaces: Vec<CandidatePromptSurfaceV1>,
}

impl CandidatePromptCatalog {
    /// Parses and validates a catalog document. The digest binds the whole
    /// catalog (id, version and every surface including its prompt body).
    pub fn from_json(text: &str) -> Result<Self, CampaignSpecError> {
        let document: CandidatePromptCatalogV1 = serde_json::from_str(text).map_err(|error| {
            CampaignSpecError::new(
                CampaignSpecErrorCode::PromptCatalogInvalid,
                format!("prompt catalog is not a valid strict v1 document: {error}"),
            )
        })?;
        if document.schema_version != 1 {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::PromptCatalogInvalid,
                format!(
                    "unsupported prompt catalog schema_version {}",
                    document.schema_version
                ),
            ));
        }
        if document.catalog_id.trim().is_empty() || document.surfaces.is_empty() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::PromptCatalogInvalid,
                "prompt catalog must declare an id and at least one surface",
            ));
        }
        for surface in &document.surfaces {
            if surface.agent_prompt_ref.trim().is_empty() {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::PromptCatalogInvalid,
                    "prompt catalog surface has an empty agent_prompt_ref",
                ));
            }
            if !is_valid_key(&surface.agent_prompt_ref) {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::PromptCatalogInvalid,
                    format!(
                        "prompt catalog ref '{}' is not a valid key",
                        surface.agent_prompt_ref
                    ),
                ));
            }
            if surface.system_prompt.trim().is_empty() {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::PromptCatalogInvalid,
                    format!(
                        "prompt catalog surface '{}' has an empty prompt body",
                        surface.agent_prompt_ref
                    ),
                ));
            }
            let expected = candidate_prompt_hash(&surface.system_prompt);
            if expected != surface.prompt_hash {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::PromptCatalogInvalid,
                    format!(
                        "prompt catalog surface '{}' has a stale prompt_hash",
                        surface.agent_prompt_ref
                    ),
                ));
            }
        }
        let mut refs: Vec<&str> = document
            .surfaces
            .iter()
            .map(|surface| surface.agent_prompt_ref.as_str())
            .collect();
        refs.sort_unstable();
        let unique = {
            let mut deduped = refs.clone();
            deduped.dedup();
            deduped
        };
        if unique.len() != refs.len() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::PromptCatalogInvalid,
                "prompt catalog declares a duplicate agent_prompt_ref",
            ));
        }
        let digest_value = serde_json::to_value(&document).unwrap_or(Value::Null);
        let digest = canonical_hash(PROMPT_CATALOG_HASH_DOMAIN, &digest_value);
        Ok(Self {
            catalog_id: document.catalog_id,
            catalog_version: document.catalog_version,
            digest,
            surfaces: document.surfaces,
        })
    }

    /// The single backend/CLI-visible catalog, compiled into the binary from a
    /// checked-in, versioned repository file. It is never loaded from a
    /// caller-writable runtime path.
    pub fn embedded() -> &'static CandidatePromptCatalog {
        static CATALOG: OnceLock<Result<CandidatePromptCatalog, CampaignSpecError>> =
            OnceLock::new();
        // A malformed checked-in catalog is a build-time contract violation;
        // fall back to an empty catalog that rejects every ref so the failure
        // is fail-closed at use time instead of panicking in production.
        CATALOG
            .get_or_init(|| CandidatePromptCatalog::from_json(CANDIDATE_CATALOG_JSON))
            .as_ref()
            .unwrap_or(&EMPTY_CATALOG)
    }

    /// The checked-in catalog JSON body (for offline audit/tests).
    pub fn raw_json() -> &'static str {
        CANDIDATE_CATALOG_JSON
    }

    pub fn catalog_id(&self) -> &str {
        &self.catalog_id
    }

    pub fn catalog_version(&self) -> u32 {
        self.catalog_version
    }

    /// Canonical digest of the whole catalog; bound into every resolved
    /// candidate so a later catalog edit cannot silently change a run.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn surfaces(&self) -> &[CandidatePromptSurfaceV1] {
        &self.surfaces
    }

    /// Resolves a ref/hash pair, failing closed on an unknown ref or a hash
    /// that does not match the checked-in body.
    pub fn resolve(
        &self,
        agent_prompt_ref: &str,
        prompt_hash: &str,
    ) -> Result<&CandidatePromptSurfaceV1, CampaignSpecError> {
        let surface = self
            .surfaces
            .iter()
            .find(|surface| surface.agent_prompt_ref == agent_prompt_ref)
            .ok_or_else(|| {
                CampaignSpecError::new(
                    CampaignSpecErrorCode::PromptRefNotAllowlisted,
                    format!("agent_prompt_ref '{agent_prompt_ref}' is not allowlisted"),
                )
            })?;
        if surface.prompt_hash != prompt_hash {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::PromptHashMismatch,
                format!("prompt_hash does not match the checked-in ref '{agent_prompt_ref}'"),
            ));
        }
        Ok(surface)
    }
}

/// The compiled-in checked-in catalog body.
const CANDIDATE_CATALOG_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../work/agent-cli-candidate-surfaces/catalog.json"
));

/// Fail-closed fallback used only if the checked-in catalog is malformed.
static EMPTY_CATALOG: CandidatePromptCatalog = CandidatePromptCatalog {
    catalog_id: String::new(),
    catalog_version: 0,
    digest: String::new(),
    surfaces: Vec::new(),
};

/// A resolved candidate prompt override. The prompt body exists only in
/// memory for the duration of one workflow start; the workflow snapshot keeps
/// the ref, hash and catalog digest only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCandidatePrompt {
    pub agent_prompt_ref: String,
    pub prompt_hash: String,
    pub surface_version: u32,
    pub catalog_digest: String,
    pub system_prompt: String,
    pub planning_prompt: Option<String>,
}

impl ResolvedCandidatePrompt {
    /// Canonical hash of the resolved declarative surface. This is what a
    /// candidate/campaign sidecar records as the change hash.
    pub fn surface_hash(&self) -> String {
        canonical_hash(
            CANDIDATE_SURFACE_HASH_DOMAIN,
            &serde_json::json!({
                "agent_prompt_ref": self.agent_prompt_ref,
                "prompt_hash": self.prompt_hash,
                "surface_version": self.surface_version,
                "catalog_digest": self.catalog_digest,
            }),
        )
    }
}

/// Resolves an allowlisted prompt ref/hash pair against the checked-in catalog.
pub fn resolve_candidate_prompt(
    agent_prompt_ref: &str,
    prompt_hash: &str,
) -> Result<ResolvedCandidatePrompt, CampaignSpecError> {
    let catalog = CandidatePromptCatalog::embedded();
    let surface = catalog.resolve(agent_prompt_ref, prompt_hash)?;
    Ok(ResolvedCandidatePrompt {
        agent_prompt_ref: surface.agent_prompt_ref.clone(),
        prompt_hash: surface.prompt_hash.clone(),
        surface_version: surface.surface_version,
        catalog_digest: catalog.digest().to_string(),
        system_prompt: surface.system_prompt.clone(),
        planning_prompt: surface.planning_prompt.clone(),
    })
}

// ---------------------------------------------------------------------------
// Strict documents
// ---------------------------------------------------------------------------

/// Why a candidate exists in the campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// The unmodified arm: exactly the frozen Agent defaults.
    Baseline,
    /// A single-variable arm: exactly one allowlisted declarative surface.
    Candidate,
}

impl CandidateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateKind::Baseline => "baseline",
            CandidateKind::Candidate => "candidate",
        }
    }
}

/// Default infra-failure stop threshold when the plan omits it.
const DEFAULT_MAX_INFRA_FAILURES: u32 = 1;

fn default_max_infra_failures() -> u32 {
    DEFAULT_MAX_INFRA_FAILURES
}

fn default_true() -> bool {
    true
}

/// Explicit stop conditions frozen into the campaign plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignStopConditions {
    /// Number of recorded infra failures that stops the campaign.
    #[serde(default = "default_max_infra_failures")]
    pub max_infra_failures: u32,
    /// Whether a `fail` verdict stops the campaign before later candidates.
    #[serde(default = "default_true")]
    pub stop_on_verdict_failure: bool,
}

impl Default for CampaignStopConditions {
    fn default() -> Self {
        Self {
            max_infra_failures: DEFAULT_MAX_INFRA_FAILURES,
            stop_on_verdict_failure: true,
        }
    }
}

/// One declared candidate arm. A non-baseline arm may declare exactly one
/// allowlisted declarative surface and nothing else.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignCandidateSpec {
    pub candidate_key: String,
    pub kind: CandidateKind,
    #[serde(default)]
    pub mutable_surface: Vec<String>,
    #[serde(default)]
    pub agent_prompt_ref: Option<String>,
    #[serde(default)]
    pub prompt_hash: Option<String>,
    /// Stage 0 accepts only the manual proposer reference.
    #[serde(default)]
    pub proposer_kind: Option<String>,
    #[serde(default)]
    pub proposer_version: Option<String>,
}

impl CampaignCandidateSpec {
    /// Whether this arm changes any declared surface.
    pub fn is_baseline(&self) -> bool {
        self.kind == CandidateKind::Baseline
    }
}

/// The strict, versioned Stage 0 campaign plan. This document is the
/// immutable orchestration input: the backend derives the campaign identity
/// and every run scope from it, and the CLI re-reads it for each run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignPlanV1 {
    pub schema_version: String,
    /// Operator-chosen stable campaign key; part of the canonical plan hash.
    pub campaign_key: String,
    pub stage: String,
    pub agent_id: String,
    /// Checked-in benchmark suite id (fixture identity is digest-bound).
    pub suite: String,
    /// Checked-in benchmark task id.
    pub task: String,
    /// Optional act-phase `group@model` override (a run-time knob, never a
    /// candidate surface).
    #[serde(default)]
    pub model: Option<String>,
    /// Stage 0 fixes this at 1.
    pub concurrency: u32,
    /// The single frozen budget envelope shared by every arm in the campaign.
    pub budget: ExperimentBudgetSpec,
    pub candidates: Vec<CampaignCandidateSpec>,
    #[serde(default)]
    pub stop_conditions: CampaignStopConditions,
}

/// Plan-level keys a caller may never inject: every one of them would let a
/// candidate or caller reach outside the allowlisted declarative surface.
const FORBIDDEN_PLAN_KEYS: &[&str] = &[
    "campaign_id",
    "campaign_scope_id",
    "candidate_scope_id",
    "trial_scope_id",
    "request_scope_id",
    "scope_id",
    "scope_ids",
    "budget_override",
    "envelope",
    "envelope_hash",
    "caps",
    "verifier",
    "verifier_id",
    "verifier_version",
    "verifier_digest",
    "evaluator",
    "evaluator_id",
    "evaluator_dir",
    "verdict",
    "verdict_dir",
    "sandbox",
    "sandbox_config",
    "sandbox_scheme_id",
    "sandbox_execution_mode",
    "allowed_paths",
    "approval_level",
    "shell_policy",
    "auto_approve",
    "path_guard",
    "holdout",
    "private_holdout",
    "promotion",
    "promotion_policy",
    "ledger",
    "agent_defaults",
    "system_prompt",
    "planning_prompt",
    "prompt",
    "raw_prompt",
    "models",
    "tools",
    "mcp_tools",
    "skills",
];

/// Candidate-level keys a caller may never inject.
const FORBIDDEN_CANDIDATE_KEYS: &[&str] = &[
    "campaign_id",
    "campaign_scope_id",
    "candidate_scope_id",
    "trial_scope_id",
    "request_scope_id",
    "scope_id",
    "budget",
    "budget_override",
    "caps",
    "verifier",
    "verifier_id",
    "verifier_version",
    "verifier_digest",
    "verifier_dir",
    "evaluator",
    "evaluator_id",
    "evaluator_dir",
    "verdict",
    "verdict_dir",
    "sandbox",
    "sandbox_config",
    "sandbox_scheme_id",
    "sandbox_execution_mode",
    "allowed_paths",
    "approval_level",
    "shell_policy",
    "auto_approve",
    "path_guard",
    "holdout",
    "private_holdout",
    "promotion",
    "promotion_policy",
    "ledger",
    "agent_defaults",
    "system_prompt",
    "planning_prompt",
    "prompt",
    "raw_prompt",
    "models",
    "tools",
    "mcp_tools",
    "skills",
    // The baseline arm is the unmodified Agent defaults; a baseline that
    // declares a prompt reference is a caller bug, not a valid arm.
    "surface_version",
];

/// The declared field names of [`CampaignPlanV1`], used to classify a plan
/// field that appears at the wrong envelope level as a misplaced (forbidden)
/// field rather than an unknown one.
const PLAN_FIELD_NAMES: &[&str] = &[
    "schema_version",
    "campaign_key",
    "stage",
    "agent_id",
    "suite",
    "task",
    "model",
    "concurrency",
    "budget",
    "candidates",
    "stop_conditions",
];

fn reject_forbidden_keys(
    value: &Value,
    forbidden: &[&str],
    context: &str,
) -> Result<(), CampaignSpecError> {
    let Some(map) = value.as_object() else {
        return Err(CampaignSpecError::new(
            CampaignSpecErrorCode::UnknownField,
            format!("{context} must be a JSON object"),
        ));
    };
    for key in map.keys() {
        if forbidden.contains(&key.as_str()) {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::ForbiddenField,
                format!("'{key}' is not an allowed field for {context}"),
            ));
        }
    }
    Ok(())
}

fn is_valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEY_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Parses a strict campaign plan, rejecting unknown and forbidden fields with
/// stable machine codes before any validation or effect.
pub fn parse_campaign_plan(value: &Value) -> Result<CampaignPlanV1, CampaignSpecError> {
    reject_forbidden_keys(value, FORBIDDEN_PLAN_KEYS, "a campaign plan")?;
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        for candidate in candidates {
            reject_forbidden_keys(candidate, FORBIDDEN_CANDIDATE_KEYS, "a candidate manifest")?;
        }
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        CampaignSpecError::new(
            CampaignSpecErrorCode::UnknownField,
            format!("campaign plan is not a valid strict v1 document: {error}"),
        )
    })
}

/// Parses a strict campaign run intent. The path campaign id is authoritative;
/// the body can never carry a scope id.
pub fn parse_campaign_run_request(
    value: &Value,
) -> Result<CampaignRunRequestV1, CampaignSpecError> {
    reject_forbidden_keys(
        value,
        &[
            "campaign_id",
            "campaign_scope_id",
            "candidate_scope_id",
            "trial_scope_id",
            "request_scope_id",
            "scope_id",
            "budget",
            "caps",
            "verifier",
            "sandbox",
            "allowed_paths",
            "approval_level",
        ],
        "a campaign run request",
    )?;
    serde_json::from_value(value.clone()).map_err(|error| {
        CampaignSpecError::new(
            CampaignSpecErrorCode::UnknownField,
            format!("campaign run request is not a valid strict v1 document: {error}"),
        )
    })
}

/// The resolved checked-in fixture projection one run intent carries.
///
/// The backend cannot re-read the benchmark fixture (the fixture is compiled
/// into the CLI adapter), so the run intent declares its fixture identity
/// together with the exact instruction bytes and the instruction digest. The
/// digest is recomputed here, so a run can never execute an instruction that
/// does not match its declared identity. The authoritative fixture digests are
/// independently re-derived by the 2E verifier, and the campaign consumer
/// cross-checks them against the verdict before accepting a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignFixtureRefV1 {
    pub suite: String,
    pub task_id: String,
    /// The exact prompt submitted for this run (a checked-in, non-secret task
    /// instruction).
    pub instruction: String,
    pub instruction_hash: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    pub manifest_digest: String,
    pub task_digest: String,
    pub verifier_id: String,
    pub verifier_version: String,
}

/// The run intent sent to create one run under an existing campaign. It
/// carries the immutable plan (so the backend can re-derive and verify the
/// campaign identity) plus the candidate key and the fixture projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignRunRequestV1 {
    pub schema_version: String,
    pub candidate_key: String,
    pub fixture: CampaignFixtureRefV1,
    pub plan: CampaignPlanV1,
}

impl CampaignRunRequestV1 {
    /// Validates the intent end to end before any effect: strict version, the
    /// full plan (including its allowlisted candidate surface), the fixture
    /// identity/digest, and the candidate/campaign binding.
    pub fn validate(&self) -> Result<(), CampaignSpecError> {
        if self.schema_version != CAMPAIGN_RUN_REQUEST_V1 {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::UnsupportedVersion,
                format!(
                    "unsupported schema_version '{}' (expected '{CAMPAIGN_RUN_REQUEST_V1}')",
                    self.schema_version
                ),
            ));
        }
        self.plan.validate()?;
        if self.fixture.suite != self.plan.suite || self.fixture.task_id != self.plan.task {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidFixture,
                "run intent fixture does not match the frozen campaign plan",
            ));
        }
        if self.fixture.instruction.trim().is_empty() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidFixture,
                "run intent fixture instruction must be non-empty",
            ));
        }
        let declared = [
            self.fixture.dataset_id.as_str(),
            self.fixture.split.as_str(),
            self.fixture.manifest_digest.as_str(),
            self.fixture.task_digest.as_str(),
            self.fixture.verifier_id.as_str(),
            self.fixture.verifier_version.as_str(),
        ];
        if declared.iter().any(|value| value.trim().is_empty()) {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidFixture,
                "run intent fixture identity must be fully declared",
            ));
        }
        let expected = domain_hash(
            FIXTURE_INSTRUCTION_HASH_DOMAIN,
            self.fixture.instruction.as_bytes(),
        );
        if expected != self.fixture.instruction_hash {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::FixtureDigestMismatch,
                "run intent instruction does not match its declared digest",
            ));
        }
        self.plan.candidate(&self.candidate_key).ok_or_else(|| {
            CampaignSpecError::new(
                CampaignSpecErrorCode::UnknownCandidate,
                format!(
                    "candidate '{}' is not declared by the campaign plan",
                    self.candidate_key
                ),
            )
        })?;
        Ok(())
    }

    /// The backend-derived campaign scope id this intent belongs to.
    pub fn campaign_id(&self) -> String {
        campaign_id_for_plan(&self.plan.plan_hash())
    }

    /// The frozen envelope shared by every arm in the campaign.
    pub fn envelope(&self) -> Result<BudgetEnvelope, CampaignSpecError> {
        self.plan.envelope()
    }

    /// The declared candidate arm for this intent (validated by [`Self::validate`]).
    pub fn candidate(&self) -> Option<&CampaignCandidateSpec> {
        self.plan.candidate(&self.candidate_key)
    }

    /// Resolves the candidate's allowlisted prompt surface. The baseline arm
    /// resolves to `None`, which keeps the frozen Agent defaults.
    pub fn resolved_prompt(&self) -> Result<Option<ResolvedCandidatePrompt>, CampaignSpecError> {
        let candidate = self.candidate().ok_or_else(|| {
            CampaignSpecError::new(
                CampaignSpecErrorCode::UnknownCandidate,
                "candidate is not declared by the campaign plan",
            )
        })?;
        match candidate.kind {
            CandidateKind::Baseline => Ok(None),
            CandidateKind::Candidate => {
                let reference = candidate.agent_prompt_ref.as_deref().ok_or_else(|| {
                    CampaignSpecError::new(
                        CampaignSpecErrorCode::IncompletePromptRef,
                        "candidate has no agent_prompt_ref",
                    )
                })?;
                let hash = candidate.prompt_hash.as_deref().ok_or_else(|| {
                    CampaignSpecError::new(
                        CampaignSpecErrorCode::IncompletePromptRef,
                        "candidate has no prompt_hash",
                    )
                })?;
                resolve_candidate_prompt(reference, hash).map(Some)
            }
        }
    }
}

/// Transport-neutral projection of a campaign budget scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignProjection {
    pub campaign_id: String,
    pub status: String,
    pub pause_reason: Option<String>,
    pub committed: BudgetVector,
    pub reserved: BudgetVector,
    pub caps: ResourceCaps,
    pub infra_failure_count: u64,
    pub infra_failure_threshold: u32,
    /// Shared candidate scopes created by runs under this campaign.
    pub candidates: Vec<CampaignCandidateProjection>,
}

/// Transport-neutral projection of one shared candidate scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignCandidateProjection {
    pub candidate_scope_id: String,
    pub status: String,
    pub committed: BudgetVector,
    pub reserved: BudgetVector,
}

/// Result of creating (or replaying the creation of) a campaign scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignCreateResult {
    pub schema_version: String,
    pub campaign_id: String,
    pub campaign_key: String,
    pub campaign_hash: String,
    pub envelope_hash: String,
    pub catalog_digest: String,
    pub candidate_order: Vec<String>,
    pub concurrency: u32,
    pub status: String,
}

/// Result of closing a campaign scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignCloseResult {
    pub campaign_id: String,
    pub status: String,
    pub pause_reason: Option<String>,
    /// Whether this call performed the transition (a replay is a no-op).
    pub changed: bool,
}

/// The prompt surface reference a run was created with. Only the reference,
/// the digest and the catalog digest are ever surfaced or persisted; the
/// prompt body never leaves the checked-in catalog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignPromptSurfaceRef {
    pub agent_prompt_ref: String,
    pub prompt_hash: String,
    pub surface_version: u32,
    pub catalog_digest: String,
    pub surface_hash: String,
}

impl From<&ResolvedCandidatePrompt> for CampaignPromptSurfaceRef {
    fn from(resolved: &ResolvedCandidatePrompt) -> Self {
        Self {
            agent_prompt_ref: resolved.agent_prompt_ref.clone(),
            prompt_hash: resolved.prompt_hash.clone(),
            surface_version: resolved.surface_version,
            catalog_digest: resolved.catalog_digest.clone(),
            surface_hash: resolved.surface_hash(),
        }
    }
}

/// Result of creating one run under a shared campaign scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CampaignRunResult {
    pub schema_version: String,
    pub campaign_id: String,
    pub candidate_key: String,
    pub candidate_scope_id: String,
    pub trial_scope_id: String,
    pub request_scope_id: String,
    pub run_id: String,
    pub session_id: String,
    /// Always `"started"`; the durable terminal state is observed separately.
    pub status: String,
    /// `None` for the baseline arm (the frozen Agent defaults apply).
    pub prompt_surface: Option<CampaignPromptSurfaceRef>,
}

/// Parses and validates a strict run intent in one step.
pub fn parse_and_validate_campaign_run_request(
    value: &Value,
) -> Result<CampaignRunRequestV1, CampaignSpecError> {
    let request = parse_campaign_run_request(value)?;
    request.validate()?;
    Ok(request)
}

/// Parses and validates a strict campaign plan in one step.
pub fn parse_and_validate_campaign_plan(
    value: &Value,
) -> Result<CampaignPlanV1, CampaignSpecError> {
    let plan = parse_campaign_plan(value)?;
    plan.validate()?;
    Ok(plan)
}

/// Parses and validates the `POST /control/v1/campaigns` body, which carries
/// exactly one field: the immutable plan. A scope id, budget override or any
/// other injected key is rejected with a stable code before any effect.
pub fn parse_and_validate_campaign_create_request(
    value: &Value,
) -> Result<CampaignPlanV1, CampaignSpecError> {
    let Some(map) = value.as_object() else {
        return Err(CampaignSpecError::new(
            CampaignSpecErrorCode::UnknownField,
            "campaign create request must be a JSON object",
        ));
    };
    for key in map.keys() {
        if key == "plan" {
            continue;
        }
        // A dangerous field (scope id, budget/verifier/sandbox/allow-path
        // override) or a plan field used at the wrong level is a *forbidden*
        // field here; anything else is simply unknown.
        let forbidden =
            FORBIDDEN_PLAN_KEYS.contains(&key.as_str()) || PLAN_FIELD_NAMES.contains(&key.as_str());
        return Err(CampaignSpecError::new(
            if forbidden {
                CampaignSpecErrorCode::ForbiddenField
            } else {
                CampaignSpecErrorCode::UnknownField
            },
            format!("'{key}' is not an allowed field for a campaign create request"),
        ));
    }
    let plan = map.get("plan").ok_or_else(|| {
        CampaignSpecError::new(
            CampaignSpecErrorCode::UnknownField,
            "campaign create request requires a 'plan' field",
        )
    })?;
    parse_and_validate_campaign_plan(plan)
}

impl CampaignPlanV1 {
    /// Validates the full plan against the checked-in prompt catalog, the
    /// frozen budget rules and the Stage 0 single-variable rules. Nothing is
    /// created and no effect is admitted before this succeeds.
    pub fn validate(&self) -> Result<(), CampaignSpecError> {
        if self.schema_version != CAMPAIGN_PLAN_V1 {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::UnsupportedVersion,
                format!(
                    "unsupported schema_version '{}' (expected '{CAMPAIGN_PLAN_V1}')",
                    self.schema_version
                ),
            ));
        }
        if self.stage != STAGE_0_MANUAL {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidStage,
                format!("unsupported campaign stage '{}'", self.stage),
            ));
        }
        if !is_valid_key(&self.campaign_key) {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidCampaignKey,
                "campaign_key must be a non-empty lowercase [a-z0-9-_] key",
            ));
        }
        if self.agent_id.trim().is_empty() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidAgentId,
                "plan must name the agent the campaign runs with",
            ));
        }
        if self.suite.trim().is_empty() || self.task.trim().is_empty() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidFixture,
                "plan must name a checked-in suite and task",
            ));
        }
        if let Some(model) = &self.model {
            if model.split('@').count() != 2 || model.starts_with('@') || model.ends_with('@') {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::InvalidFixture,
                    "model must be 'group@model'",
                ));
            }
        }
        if self.concurrency != STAGE_0_CONCURRENCY {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::ConcurrencyNotOne,
                format!(
                    "stage 0 requires concurrency {STAGE_0_CONCURRENCY}, got {}",
                    self.concurrency
                ),
            ));
        }
        // Validate the frozen budget with the exact 2C rules before anything
        // else; a rejected envelope is never partially applied.
        self.envelope()?;
        if self.candidates.is_empty() {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidCandidateOrder,
                "plan must declare at least a baseline and one candidate",
            ));
        }
        let catalog = CandidatePromptCatalog::embedded();
        let mut seen: Vec<&str> = Vec::with_capacity(self.candidates.len());
        for (index, candidate) in self.candidates.iter().enumerate() {
            if !is_valid_key(&candidate.candidate_key) {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::InvalidCandidateKey,
                    format!(
                        "candidate_key '{}' must be a non-empty lowercase [a-z0-9-_] key",
                        candidate.candidate_key
                    ),
                ));
            }
            if seen.contains(&candidate.candidate_key.as_str()) {
                return Err(CampaignSpecError::new(
                    CampaignSpecErrorCode::DuplicateCandidate,
                    format!("duplicate candidate_key '{}'", candidate.candidate_key),
                ));
            }
            seen.push(candidate.candidate_key.as_str());

            validate_proposer_reference(candidate)?;

            match candidate.kind {
                CandidateKind::Baseline => {
                    if index != 0 {
                        return Err(CampaignSpecError::new(
                            CampaignSpecErrorCode::InvalidCandidateOrder,
                            "the baseline arm must be the first declared candidate",
                        ));
                    }
                    if !candidate.mutable_surface.is_empty()
                        || candidate.agent_prompt_ref.is_some()
                        || candidate.prompt_hash.is_some()
                    {
                        return Err(CampaignSpecError::new(
                            CampaignSpecErrorCode::BaselineNotImmutable,
                            "the baseline arm must not declare a mutable surface or prompt ref",
                        ));
                    }
                }
                CandidateKind::Candidate => {
                    if index == 0 {
                        return Err(CampaignSpecError::new(
                            CampaignSpecErrorCode::InvalidCandidateOrder,
                            "the first declared candidate must be the baseline arm",
                        ));
                    }
                    if candidate.mutable_surface.len() != 1
                        || candidate.mutable_surface[0] != CANDIDATE_SURFACE_AGENT_PROMPT_REF
                    {
                        return Err(CampaignSpecError::new(
                            CampaignSpecErrorCode::InvalidSurface,
                            format!(
                                "stage 0 allows exactly one mutable surface: \
                                 ['{CANDIDATE_SURFACE_AGENT_PROMPT_REF}']"
                            ),
                        ));
                    }
                    let (Some(agent_prompt_ref), Some(prompt_hash)) = (
                        candidate.agent_prompt_ref.as_deref(),
                        candidate.prompt_hash.as_deref(),
                    ) else {
                        return Err(CampaignSpecError::new(
                            CampaignSpecErrorCode::IncompletePromptRef,
                            "an agent_prompt_ref surface requires both agent_prompt_ref and prompt_hash",
                        ));
                    };
                    catalog.resolve(agent_prompt_ref, prompt_hash)?;
                }
            }
        }
        if !self.candidates.iter().any(|c| c.is_baseline()) {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidCandidateOrder,
                "plan must declare exactly one baseline candidate",
            ));
        }
        let baseline_count = self
            .candidates
            .iter()
            .filter(|candidate| candidate.is_baseline())
            .count();
        if baseline_count != 1 {
            return Err(CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidCandidateOrder,
                "plan must declare exactly one baseline candidate",
            ));
        }
        Ok(())
    }

    /// Converts the frozen plan budget into the 2B envelope, reusing the
    /// exact 2C spec rules so a campaign cannot admit what a 2C run rejects.
    pub fn envelope(&self) -> Result<BudgetEnvelope, CampaignSpecError> {
        let spec = ExperimentRunSpecV1 {
            schema_version: EXPERIMENT_RUN_SPEC_V1.to_string(),
            planning_mode: false,
            workflow: ExperimentWorkflowOverride::default(),
            budget: self.budget.clone(),
        };
        spec.to_envelope().map_err(|error| {
            CampaignSpecError::new(
                CampaignSpecErrorCode::InvalidBudget,
                format!("campaign budget rejected: {}", error.code.as_str()),
            )
        })
    }

    /// Canonical hash of the frozen plan. Volatile fields do not participate,
    /// so an offline re-parse of the same plan yields the same hash.
    pub fn plan_hash(&self) -> String {
        canonical_hash(
            PLAN_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// Declaration order of candidate keys; Stage 0 never reorders.
    pub fn candidate_order(&self) -> Vec<String> {
        self.candidates
            .iter()
            .map(|candidate| candidate.candidate_key.clone())
            .collect()
    }

    pub fn candidate(&self, candidate_key: &str) -> Option<&CampaignCandidateSpec> {
        self.candidates
            .iter()
            .find(|candidate| candidate.candidate_key == candidate_key)
    }

    /// Canonical hash of one candidate declaration, bound to the plan's agent
    /// identity so the same candidate key in another campaign differs.
    pub fn candidate_hash(&self, candidate: &CampaignCandidateSpec) -> String {
        canonical_hash(
            CANDIDATE_HASH_DOMAIN,
            &serde_json::json!({
                "campaign_key": self.campaign_key,
                "base_agent_id": self.agent_id,
                "candidate_key": candidate.candidate_key,
                "kind": candidate.kind.as_str(),
                "mutable_surface": candidate.mutable_surface,
                "agent_prompt_ref": candidate.agent_prompt_ref,
                "prompt_hash": candidate.prompt_hash,
            }),
        )
    }
}

fn validate_proposer_reference(candidate: &CampaignCandidateSpec) -> Result<(), CampaignSpecError> {
    match (
        candidate.proposer_kind.as_deref(),
        candidate.proposer_version.as_deref(),
    ) {
        (None, None) => Ok(()),
        (Some(PROPOSER_KIND_MANUAL), Some(PROPOSER_VERSION)) => Ok(()),
        _ => Err(CampaignSpecError::new(
            CampaignSpecErrorCode::UnsupportedProposer,
            format!(
                "stage 0 supports only the checked-in '{PROPOSER_KIND_MANUAL}' \
                 proposer version '{PROPOSER_VERSION}'"
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// Backend-derived opaque identities
// ---------------------------------------------------------------------------

/// Derives the opaque campaign scope id from the frozen plan hash. The id is
/// backend-minted (a domain-separated hash) so the same plan always maps to
/// the same campaign and a caller can never invent a scope id.
pub fn campaign_id_for_plan(plan_hash: &str) -> String {
    format!(
        "camp-{}",
        &domain_hash(CAMPAIGN_ID_DOMAIN, plan_hash.as_bytes())[..32]
    )
}

/// Derives the shared candidate scope id for one candidate key. The candidate
/// scope is reused by every run of that candidate inside the campaign.
pub fn candidate_scope_id_for(campaign_id: &str, candidate_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CANDIDATE_SCOPE_ID_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(campaign_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(candidate_key.as_bytes());
    format!("cand-{}", &hex::encode(hasher.finalize())[..32])
}

/// Derives the trial scope id for one (candidate, trial key) pair. A trial is
/// one task/replicate executed by one candidate.
pub fn trial_scope_id_for(candidate_scope_id: &str, trial_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(TRIAL_SCOPE_ID_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(candidate_scope_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(trial_key.as_bytes());
    format!("trial-{}", &hex::encode(hasher.finalize())[..32])
}

/// Rejects a campaign id that is not a backend-minted campaign scope id.
pub fn validate_campaign_id(campaign_id: &str) -> Result<(), CampaignSpecError> {
    let valid = campaign_id.len() == 37
        && campaign_id.starts_with("camp-")
        && campaign_id[5..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if valid {
        Ok(())
    } else {
        Err(CampaignSpecError::new(
            CampaignSpecErrorCode::InvalidCampaignId,
            "campaign id must be a backend-minted campaign scope id",
        ))
    }
}

/// The four-level scope kinds a campaign run must produce, innermost first.
/// Kept beside the identity derivation so a chain bug is a compile-adjacent
/// review of one place.
pub const CAMPAIGN_RUN_SCOPE_KINDS: [ScopeKind; 4] = [
    ScopeKind::Request,
    ScopeKind::Trial,
    ScopeKind::Candidate,
    ScopeKind::Campaign,
];

/// Resolves the workflow-local experiment prompt override embedded in a
/// serialized workflow agent config, if any.
///
/// The workflow snapshot stores only the ref, hash and catalog digest; the
/// prompt body is re-resolved here from the checked-in catalog. An unknown
/// ref, a stale hash or a catalog drift therefore fails closed instead of
/// silently running with the baseline prompt.
pub fn resolve_workflow_prompt_override(
    agent_config_json: &str,
) -> Result<Option<ResolvedCandidatePrompt>, CampaignSpecError> {
    let value: Value = match serde_json::from_str(agent_config_json) {
        Ok(value) => value,
        // A non-JSON config is not a campaign config; the normal start path
        // already treats an unparsable snapshot as "no overrides".
        Err(_) => return Ok(None),
    };
    let Some(reference) = value
        .get("experimentAgentPromptRef")
        .and_then(Value::as_str)
        .filter(|reference| !reference.trim().is_empty())
    else {
        return Ok(None);
    };
    let prompt_hash = value
        .get("experimentAgentPromptHash")
        .and_then(Value::as_str)
        .filter(|hash| !hash.trim().is_empty())
        .ok_or_else(|| {
            CampaignSpecError::new(
                CampaignSpecErrorCode::IncompletePromptRef,
                "workflow experiment prompt ref has no prompt hash",
            )
        })?;
    let resolved = resolve_candidate_prompt(reference, prompt_hash)?;
    let recorded_digest = value
        .get("experimentPromptCatalogDigest")
        .and_then(Value::as_str)
        .unwrap_or("");
    if recorded_digest != resolved.catalog_digest {
        return Err(CampaignSpecError::new(
            CampaignSpecErrorCode::PromptHashMismatch,
            "workflow experiment prompt catalog digest does not match the checked-in catalog",
        ));
    }
    Ok(Some(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn budget_json() -> Value {
        json!({
            "money_mode": { "mode": "token_resource_only" },
            "caps": {
                "input_tokens": 65536,
                "output_tokens": 128000,
                "wall_time_ms": 300000,
                "tool_calls": 0,
                "processes": 0,
                "concurrency": 1
            },
            "required_dimensions": [],
            "max_attempts": 1
        })
    }

    fn checked_in_ref() -> (String, String) {
        let catalog = CandidatePromptCatalog::embedded();
        let surface = catalog
            .surfaces()
            .first()
            .expect("the checked-in catalog declares at least one surface");
        (
            surface.agent_prompt_ref.clone(),
            surface.prompt_hash.clone(),
        )
    }

    fn plan_json() -> Value {
        let (reference, hash) = checked_in_ref();
        json!({
            "schema_version": CAMPAIGN_PLAN_V1,
            "campaign_key": "stage0-a",
            "stage": STAGE_0_MANUAL,
            "agent_id": "agent-1",
            "suite": "chatspeed-smoke",
            "task": "smoke_reply_ok",
            "model": "cs@free:ds-v4-flash",
            "concurrency": 1,
            "budget": budget_json(),
            "candidates": [
                { "candidate_key": "baseline", "kind": "baseline" },
                {
                    "candidate_key": "prompt-a",
                    "kind": "candidate",
                    "mutable_surface": ["agent_prompt_ref"],
                    "agent_prompt_ref": reference,
                    "prompt_hash": hash
                }
            ]
        })
    }

    fn plan() -> CampaignPlanV1 {
        parse_campaign_plan(&plan_json()).expect("plan parses")
    }

    /// Parses then semantically validates, returning the first rejection.
    fn plan_error(value: &Value) -> CampaignSpecError {
        match parse_campaign_plan(value) {
            Err(error) => error,
            Ok(plan) => plan.validate().expect_err("plan must be rejected"),
        }
    }

    #[test]
    fn checked_in_catalog_is_valid_and_digest_is_stable() {
        let catalog = CandidatePromptCatalog::embedded();
        assert!(!catalog.surfaces().is_empty());
        assert_eq!(catalog.catalog_id(), "chatspeed-candidate-surfaces");
        assert_eq!(catalog.digest().len(), 64);
        let reparsed = CandidatePromptCatalog::from_json(CandidatePromptCatalog::raw_json())
            .expect("catalog reparses");
        assert_eq!(reparsed.digest(), catalog.digest());
    }

    #[test]
    fn valid_plan_round_trips_and_hashes_stably() {
        let plan = plan();
        plan.validate().expect("validates");
        let reparsed = parse_campaign_plan(&serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(reparsed.plan_hash(), plan.plan_hash());
        assert_eq!(plan.candidate_order(), vec!["baseline", "prompt-a"]);
        let envelope = plan.envelope().expect("envelope");
        assert_eq!(envelope.max_attempts, 1);
        assert_eq!(envelope_hash(&envelope).len(), 64);
    }

    #[test]
    fn unknown_top_level_field_is_rejected() {
        let mut value = plan_json();
        value["bogus"] = json!(1);
        let error = parse_campaign_plan(&value).expect_err("unknown field must fail");
        assert_eq!(error.code, CampaignSpecErrorCode::UnknownField);
    }

    #[test]
    fn forbidden_scope_and_trust_fields_are_rejected_before_effect() {
        for key in [
            "campaign_scope_id",
            "candidate_scope_id",
            "trial_scope_id",
            "request_scope_id",
            "scope_id",
            "verifier_id",
            "evaluator_id",
            "sandbox_config",
            "allowed_paths",
            "approval_level",
            "shell_policy",
            "private_holdout",
            "promotion_policy",
            "agent_defaults",
            "system_prompt",
        ] {
            let mut value = plan_json();
            value[key] = json!("injected");
            let error = parse_campaign_plan(&value).unwrap_err();
            assert_eq!(
                error.code,
                CampaignSpecErrorCode::ForbiddenField,
                "{key} must be rejected as forbidden"
            );
        }
    }

    #[test]
    fn forbidden_candidate_fields_are_rejected_before_effect() {
        for key in [
            "candidate_scope_id",
            "budget",
            "verifier_digest",
            "sandbox_scheme_id",
            "allowed_paths",
            "approval_level",
            "private_holdout",
            "system_prompt",
            "prompt",
            "models",
        ] {
            let mut value = plan_json();
            value["candidates"][1][key] = json!("injected");
            let error = parse_campaign_plan(&value).unwrap_err();
            assert_eq!(
                error.code,
                CampaignSpecErrorCode::ForbiddenField,
                "candidate field {key} must be rejected as forbidden"
            );
        }
    }

    #[test]
    fn duplicate_candidate_is_rejected() {
        let mut value = plan_json();
        value["candidates"][1]["candidate_key"] = json!("baseline");
        let error = plan_error(&value);
        assert_eq!(error.code, CampaignSpecErrorCode::DuplicateCandidate);
    }

    #[test]
    fn baseline_must_be_first_and_immutable() {
        let mut swapped = plan_json();
        swapped["candidates"] = json!([
            {
                "candidate_key": "prompt-a",
                "kind": "candidate",
                "mutable_surface": ["agent_prompt_ref"],
                "agent_prompt_ref": checked_in_ref().0,
                "prompt_hash": checked_in_ref().1
            },
            { "candidate_key": "baseline", "kind": "baseline" }
        ]);
        let error = plan_error(&swapped);
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidCandidateOrder);

        let mut mutable_baseline = plan_json();
        mutable_baseline["candidates"][0]["agent_prompt_ref"] = json!("smoke-terse-v1");
        let error = plan_error(&mutable_baseline);
        assert_eq!(error.code, CampaignSpecErrorCode::BaselineNotImmutable);
    }

    #[test]
    fn non_allowlisted_surface_and_ref_hash_mismatch_fail_closed() {
        let mut multi_surface = plan_json();
        multi_surface["candidates"][1]["mutable_surface"] =
            json!(["agent_prompt_ref", "allowed_paths"]);
        let error = plan_error(&multi_surface);
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidSurface);

        let mut unknown_ref = plan_json();
        unknown_ref["candidates"][1]["agent_prompt_ref"] = json!("not-in-catalog");
        let error = plan_error(&unknown_ref);
        assert_eq!(error.code, CampaignSpecErrorCode::PromptRefNotAllowlisted);

        let mut bad_hash = plan_json();
        bad_hash["candidates"][1]["prompt_hash"] = json!("0".repeat(64));
        let error = plan_error(&bad_hash);
        assert_eq!(error.code, CampaignSpecErrorCode::PromptHashMismatch);
    }

    #[test]
    fn concurrency_pressure_and_stage_are_rejected() {
        let mut concurrent = plan_json();
        concurrent["concurrency"] = json!(2);
        let error = plan_error(&concurrent);
        assert_eq!(error.code, CampaignSpecErrorCode::ConcurrencyNotOne);

        let mut stage = plan_json();
        stage["stage"] = json!("stage_1_auto");
        let error = plan_error(&stage);
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidStage);
    }

    #[test]
    fn non_manual_proposer_is_rejected() {
        let mut value = plan_json();
        value["candidates"][1]["proposer_kind"] = json!("llm");
        value["candidates"][1]["proposer_version"] = json!("1");
        let error = plan_error(&value);
        assert_eq!(error.code, CampaignSpecErrorCode::UnsupportedProposer);
    }

    #[test]
    fn budget_reuses_two_c_rules() {
        let mut value = plan_json();
        value["budget"]["caps"]["disk_bytes"] = json!(1024);
        let plan = parse_campaign_plan(&value).expect("parses");
        let error = plan.validate().unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidBudget);

        let mut value = plan_json();
        value["budget"]["max_attempts"] = json!(2);
        let plan = parse_campaign_plan(&value).expect("parses");
        let error = plan.validate().unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidBudget);
    }

    #[test]
    fn plan_hash_ignores_no_fields_and_binds_candidates() {
        let base = plan();
        let mut changed = plan_json();
        changed["candidates"][1]["candidate_key"] = json!("prompt-b");
        let changed = parse_campaign_plan(&changed).unwrap();
        assert_ne!(base.plan_hash(), changed.plan_hash());
    }

    #[test]
    fn derived_identities_are_opaque_stable_and_distinct() {
        let plan = plan();
        let campaign_id = campaign_id_for_plan(&plan.plan_hash());
        assert_eq!(campaign_id, campaign_id_for_plan(&plan.plan_hash()));
        validate_campaign_id(&campaign_id).expect("valid campaign id");
        assert!(validate_campaign_id("campaign-1").is_err());
        assert!(validate_campaign_id("").is_err());

        let candidate = candidate_scope_id_for(&campaign_id, "prompt-a");
        let other = candidate_scope_id_for(&campaign_id, "baseline");
        assert_ne!(candidate, other);
        assert_eq!(
            candidate,
            candidate_scope_id_for(&campaign_id, "prompt-a"),
            "candidate scope ids must be stable across runs"
        );
        let trial = trial_scope_id_for(&candidate, "smoke_reply_ok");
        assert_ne!(trial, trial_scope_id_for(&candidate, "smoke_echo_ping"));
        assert_ne!(trial, candidate);
    }

    #[test]
    fn resolved_prompt_override_round_trips_through_a_config_snapshot() {
        let (reference, hash) = checked_in_ref();
        let resolved = resolve_candidate_prompt(&reference, &hash).expect("resolves");
        let config = json!({
            "experimentAgentPromptRef": reference,
            "experimentAgentPromptHash": hash,
            "experimentPromptCatalogDigest": resolved.catalog_digest,
        })
        .to_string();
        let from_config = resolve_workflow_prompt_override(&config)
            .expect("config resolves")
            .unwrap();
        assert_eq!(from_config, resolved);
        assert_eq!(from_config.surface_hash().len(), 64);

        // No override -> the normal workflow path is untouched.
        assert!(resolve_workflow_prompt_override("{}").unwrap().is_none());
        assert!(resolve_workflow_prompt_override("not-json")
            .unwrap()
            .is_none());
    }

    #[test]
    fn workflow_prompt_override_fails_closed_on_drift() {
        let (reference, hash) = checked_in_ref();
        let resolved = resolve_candidate_prompt(&reference, &hash).expect("resolves");

        let drifted_catalog = json!({
            "experimentAgentPromptRef": reference,
            "experimentAgentPromptHash": hash,
            "experimentPromptCatalogDigest": "0".repeat(64),
        })
        .to_string();
        let error = resolve_workflow_prompt_override(&drifted_catalog).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::PromptHashMismatch);

        let missing_hash = json!({ "experimentAgentPromptRef": reference }).to_string();
        let error = resolve_workflow_prompt_override(&missing_hash).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::IncompletePromptRef);

        let unknown_ref = json!({
            "experimentAgentPromptRef": "nope",
            "experimentAgentPromptHash": resolved.prompt_hash,
            "experimentPromptCatalogDigest": resolved.catalog_digest,
        })
        .to_string();
        let error = resolve_workflow_prompt_override(&unknown_ref).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::PromptRefNotAllowlisted);
    }

    #[test]
    fn malformed_catalog_is_rejected() {
        let error = CandidatePromptCatalog::from_json(r#"{"schema_version":1}"#).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::PromptCatalogInvalid);

        let stale = json!({
            "schema_version": 1,
            "catalog_id": "x",
            "catalog_version": 1,
            "surfaces": [{
                "agent_prompt_ref": "a",
                "surface_version": 1,
                "prompt_hash": "deadbeef",
                "system_prompt": "hello"
            }]
        });
        let error = CandidatePromptCatalog::from_json(&stale.to_string()).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::PromptCatalogInvalid);
    }

    fn fixture_json(instruction: &str) -> Value {
        json!({
            "suite": "chatspeed-smoke",
            "task_id": "smoke_reply_ok",
            "instruction": instruction,
            "instruction_hash": domain_hash(
                FIXTURE_INSTRUCTION_HASH_DOMAIN,
                instruction.as_bytes()
            ),
            "dataset_id": "chatspeed-smoke",
            "dataset_version": 2,
            "split": "smoke",
            "manifest_digest": "a".repeat(64),
            "task_digest": "b".repeat(64),
            "verifier_id": "chatspeed-smoke-verifier",
            "verifier_version": "2",
        })
    }

    fn run_request_json() -> Value {
        json!({
            "schema_version": CAMPAIGN_RUN_REQUEST_V1,
            "candidate_key": "prompt-a",
            "fixture": fixture_json("Reply with exactly: OK"),
            "plan": plan_json(),
        })
    }

    #[test]
    fn create_request_wraps_the_plan_strictly() {
        let wrapped = json!({ "plan": plan_json() });
        let parsed = parse_and_validate_campaign_create_request(&wrapped).expect("parses");
        assert_eq!(parsed.plan_hash(), plan().plan_hash());

        // A bare plan body is not the create envelope: its fields are
        // misplaced, so they are rejected as forbidden rather than unknown.
        let error = parse_and_validate_campaign_create_request(&plan_json()).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::ForbiddenField);

        // Injected trust/scope fields are rejected with the stable code.
        for key in ["campaign_scope_id", "scope_id", "budget", "allowed_paths"] {
            let mut value = json!({ "plan": plan_json() });
            value[key] = json!("injected");
            let error = parse_and_validate_campaign_create_request(&value).unwrap_err();
            assert_eq!(
                error.code,
                CampaignSpecErrorCode::ForbiddenField,
                "{key} must be forbidden"
            );
        }

        let mut unknown = json!({ "plan": plan_json() });
        unknown["bogus"] = json!(1);
        let error = parse_and_validate_campaign_create_request(&unknown).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::UnknownField);
    }

    #[test]
    fn run_request_is_strict_and_carries_no_scope_id() {
        let value = run_request_json();
        let request = parse_and_validate_campaign_run_request(&value).expect("parses");
        assert_eq!(request.candidate_key, "prompt-a");
        assert_eq!(
            request.campaign_id(),
            campaign_id_for_plan(&request.plan.plan_hash())
        );
        assert!(request.resolved_prompt().expect("resolves").is_some());

        let mut injected = value.clone();
        injected["candidate_scope_id"] = json!("cand-1");
        let error = parse_campaign_run_request(&injected).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::ForbiddenField);

        let mut unknown = value;
        unknown["bogus"] = json!(1);
        let error = parse_campaign_run_request(&unknown).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::UnknownField);
    }

    #[test]
    fn run_request_fails_closed_on_digest_candidate_and_fixture_mismatch() {
        let mut tampered = run_request_json();
        tampered["fixture"]["instruction"] = json!("Reply with exactly: PONG");
        let error = parse_and_validate_campaign_run_request(&tampered).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::FixtureDigestMismatch);

        let mut unknown_candidate = run_request_json();
        unknown_candidate["candidate_key"] = json!("not-declared");
        let error = parse_and_validate_campaign_run_request(&unknown_candidate).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::UnknownCandidate);

        let mut wrong_task = run_request_json();
        wrong_task["fixture"]["task_id"] = json!("smoke_echo_ping");
        let error = parse_and_validate_campaign_run_request(&wrong_task).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::InvalidFixture);

        let mut wrong_version = run_request_json();
        wrong_version["schema_version"] = json!("campaign_run_request.v2");
        let error = parse_and_validate_campaign_run_request(&wrong_version).unwrap_err();
        assert_eq!(error.code, CampaignSpecErrorCode::UnsupportedVersion);
    }

    #[test]
    fn baseline_candidate_resolves_to_no_prompt_override() {
        let mut value = run_request_json();
        value["candidate_key"] = json!("baseline");
        let request = parse_and_validate_campaign_run_request(&value).expect("parses");
        assert!(request.resolved_prompt().expect("resolves").is_none());
    }
}
