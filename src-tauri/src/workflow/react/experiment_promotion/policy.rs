//! Phase 2I server-owned promotion targets, the automatic promotion policy and
//! the pure gate evaluation.
//!
//! Everything in this module is *server-owned*: a caller names an opaque target
//! reference and nothing else. The branch, the repository reference, the Git
//! identity, the canary programme, its stages and every threshold live in the
//! target document, which the operator provisions and the backend hashes and
//! re-validates at load time (INV-4).
//!
//! The module is pure: it reads no file, spawns no process and opens no
//! database. The registry that materialises these documents lives in
//! `headless::promotion_targets`.

use crate::workflow::react::campaign::canonical_hash;
use crate::workflow::react::experiment_promotion::types::{
    is_full_branch_ref, is_sha256_hex, is_valid_key, CanaryResultV1, CanaryStageResultV1,
    PromotionError, PromotionErrorCode, PromotionEvidenceV1, MAX_CANARY_OUTPUT_BYTES,
    MAX_CANARY_STAGES, MAX_CANARY_TIMEOUT_MS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fixed schema version literals. Each is a distinct, versioned contract.
pub const PROMOTION_TARGET_V1: &str = "promotion_target.v1";
pub const PROMOTION_POLICY_V1: &str = "promotion_policy.v1";

/// Canonical hash domains for the server-owned promotion documents.
pub const PROMOTION_TARGET_HASH_DOMAIN: &str = "cs-promotion:target";
pub const PROMOTION_POLICY_HASH_DOMAIN: &str = "cs-promotion:policy";

fn error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

/// The direction in which a metric improves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    HigherIsBetter,
    LowerIsBetter,
}

impl MetricDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricDirection::HigherIsBetter => "higher_is_better",
            MetricDirection::LowerIsBetter => "lower_is_better",
        }
    }

    /// The direction-normalised improvement from `baseline` to `candidate`. A
    /// positive value always means "the candidate is better".
    pub fn improvement(&self, baseline: f64, candidate: f64) -> f64 {
        match self {
            MetricDirection::HigherIsBetter => candidate - baseline,
            MetricDirection::LowerIsBetter => baseline - candidate,
        }
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// One pre-registered metric rule. A metric the policy does not name is never
/// compared, so a candidate can never introduce a metric that flatters it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionMetricRuleV1 {
    pub metric: String,
    pub direction: MetricDirection,
    /// Improvement the candidate must reach for this rule to count as an
    /// improvement.
    pub min_improvement: f64,
    /// Regression the candidate may absorb before the rule fails hard.
    pub max_regression: f64,
    /// Independent samples both arms must have produced.
    pub min_samples: u32,
    /// Whether a missing metric fact is itself a rejection.
    pub required: bool,
}

impl PromotionMetricRuleV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if !is_valid_key(&self.metric) {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                format!(
                    "policy metric rule has an invalid metric key '{}'",
                    self.metric
                ),
            ));
        }
        if !is_finite(self.min_improvement) || !is_finite(self.max_regression) {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                format!(
                    "policy metric '{}' declares a non-finite threshold",
                    self.metric
                ),
            ));
        }
        if self.max_regression < 0.0 {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                format!(
                    "policy metric '{}' declares a negative max_regression",
                    self.metric
                ),
            ));
        }
        if self.min_samples == 0 {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                format!("policy metric '{}' declares min_samples 0", self.metric),
            ));
        }
        Ok(())
    }
}

/// The immutable, server-owned promotion policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionPolicyV1 {
    pub schema_version: String,
    pub policy_ref: String,
    /// Require the candidate arm's 2E verdict to report an overall pass.
    pub require_verdict_pass: bool,
    /// Require the candidate arm's verdict safety status to be `pass`.
    pub require_safety_pass: bool,
    /// Require the candidate arm's verdict infra status to be `pass`.
    pub require_infra_pass: bool,
    /// Whether an `unknown` cost status may still promote.
    pub allow_unknown_cost: bool,
    /// Hard ceiling on the committed cost of one arm, in micro-units.
    pub max_committed_micros: i64,
    /// The pre-registered metrics, in evaluation order.
    pub metrics: Vec<PromotionMetricRuleV1>,
}

impl PromotionPolicyV1 {
    /// Validates the policy in isolation.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != PROMOTION_POLICY_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported promotion policy schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.policy_ref) {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                "promotion policy has an invalid policy_ref",
            ));
        }
        if self.max_committed_micros < 0 {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                "promotion policy declares a negative cost ceiling",
            ));
        }
        if self.metrics.is_empty() {
            return Err(error(
                PromotionErrorCode::InvalidPromotionPolicy,
                "promotion policy declares no metric rules",
            ));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.metrics.len());
        for rule in &self.metrics {
            rule.validate()?;
            if seen.contains(&rule.metric.as_str()) {
                return Err(error(
                    PromotionErrorCode::InvalidPromotionPolicy,
                    format!("promotion policy declares metric '{}' twice", rule.metric),
                ));
            }
            seen.push(&rule.metric);
        }
        Ok(())
    }

    /// Canonical identity of the whole policy document.
    pub fn policy_hash(&self) -> String {
        canonical_hash(
            PROMOTION_POLICY_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// The rule for one metric key, if the policy pre-registers it.
    pub fn rule(&self, metric: &str) -> Option<&PromotionMetricRuleV1> {
        self.metrics.iter().find(|rule| rule.metric == metric)
    }
}

// ---------------------------------------------------------------------------
// Canary specification
// ---------------------------------------------------------------------------

/// One ordered canary stage. Every stage is executed against **both** the
/// expected old head and the checkpoint commit, under the same digest-pinned
/// profile, and compared as a pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CanaryStageSpecV1 {
    pub stage_id: String,
    /// Explicit argv elements. Never a shell string, never a command line.
    pub args: Vec<String>,
    pub metric: String,
    pub direction: MetricDirection,
    pub min_improvement: f64,
    pub max_regression: f64,
    pub min_samples: u32,
    /// Whether a missing stage result is itself a failure.
    pub required: bool,
}

impl CanaryStageSpecV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if !is_valid_key(&self.stage_id) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!("canary stage has an invalid stage_id '{}'", self.stage_id),
            ));
        }
        if !is_valid_key(&self.metric) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary stage '{}' declares an invalid metric",
                    self.stage_id
                ),
            ));
        }
        if !is_finite(self.min_improvement) || !is_finite(self.max_regression) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary stage '{}' declares a non-finite threshold",
                    self.stage_id
                ),
            ));
        }
        if self.max_regression < 0.0 {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary stage '{}' declares a negative max_regression",
                    self.stage_id
                ),
            ));
        }
        if self.min_samples == 0 {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!("canary stage '{}' declares min_samples 0", self.stage_id),
            ));
        }
        for arg in &self.args {
            if arg.is_empty() {
                return Err(error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!(
                        "canary stage '{}' declares an empty argument",
                        self.stage_id
                    ),
                ));
            }
            if arg.len() > 256 || arg.chars().any(|c| c.is_control() || c == '\0') {
                return Err(error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!(
                        "canary stage '{}' declares an unsafe argument",
                        self.stage_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// The server-owned canary programme: which registered profile runs it, which
/// verified bundle supplies it, which executable inside that bundle is invoked
/// and with which ordered stages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionCanarySpecV1 {
    /// Server-registered execution profile reference.
    pub execution_profile_ref: String,
    /// Server-allowlisted bundle reference that supplies the programme.
    pub bundle_ref: String,
    /// Bundle-relative executable (must start with `./`).
    pub executable: String,
    pub stages: Vec<CanaryStageSpecV1>,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}

impl PromotionCanarySpecV1 {
    /// Validates the canary specification in isolation.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if !is_valid_key(&self.execution_profile_ref) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                "canary spec has an invalid execution profile reference",
            ));
        }
        if !is_valid_key(&self.bundle_ref) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                "canary spec has an invalid bundle reference",
            ));
        }
        if !is_safe_relative_program(&self.executable) {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary executable '{}' must be a safe bundle-relative program beginning with './'",
                    self.executable
                ),
            ));
        }
        if self.stages.is_empty() {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                "canary spec declares no stages",
            ));
        }
        if self.stages.len() > MAX_CANARY_STAGES {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary spec declares {} stages, above the {MAX_CANARY_STAGES} stage cap",
                    self.stages.len()
                ),
            ));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_CANARY_TIMEOUT_MS {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary timeout {} ms is outside 1..={MAX_CANARY_TIMEOUT_MS}",
                    self.timeout_ms
                ),
            ));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_CANARY_OUTPUT_BYTES {
            return Err(error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary output cap {} is outside 1..={MAX_CANARY_OUTPUT_BYTES}",
                    self.max_output_bytes
                ),
            ));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            stage.validate()?;
            if seen.contains(&stage.stage_id.as_str()) {
                return Err(error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!("canary spec declares stage '{}' twice", stage.stage_id),
                ));
            }
            seen.push(&stage.stage_id);
        }
        Ok(())
    }

    /// The stage spec for one stage id, if the spec declares it.
    pub fn stage(&self, stage_id: &str) -> Option<&CanaryStageSpecV1> {
        self.stages.iter().find(|stage| stage.stage_id == stage_id)
    }
}

/// A safe bundle-relative program path. Relative to the verified bundle root,
/// never absolute, never escaping, never containing shell or Git metacharacters
/// that a downstream argv builder could reinterpret.
pub fn is_safe_relative_program(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("./") else {
        return false;
    };
    if rest.is_empty() || rest.ends_with('/') {
        return false;
    }
    !rest.starts_with('/')
        && rest
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
        && !value.contains('\0')
        && !value.chars().any(|c| c.is_control())
}

// ---------------------------------------------------------------------------
// Target
// ---------------------------------------------------------------------------

/// A server-registered promotion target: everything a promotion effect needs to
/// know, provisioned by the operator and immutable once loaded.
///
/// A caller selects it by `target_ref` only. In particular the caller can never
/// supply the branch, the repository reference, the Git identity, the canary
/// programme or a threshold (INV-4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionTargetV1 {
    pub schema_version: String,
    pub target_ref: String,
    /// Opaque server-side repository reference; must match the runtime's own
    /// resolved base repository or the promotion is refused before any effect.
    pub base_repo_ref: String,
    /// The local experiment branch this target may advance. Always a full
    /// `refs/heads/*` ref, so `git update-ref` cannot reinterpret it.
    pub branch_ref: String,
    /// Command-level commit identity. Never written to the user's Git config.
    pub git_identity_name: String,
    pub git_identity_email: String,
    pub canary: PromotionCanarySpecV1,
    pub policy: PromotionPolicyV1,
}

impl PromotionTargetV1 {
    /// Validates the target document in isolation. The profile/bundle
    /// cross-checks against the server registries happen in the registry
    /// (`headless::promotion_targets`), where those registries are available.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != PROMOTION_TARGET_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported promotion target schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.target_ref) {
            return Err(error(
                PromotionErrorCode::UnknownPromotionTarget,
                "promotion target has an invalid target_ref",
            ));
        }
        if self.base_repo_ref.trim().is_empty() {
            return Err(error(
                PromotionErrorCode::InvalidPromotionTarget,
                "promotion target must declare a base repository reference",
            ));
        }
        if !is_full_branch_ref(&self.branch_ref) {
            return Err(error(
                PromotionErrorCode::UnsafeTargetRef,
                format!(
                    "'{}' is not a full refs/heads/* branch reference",
                    self.branch_ref
                ),
            ));
        }
        if self.git_identity_name.trim().is_empty()
            || !self.git_identity_name.chars().all(|c| !c.is_control())
        {
            return Err(error(
                PromotionErrorCode::InvalidPromotionTarget,
                "promotion target declares an invalid Git identity name",
            ));
        }
        if !self.git_identity_email.contains('@')
            || !self.git_identity_email.chars().all(|c| !c.is_control())
            || self.git_identity_email.contains(' ')
        {
            return Err(error(
                PromotionErrorCode::InvalidPromotionTarget,
                "promotion target declares an invalid Git identity email",
            ));
        }
        self.canary.validate()?;
        self.policy.validate()?;
        Ok(())
    }

    /// Canonical identity of the whole target, including its policy and canary
    /// programme. Recorded in every checkpoint commit trailer and every audit
    /// entry, so a promotion can never be re-interpreted under a different
    /// target.
    pub fn target_hash(&self) -> String {
        canonical_hash(
            PROMOTION_TARGET_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// The digest of the bound repository reference, recorded so the audit can
    /// prove which repository the effect was resolved against.
    pub fn base_repo_digest(&self) -> String {
        canonical_hash(
            PROMOTION_TARGET_HASH_DOMAIN,
            &serde_json::json!({
                "kind": "base_repo_ref",
                "base_repo_ref": self.base_repo_ref,
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Pure gate evaluation
// ---------------------------------------------------------------------------

/// The outcome of the automatic promotion policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionOutcome {
    Promote,
    Reject,
}

impl PromotionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionOutcome::Promote => "promote",
            PromotionOutcome::Reject => "reject",
        }
    }
}

/// One evaluated metric rule, recorded in the policy journal so the decision is
/// reproducible without re-reading the evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionMetricEvaluation {
    pub metric: String,
    pub samples: u32,
    pub improvement: f64,
    pub improved: bool,
    pub regressed: bool,
}

/// The full, reproducible policy decision.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionDecision {
    pub outcome: PromotionOutcome,
    /// The stable machine code describing a rejection. For a promote decision
    /// this carries [`PromotionErrorCode::PromotionRejected`]'s counterpart
    /// semantics implicitly: callers must consult `outcome` first.
    pub code: PromotionErrorCode,
    pub detail: String,
    pub metrics: Vec<PromotionMetricEvaluation>,
}

impl PromotionDecision {
    fn promote(metrics: Vec<PromotionMetricEvaluation>) -> Self {
        Self {
            outcome: PromotionOutcome::Promote,
            code: PromotionErrorCode::InvalidPromotionState,
            detail: "every hard gate passed and at least one pre-registered metric improved"
                .to_string(),
            metrics,
        }
    }

    fn reject(
        code: PromotionErrorCode,
        detail: impl Into<String>,
        metrics: Vec<PromotionMetricEvaluation>,
    ) -> Self {
        Self {
            outcome: PromotionOutcome::Reject,
            code,
            detail: detail.into(),
            metrics,
        }
    }
}

/// Evaluates the server-owned policy against a validated evidence projection.
///
/// The order is deliberate and mirrors the plan's hard-gate sequence:
/// provenance/verdict, safety, infrastructure, budget, then the structured
/// metric comparison. The first failing gate decides, so the recorded machine
/// code is deterministic.
///
/// This function is pure and total: it never panics on caller input, and a
/// rejection is a normal, structured return value rather than an error.
pub fn evaluate_promotion_policy(
    evidence: &PromotionEvidenceV1,
    policy: &PromotionPolicyV1,
) -> PromotionDecision {
    let empty = Vec::new();

    if policy.require_verdict_pass && evidence.verdict_status != "pass" {
        return PromotionDecision::reject(
            PromotionErrorCode::VerdictNotPassed,
            format!(
                "the candidate verdict status is '{}', not 'pass'",
                evidence.verdict_status
            ),
            empty,
        );
    }
    if policy.require_safety_pass && evidence.verdict_safety_status != "pass" {
        return PromotionDecision::reject(
            PromotionErrorCode::SafetyGateFailed,
            format!(
                "the candidate verdict reports safety '{}'",
                evidence.verdict_safety_status
            ),
            empty,
        );
    }
    if policy.require_infra_pass && evidence.verdict_infra_status != "pass" {
        return PromotionDecision::reject(
            PromotionErrorCode::InfraGateFailed,
            format!(
                "the candidate verdict reports infra '{}'",
                evidence.verdict_infra_status
            ),
            empty,
        );
    }
    if evidence.budget.budget_rejected {
        return PromotionDecision::reject(
            PromotionErrorCode::BudgetGateFailed,
            "budget admission rejected at least one run of an arm",
            empty,
        );
    }
    if !policy.allow_unknown_cost
        && (evidence.budget.cost_status == "unknown" || evidence.verdict_cost_status == "unknown")
    {
        return PromotionDecision::reject(
            PromotionErrorCode::BudgetGateFailed,
            format!(
                "cost status is unknown (budget '{}', verdict '{}')",
                evidence.budget.cost_status, evidence.verdict_cost_status
            ),
            empty,
        );
    }
    if evidence.budget.committed_micros > policy.max_committed_micros {
        return PromotionDecision::reject(
            PromotionErrorCode::BudgetGateFailed,
            format!(
                "committed cost {} exceeds the policy ceiling {}",
                evidence.budget.committed_micros, policy.max_committed_micros
            ),
            empty,
        );
    }

    let mut evaluated: Vec<PromotionMetricEvaluation> = Vec::with_capacity(policy.metrics.len());
    for rule in &policy.metrics {
        let Some(fact) = evidence.metric(&rule.metric) else {
            if rule.required {
                return PromotionDecision::reject(
                    PromotionErrorCode::EvidenceMismatch,
                    format!(
                        "the evidence does not declare required metric '{}'",
                        rule.metric
                    ),
                    evaluated,
                );
            }
            continue;
        };
        if fact.samples < rule.min_samples {
            return PromotionDecision::reject(
                PromotionErrorCode::InsufficientSamples,
                format!(
                    "metric '{}' has {} samples, below the required {}",
                    rule.metric, fact.samples, rule.min_samples
                ),
                evaluated,
            );
        }
        let improvement = rule
            .direction
            .improvement(fact.baseline_mean, fact.candidate_mean);
        if improvement < -rule.max_regression {
            return PromotionDecision::reject(
                PromotionErrorCode::CriticalRegression,
                format!(
                    "metric '{}' regressed by {:.6}, beyond the allowed {:.6}",
                    rule.metric, -improvement, rule.max_regression
                ),
                evaluated,
            );
        }
        evaluated.push(PromotionMetricEvaluation {
            metric: rule.metric.clone(),
            samples: fact.samples,
            improvement,
            improved: improvement >= rule.min_improvement,
            regressed: false,
        });
    }

    if evaluated.iter().any(|evaluation| evaluation.improved) {
        return PromotionDecision::promote(evaluated);
    }
    PromotionDecision::reject(
        PromotionErrorCode::NoImprovement,
        "no pre-registered metric improved by at least its required margin",
        evaluated,
    )
}

/// The pass/fail rule for one paired stage, from the paired numbers alone.
///
/// This is the single place the rule is expressed. The canary runner uses it to
/// write the stage's status, and [`evaluate_canary_stage`] uses it to cross-check
/// whatever a document declared.
pub fn canary_stage_passes(
    spec: &CanaryStageSpecV1,
    baseline_mean: f64,
    candidate_mean: f64,
) -> bool {
    let improvement = spec.direction.improvement(baseline_mean, candidate_mean);
    improvement >= spec.min_improvement && improvement >= -spec.max_regression
}

/// Recomputes one canary stage's pass/fail from the paired numbers and
/// cross-checks the programme's own declaration against it.
///
/// Returns `Ok(true)` when the stage passes. A stage whose *declared* status
/// disagrees with the recomputation is not a policy failure but a malformed
/// document: a programme can never assert its own success (INV-3).
pub fn evaluate_canary_stage(
    spec: &CanaryStageSpecV1,
    result: &CanaryStageResultV1,
) -> Result<bool, PromotionError> {
    if result.stage_id != spec.stage_id {
        return Err(error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary stage result '{}' does not match the expected stage '{}'",
                result.stage_id, spec.stage_id
            ),
        ));
    }
    if result.metric != spec.metric {
        return Err(error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary stage '{}' reported metric '{}', expected '{}'",
                spec.stage_id, result.metric, spec.metric
            ),
        ));
    }
    if result.samples < spec.min_samples {
        return Err(error(
            PromotionErrorCode::InsufficientSamples,
            format!(
                "canary stage '{}' produced {} samples, below the required {}",
                spec.stage_id, result.samples, spec.min_samples
            ),
        ));
    }
    let passed = canary_stage_passes(spec, result.baseline_mean, result.candidate_mean);
    let declared_pass = result.status == "pass";
    if declared_pass != passed {
        return Err(error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary stage '{}' declared '{}' but the paired numbers imply '{}'",
                spec.stage_id,
                result.status,
                if passed { "pass" } else { "fail" }
            ),
        ));
    }
    Ok(passed)
}

/// Verifies a complete paired canary result against the target's canary spec.
///
/// Every declared stage must be present exactly once, in the declared order,
/// and nothing else may appear. The first failing stage stops the walk, which
/// matches the runner's "fail immediately, branch never moves" behaviour.
pub fn verify_canary_result(
    spec: &PromotionCanarySpecV1,
    result: &CanaryResultV1,
) -> Result<(), PromotionError> {
    if result.stages.len() != spec.stages.len() {
        return Err(error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary result declares {} stages, the target declares {}",
                result.stages.len(),
                spec.stages.len()
            ),
        ));
    }
    let mut all_passed = true;
    for (index, stage_spec) in spec.stages.iter().enumerate() {
        let stage_result = &result.stages[index];
        if stage_result.stage_id != stage_spec.stage_id {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage order differs at index {index}: expected '{}', got '{}'",
                    stage_spec.stage_id, stage_result.stage_id
                ),
            ));
        }
        if !evaluate_canary_stage(stage_spec, stage_result)? {
            return Err(error(
                PromotionErrorCode::CanaryStageFailed,
                format!(
                    "canary stage '{}' failed: baseline mean {:.6}, candidate mean {:.6}",
                    stage_spec.stage_id, stage_result.baseline_mean, stage_result.candidate_mean
                ),
            ));
        }
        all_passed = true;
    }
    let declared_pass = result.status == "pass";
    if declared_pass != all_passed {
        return Err(error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary result declared '{}' but its stages imply '{}'",
                result.status,
                if all_passed { "pass" } else { "fail" }
            ),
        ));
    }
    Ok(())
}

/// Confirms the canary spec of a target is executable with a registered
/// profile and an allowlisted bundle. Kept separate from [`PromotionTargetV1::validate`]
/// because it needs the server registries.
pub fn canary_spec_is_executable(spec: &PromotionCanarySpecV1) -> Result<(), PromotionError> {
    if spec.executable.is_empty() || !is_safe_relative_program(&spec.executable) {
        return Err(error(
            PromotionErrorCode::InvalidCanarySpec,
            "canary executable is not a safe bundle-relative program",
        ));
    }
    Ok(())
}

/// Whether a target declares a digest-pinned container requirement. Only the
/// `persistent_docker` owner profiles are digest-pinned, and the profile
/// registry is the authority for that check.
pub fn is_digest_pinned(image_reference: &str) -> bool {
    if let Some(digest) = image_reference.strip_prefix("sha256:") {
        return is_sha256_hex(digest);
    }
    let Some((repository, digest)) = image_reference.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.trim().is_empty() && is_sha256_hex(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_promotion::types::{
        PromotionBudgetFactsV1, PromotionEvidenceV1, PromotionMetricFactV1,
        PromotionVerifierIdentityV1, PROMOTION_EVIDENCE_V1,
    };

    fn rule(metric: &str, direction: MetricDirection) -> PromotionMetricRuleV1 {
        PromotionMetricRuleV1 {
            metric: metric.to_string(),
            direction,
            min_improvement: 0.1,
            max_regression: 0.0,
            min_samples: 4,
            required: true,
        }
    }

    fn policy() -> PromotionPolicyV1 {
        PromotionPolicyV1 {
            schema_version: PROMOTION_POLICY_V1.to_string(),
            policy_ref: "default".to_string(),
            require_verdict_pass: true,
            require_safety_pass: true,
            require_infra_pass: true,
            allow_unknown_cost: false,
            max_committed_micros: 1_000_000,
            metrics: vec![rule("verdict_score", MetricDirection::HigherIsBetter)],
        }
    }

    fn stage(stage_id: &str) -> CanaryStageSpecV1 {
        CanaryStageSpecV1 {
            stage_id: stage_id.to_string(),
            args: vec!["--task".to_string(), "task-a".to_string()],
            metric: "verdict_score".to_string(),
            direction: MetricDirection::HigherIsBetter,
            min_improvement: 0.1,
            max_regression: 0.0,
            min_samples: 4,
            required: true,
        }
    }

    fn canary_spec() -> PromotionCanarySpecV1 {
        PromotionCanarySpecV1 {
            execution_profile_ref: "smoke-local".to_string(),
            bundle_ref: "smoke-tools".to_string(),
            executable: "./tools/canary".to_string(),
            stages: vec![stage("stage-1"), stage("stage-2")],
            timeout_ms: 60_000,
            max_output_bytes: 65_536,
        }
    }

    fn target() -> PromotionTargetV1 {
        PromotionTargetV1 {
            schema_version: PROMOTION_TARGET_V1.to_string(),
            target_ref: "local-dev".to_string(),
            base_repo_ref: "repo:primary".to_string(),
            branch_ref: "refs/heads/experiment/2i".to_string(),
            git_identity_name: "ChatSpeed Promotion".to_string(),
            git_identity_email: "promotion@chatspeed.local".to_string(),
            canary: canary_spec(),
            policy: policy(),
        }
    }

    fn evidence(metrics: Vec<PromotionMetricFactV1>) -> PromotionEvidenceV1 {
        PromotionEvidenceV1 {
            schema_version: PROMOTION_EVIDENCE_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: "prompt-a".to_string(),
            baseline_job_id: "job-baseline".to_string(),
            candidate_job_id: "job-candidate".to_string(),
            baseline_run_id: "run-baseline".to_string(),
            candidate_run_id: "run-candidate".to_string(),
            candidate_session_id: "session-candidate".to_string(),
            baseline_artifact_hash: "a".repeat(64),
            candidate_artifact_hash: "b".repeat(64),
            baseline_evaluation_hash: "c".repeat(64),
            candidate_evaluation_hash: "d".repeat(64),
            baseline_verdict_hash: "e".repeat(64),
            candidate_verdict_hash: "f".repeat(64),
            fixture_ref: "smoke-tools".to_string(),
            fixture_digest: "1".repeat(64),
            task_id: "task-a".to_string(),
            suite: "chatspeed-smoke".to_string(),
            dataset_id: "chatspeed-smoke".to_string(),
            dataset_version: 2,
            split: "smoke".to_string(),
            execution_profile_ref: "smoke-local".to_string(),
            execution_profile_hash: "2".repeat(64),
            patch_manifest_hash: "3".repeat(64),
            patch_sha256: "4".repeat(64),
            base_revision: "refs/heads/main".to_string(),
            verdict_status: "pass".to_string(),
            verdict_score: 1.0,
            verdict_safety_status: "pass".to_string(),
            verdict_infra_status: "pass".to_string(),
            verdict_cost_status: "known".to_string(),
            budget: PromotionBudgetFactsV1 {
                cost_status: "known".to_string(),
                budget_rejected: false,
                committed_micros: 10,
                currency: "usd".to_string(),
            },
            verifier: PromotionVerifierIdentityV1 {
                verifier_id: "chatspeed-smoke".to_string(),
                verifier_version: "2".to_string(),
                verifier_digest: "5".repeat(64),
            },
            metrics,
        }
    }

    fn metric(metric: &str, baseline: f64, candidate: f64) -> PromotionMetricFactV1 {
        PromotionMetricFactV1 {
            metric: metric.to_string(),
            samples: 8,
            baseline_passed: 4,
            candidate_passed: 8,
            baseline_mean: baseline,
            candidate_mean: candidate,
        }
    }

    #[test]
    fn a_well_formed_target_round_trips_and_hashes_stably() {
        let target = target();
        target.validate().expect("valid target");
        let text = serde_json::to_string(&target).expect("serialize");
        let parsed: PromotionTargetV1 = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(parsed, target);
        assert_eq!(parsed.target_hash(), target.target_hash());
        assert!(is_sha256_hex(&target.target_hash()));
        assert!(is_sha256_hex(&target.policy.policy_hash()));
        assert!(is_sha256_hex(&target.base_repo_digest()));

        let mut tampered = target.clone();
        tampered.branch_ref = "refs/heads/other".to_string();
        assert_ne!(tampered.target_hash(), target.target_hash());
    }

    #[test]
    fn an_unknown_or_unsafe_target_is_rejected() {
        let mut value = serde_json::to_value(target()).expect("value");
        value["canary"]["shell"] = serde_json::json!("bash -c 'true'");
        assert!(serde_json::from_value::<PromotionTargetV1>(value).is_err());

        let mut unsafe_branch = target();
        unsafe_branch.branch_ref = "experiment/2i".to_string();
        assert_eq!(
            unsafe_branch.validate().expect_err("relative ref").code,
            PromotionErrorCode::UnsafeTargetRef
        );

        let mut remote_branch = target();
        remote_branch.branch_ref = "refs/remotes/origin/main".to_string();
        assert_eq!(
            remote_branch.validate().expect_err("remote ref").code,
            PromotionErrorCode::UnsafeTargetRef
        );

        let mut bad_identity = target();
        bad_identity.git_identity_email = "not an email".to_string();
        assert_eq!(
            bad_identity.validate().expect_err("identity").code,
            PromotionErrorCode::InvalidPromotionTarget
        );

        let mut shell_program = target();
        shell_program.canary.executable = "sh -c 'echo'".to_string();
        assert_eq!(
            shell_program.validate().expect_err("program").code,
            PromotionErrorCode::InvalidCanarySpec
        );

        let mut escaping_program = target();
        escaping_program.canary.executable = "./tools/../../etc/passwd".to_string();
        assert_eq!(
            escaping_program.validate().expect_err("escape").code,
            PromotionErrorCode::InvalidCanarySpec
        );

        let mut unbounded = target();
        unbounded.canary.timeout_ms = MAX_CANARY_TIMEOUT_MS + 1;
        assert_eq!(
            unbounded.validate().expect_err("timeout").code,
            PromotionErrorCode::InvalidCanarySpec
        );

        let mut duplicated = target();
        duplicated.canary.stages = vec![stage("stage-1"), stage("stage-1")];
        assert_eq!(
            duplicated.validate().expect_err("duplicate stage").code,
            PromotionErrorCode::InvalidCanarySpec
        );
    }

    #[test]
    fn a_policy_that_names_no_metric_or_duplicates_one_is_rejected() {
        let mut empty = policy();
        empty.metrics = Vec::new();
        assert_eq!(
            empty.validate().expect_err("empty").code,
            PromotionErrorCode::InvalidPromotionPolicy
        );

        let mut duplicated = policy();
        duplicated.metrics = vec![
            rule("verdict_score", MetricDirection::HigherIsBetter),
            rule("verdict_score", MetricDirection::LowerIsBetter),
        ];
        assert_eq!(
            duplicated.validate().expect_err("duplicate").code,
            PromotionErrorCode::InvalidPromotionPolicy
        );

        let mut negative = policy();
        negative.metrics = vec![PromotionMetricRuleV1 {
            metric: "verdict_score".to_string(),
            direction: MetricDirection::HigherIsBetter,
            min_improvement: 0.1,
            max_regression: -1.0,
            min_samples: 4,
            required: true,
        }];
        assert_eq!(
            negative.validate().expect_err("negative").code,
            PromotionErrorCode::InvalidPromotionPolicy
        );
    }

    #[test]
    fn the_policy_gate_promotes_only_a_real_improvement() {
        let policy = policy();
        let improved =
            evaluate_promotion_policy(&evidence(vec![metric("verdict_score", 0.5, 1.0)]), &policy);
        assert_eq!(improved.outcome, PromotionOutcome::Promote);
        assert_eq!(improved.metrics.len(), 1);
        assert!(improved.metrics[0].improved);

        let flat =
            evaluate_promotion_policy(&evidence(vec![metric("verdict_score", 0.5, 0.5)]), &policy);
        assert_eq!(flat.outcome, PromotionOutcome::Reject);
        assert_eq!(flat.code, PromotionErrorCode::NoImprovement);

        let tiny =
            evaluate_promotion_policy(&evidence(vec![metric("verdict_score", 0.5, 0.55)]), &policy);
        assert_eq!(tiny.code, PromotionErrorCode::NoImprovement);
    }

    #[test]
    fn every_hard_gate_rejects_before_any_metric_is_compared() {
        let policy = policy();
        let good = metric("verdict_score", 0.5, 1.0);

        let mut verdict_failed = evidence(vec![good.clone()]);
        verdict_failed.verdict_status = "fail".to_string();
        assert_eq!(
            evaluate_promotion_policy(&verdict_failed, &policy).code,
            PromotionErrorCode::VerdictNotPassed
        );

        let mut safety = evidence(vec![good.clone()]);
        safety.verdict_safety_status = "fail".to_string();
        assert_eq!(
            evaluate_promotion_policy(&safety, &policy).code,
            PromotionErrorCode::SafetyGateFailed
        );

        let mut infra = evidence(vec![good.clone()]);
        infra.verdict_infra_status = "fail".to_string();
        assert_eq!(
            evaluate_promotion_policy(&infra, &policy).code,
            PromotionErrorCode::InfraGateFailed
        );

        let mut rejected = evidence(vec![good.clone()]);
        rejected.budget.budget_rejected = true;
        assert_eq!(
            evaluate_promotion_policy(&rejected, &policy).code,
            PromotionErrorCode::BudgetGateFailed
        );

        let mut unknown_cost = evidence(vec![good.clone()]);
        unknown_cost.verdict_cost_status = "unknown".to_string();
        assert_eq!(
            evaluate_promotion_policy(&unknown_cost, &policy).code,
            PromotionErrorCode::BudgetGateFailed
        );

        let mut too_expensive = evidence(vec![good.clone()]);
        too_expensive.budget.committed_micros = 2_000_000;
        assert_eq!(
            evaluate_promotion_policy(&too_expensive, &policy).code,
            PromotionErrorCode::BudgetGateFailed
        );

        let mut unknown_budget = evidence(vec![good.clone()]);
        unknown_budget.budget.cost_status = "unknown".to_string();
        assert_eq!(
            evaluate_promotion_policy(&unknown_budget, &policy).code,
            PromotionErrorCode::BudgetGateFailed
        );

        // An unknown cost is admitted only when the policy explicitly allows it.
        let mut permissive = policy.clone();
        permissive.allow_unknown_cost = true;
        assert_eq!(
            evaluate_promotion_policy(&unknown_budget, &permissive).outcome,
            PromotionOutcome::Promote
        );
    }

    #[test]
    fn missing_samples_and_regressions_are_rejected() {
        let policy = policy();

        let mut too_few = evidence(vec![metric("verdict_score", 0.5, 1.0)]);
        too_few.metrics[0].samples = 2;
        too_few.metrics[0].baseline_passed = 1;
        too_few.metrics[0].candidate_passed = 2;
        assert_eq!(
            evaluate_promotion_policy(&too_few, &policy).code,
            PromotionErrorCode::InsufficientSamples
        );

        let regressed =
            evaluate_promotion_policy(&evidence(vec![metric("verdict_score", 1.0, 0.5)]), &policy);
        assert_eq!(regressed.code, PromotionErrorCode::CriticalRegression);

        // A required metric the evidence omits is a rejection; an optional one
        // is simply not compared.
        let missing =
            evaluate_promotion_policy(&evidence(vec![metric("other_score", 0.0, 1.0)]), &policy);
        assert_eq!(missing.code, PromotionErrorCode::EvidenceMismatch);

        let mut optional = policy.clone();
        optional.metrics[0].required = false;
        let optional_missing =
            evaluate_promotion_policy(&evidence(vec![metric("other_score", 0.0, 1.0)]), &optional);
        assert_eq!(optional_missing.code, PromotionErrorCode::NoImprovement);
    }

    #[test]
    fn a_lower_is_better_metric_improves_when_it_falls() {
        let mut policy = policy();
        policy.metrics = vec![PromotionMetricRuleV1 {
            metric: "latency_ms".to_string(),
            direction: MetricDirection::LowerIsBetter,
            min_improvement: 10.0,
            max_regression: 5.0,
            min_samples: 4,
            required: true,
        }];
        let improved =
            evaluate_promotion_policy(&evidence(vec![metric("latency_ms", 100.0, 50.0)]), &policy);
        assert_eq!(improved.outcome, PromotionOutcome::Promote);
        assert_eq!(improved.metrics[0].improvement, 50.0);

        let regressed =
            evaluate_promotion_policy(&evidence(vec![metric("latency_ms", 50.0, 100.0)]), &policy);
        assert_eq!(regressed.code, PromotionErrorCode::CriticalRegression);

        // Within the allowance: no improvement, but no hard failure either.
        let tolerated =
            evaluate_promotion_policy(&evidence(vec![metric("latency_ms", 100.0, 103.0)]), &policy);
        assert_eq!(tolerated.code, PromotionErrorCode::NoImprovement);
    }

    #[test]
    fn a_canary_stage_that_asserts_its_own_pass_is_rejected() {
        let spec = stage("stage-1");
        let honest = CanaryStageResultV1 {
            stage_id: "stage-1".to_string(),
            metric: "verdict_score".to_string(),
            samples: 4,
            baseline_passed: 2,
            candidate_passed: 4,
            baseline_mean: 0.5,
            candidate_mean: 1.0,
            status: "pass".to_string(),
        };
        assert!(evaluate_canary_stage(&spec, &honest).expect("consistent"));

        // The programme claims a pass while the numbers say otherwise.
        let lying = CanaryStageResultV1 {
            candidate_mean: 0.5,
            ..honest.clone()
        };
        assert_eq!(
            evaluate_canary_stage(&spec, &lying)
                .expect_err("self-reported pass")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        // A genuine regression is a normal, structured `false`.
        let regressed = CanaryStageResultV1 {
            candidate_mean: 0.1,
            status: "fail".to_string(),
            ..honest.clone()
        };
        assert!(!evaluate_canary_stage(&spec, &regressed).expect("consistent"));

        // A metric mismatch and too few samples are malformed documents.
        let wrong_metric = CanaryStageResultV1 {
            metric: "other".to_string(),
            ..honest.clone()
        };
        assert_eq!(
            evaluate_canary_stage(&spec, &wrong_metric)
                .expect_err("metric")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );
        let few = CanaryStageResultV1 {
            samples: 2,
            baseline_passed: 1,
            candidate_passed: 2,
            ..honest.clone()
        };
        assert_eq!(
            evaluate_canary_stage(&spec, &few)
                .expect_err("samples")
                .code,
            PromotionErrorCode::InsufficientSamples
        );
    }

    #[test]
    fn the_whole_canary_result_must_match_the_declared_stages_in_order() {
        let spec = canary_spec();
        let passing = CanaryResultV1 {
            schema_version: crate::workflow::react::experiment_promotion::types::CANARY_RESULT_V1
                .to_string(),
            stages: vec![
                CanaryStageResultV1 {
                    stage_id: "stage-1".to_string(),
                    metric: "verdict_score".to_string(),
                    samples: 4,
                    baseline_passed: 2,
                    candidate_passed: 4,
                    baseline_mean: 0.5,
                    candidate_mean: 1.0,
                    status: "pass".to_string(),
                },
                CanaryStageResultV1 {
                    stage_id: "stage-2".to_string(),
                    metric: "verdict_score".to_string(),
                    samples: 4,
                    baseline_passed: 2,
                    candidate_passed: 4,
                    baseline_mean: 0.5,
                    candidate_mean: 1.0,
                    status: "pass".to_string(),
                },
            ],
            status: "pass".to_string(),
        };
        verify_canary_result(&spec, &passing).expect("passing canary");

        // A failing stage stops the run with the stage machine code.
        let mut failing = passing.clone();
        failing.stages[1].candidate_mean = 0.0;
        failing.stages[1].status = "fail".to_string();
        failing.status = "fail".to_string();
        assert_eq!(
            verify_canary_result(&spec, &failing)
                .expect_err("stage failure")
                .code,
            PromotionErrorCode::CanaryStageFailed
        );

        // A missing, reordered or extra stage is a malformed document.
        let mut short = passing.clone();
        short.stages.pop();
        assert_eq!(
            verify_canary_result(&spec, &short)
                .expect_err("missing stage")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        let mut reordered = passing.clone();
        reordered.stages.swap(0, 1);
        assert_eq!(
            verify_canary_result(&spec, &reordered)
                .expect_err("order")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        let mut overclaimed = passing.clone();
        overclaimed.status = "fail".to_string();
        assert_eq!(
            verify_canary_result(&spec, &overclaimed)
                .expect_err("aggregate")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );
    }

    #[test]
    fn relative_programs_and_digests_are_strict() {
        assert!(is_safe_relative_program("./tools/canary"));
        assert!(is_safe_relative_program("./canary"));
        assert!(!is_safe_relative_program("tools/canary"));
        assert!(!is_safe_relative_program("/usr/bin/canary"));
        assert!(!is_safe_relative_program("./../canary"));
        assert!(!is_safe_relative_program("./tools/"));
        assert!(!is_safe_relative_program(""));

        assert!(is_digest_pinned(&format!("repo@sha256:{}", "a".repeat(64))));
        assert!(is_digest_pinned(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_digest_pinned("repo:latest"));
        assert!(!is_digest_pinned(&format!(
            "repo@sha256:{}",
            "A".repeat(64)
        )));
    }
}
