//! Budgeted experiment run spec, transport-neutral request/result and stable
//! validation errors (Phase 2C).
//!
//! This module defines the *contract* a caller submits to start one
//! budget-constrained, single-attempt experiment workflow. It is intentionally
//! strict and versioned:
//!
//! - [`ExperimentRunSpecV1`] rejects unknown fields and a wrong
//!   `schema_version` so a future schema cannot be silently misread as v1.
//! - The spec carries only a limited workflow override, the planning mode and
//!   a frozen budget envelope. The agent and prompt are supplied separately on
//!   the request, never inside the spec, so identity stays backend-controlled.
//! - [`ExperimentRunSpecV1::to_envelope`] converts the frozen budget into the
//!   2B [`BudgetEnvelope`] and enforces the 2C-only rules *before* any effect:
//!   `max_attempts == 1`, no hard cap on the not-yet-observable disk/network
//!   dimensions, and required dimensions must be hard-capped.
//!
//! The spec never mints scope, effect or attempt identity. Those are derived
//! by the backend facade from the durable session id.

use crate::budget::types::{
    BudgetEnvelope, CapLimit, MoneyMode, PricingSnapshot, ResourceCaps, ResourceDimension,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Fixed schema version literal for the v1 experiment run spec.
pub const EXPERIMENT_RUN_SPEC_V1: &str = "experiment_run_spec.v1";

/// Stable machine codes for experiment spec validation failures. These
/// describe a rejected request *before* any effect and are distinct from the
/// 2B admission runtime codes (a cap breach is an admission error, not a spec
/// error). They are part of the CLI/HTTP contract and must stay stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentSpecErrorCode {
    /// The `schema_version` is missing or not the supported v1 literal.
    UnsupportedVersion,
    /// The prompt is empty after trimming.
    EmptyPrompt,
    /// A workflow override value is malformed.
    InvalidConfig,
    /// The budget envelope is structurally invalid.
    InvalidBudget,
    /// The envelope requests more than one attempt; 2C is single-attempt.
    MaxAttemptsNotOne,
    /// A dimension that cannot yet be observed (disk/network) was capped or
    /// required; 2C fails closed rather than assuming zero usage.
    ResourceUnobservable,
    /// A money-budget currency does not match its pricing snapshot.
    CurrencyMismatch,
    /// The selected agent is a child agent and cannot host a run.
    ChildAgent,
}

impl ExperimentSpecErrorCode {
    /// Stable snake_case machine code.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExperimentSpecErrorCode::UnsupportedVersion => "unsupported_spec_version",
            ExperimentSpecErrorCode::EmptyPrompt => "empty_prompt",
            ExperimentSpecErrorCode::InvalidConfig => "invalid_config",
            ExperimentSpecErrorCode::InvalidBudget => "invalid_budget",
            ExperimentSpecErrorCode::MaxAttemptsNotOne => "max_attempts_not_one",
            ExperimentSpecErrorCode::ResourceUnobservable => "resource_unobservable",
            ExperimentSpecErrorCode::CurrencyMismatch => "currency_mismatch",
            ExperimentSpecErrorCode::ChildAgent => "child_agent",
        }
    }
}

/// A spec validation failure carrying a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentSpecError {
    pub code: ExperimentSpecErrorCode,
    pub message: String,
}

impl ExperimentSpecError {
    pub fn new(code: ExperimentSpecErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ExperimentSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ExperimentSpecError {}

/// Monetary mode of a budget spec. Mirrors [`MoneyMode`] but as a strict,
/// externally parseable DTO. A missing currency is never inferred as "free";
/// `token_resource_only` is the explicit no-money mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "mode")]
pub enum ExperimentMoneyMode {
    /// No monetary cap; token/resource hard caps still apply.
    TokenResourceOnly,
    /// Monetary hard cap in a profile-declared local currency with a bound
    /// pricing snapshot.
    Money {
        currency_code: String,
        cap_money_micros: u64,
        pricing: ExperimentPricingSpec,
    },
}

/// Strict pricing snapshot DTO bound to one provider/model in one currency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentPricingSpec {
    pub currency_code: String,
    pub provider_id: String,
    pub model_id: String,
    pub input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    pub cache_read_micros_per_million: u64,
    pub cache_write_micros_per_million: u64,
    #[serde(default)]
    pub reasoning_micros_per_million: Option<u64>,
    pub multiplier_micros: u64,
    pub source_hash: String,
}

impl ExperimentPricingSpec {
    fn to_snapshot(&self) -> PricingSnapshot {
        PricingSnapshot {
            currency_code: self.currency_code.clone(),
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
            input_micros_per_million: self.input_micros_per_million,
            output_micros_per_million: self.output_micros_per_million,
            cache_read_micros_per_million: self.cache_read_micros_per_million,
            cache_write_micros_per_million: self.cache_write_micros_per_million,
            reasoning_micros_per_million: self.reasoning_micros_per_million,
            multiplier_micros: self.multiplier_micros,
            source_hash: self.source_hash.clone(),
        }
    }
}

/// Per-dimension hard caps. A dimension that is absent (`null`) is explicitly
/// `not_applicable`, never unlimited.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentCapsSpec {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub wall_time_ms: Option<u64>,
    #[serde(default)]
    pub tool_calls: Option<u64>,
    #[serde(default)]
    pub processes: Option<u64>,
    #[serde(default)]
    pub disk_bytes: Option<u64>,
    #[serde(default)]
    pub network_bytes: Option<u64>,
    #[serde(default)]
    pub concurrency: Option<u64>,
    #[serde(default)]
    pub money_micros: Option<u64>,
}

fn cap(value: Option<u64>) -> CapLimit {
    match value {
        Some(limit) => CapLimit::HardCap(limit),
        None => CapLimit::NotApplicable,
    }
}

impl ExperimentCapsSpec {
    fn to_caps(&self) -> ResourceCaps {
        ResourceCaps {
            input_tokens: cap(self.input_tokens),
            output_tokens: cap(self.output_tokens),
            cache_read_tokens: cap(self.cache_read_tokens),
            cache_write_tokens: cap(self.cache_write_tokens),
            wall_time_ms: cap(self.wall_time_ms),
            tool_calls: cap(self.tool_calls),
            processes: cap(self.processes),
            disk_bytes: cap(self.disk_bytes),
            network_bytes: cap(self.network_bytes),
            concurrency: cap(self.concurrency),
            money: cap(self.money_micros),
        }
    }
}

/// Frozen budget envelope submitted with an experiment run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentBudgetSpec {
    pub money_mode: ExperimentMoneyMode,
    pub caps: ExperimentCapsSpec,
    #[serde(default)]
    pub required_dimensions: Vec<String>,
    /// 2C admits exactly one attempt per effect; any other value is rejected
    /// before an effect.
    pub max_attempts: u32,
    #[serde(default)]
    pub infra_failure_threshold: Option<u32>,
    #[serde(default)]
    pub reservation_lease_ms: Option<u64>,
}

/// Default reservation lease (10 minutes) when the spec omits it.
const DEFAULT_RESERVATION_LEASE_MS: u64 = 600_000;
/// Default number of infra failures that pauses the campaign.
const DEFAULT_INFRA_FAILURE_THRESHOLD: u32 = 5;

/// Limited workflow override an experiment may carry. Kept to the same knobs
/// the normal create request exposes so the backend can reuse the shared
/// config resolver unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentWorkflowOverride {
    /// Optional `group@model` act-phase override (identical to the CLI
    /// `--model` shortcut). Resolved by the backend, never trusted as scope.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_paths: Option<Vec<String>>,
    #[serde(default)]
    pub auto_approve_plan: Option<bool>,
    #[serde(default)]
    pub final_audit: Option<bool>,
}

/// The strict, versioned experiment run spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentRunSpecV1 {
    pub schema_version: String,
    #[serde(default)]
    pub planning_mode: bool,
    #[serde(default)]
    pub workflow: ExperimentWorkflowOverride,
    pub budget: ExperimentBudgetSpec,
}

impl ExperimentRunSpecV1 {
    /// Validates the version discriminator and converts the frozen budget
    /// into a 2B [`BudgetEnvelope`], applying the 2C-only rules before any
    /// effect. Fails closed with a stable machine code.
    pub fn to_envelope(&self) -> Result<BudgetEnvelope, ExperimentSpecError> {
        if self.schema_version != EXPERIMENT_RUN_SPEC_V1 {
            return Err(ExperimentSpecError::new(
                ExperimentSpecErrorCode::UnsupportedVersion,
                format!(
                    "unsupported schema_version '{}' (expected '{EXPERIMENT_RUN_SPEC_V1}')",
                    self.schema_version
                ),
            ));
        }
        if self.budget.max_attempts != 1 {
            return Err(ExperimentSpecError::new(
                ExperimentSpecErrorCode::MaxAttemptsNotOne,
                format!(
                    "experiment budget max_attempts must be 1, got {}",
                    self.budget.max_attempts
                ),
            ));
        }
        // Disk and network bytes have no owner instrumentation before 2G. A
        // hard cap on them would be silently unmeasured, so 2C fails closed
        // instead of pretending zero usage (mirrors the 2B resource_unobservable
        // contract).
        for (dimension, value) in [
            (ResourceDimension::DiskBytes, self.budget.caps.disk_bytes),
            (
                ResourceDimension::NetworkBytes,
                self.budget.caps.network_bytes,
            ),
        ] {
            if value.is_some() {
                return Err(ExperimentSpecError::new(
                    ExperimentSpecErrorCode::ResourceUnobservable,
                    format!(
                        "dimension {} cannot be hard-capped before isolated-owner instrumentation",
                        dimension.as_str()
                    ),
                ));
            }
        }

        let mut required_dimensions = BTreeSet::new();
        for name in &self.budget.required_dimensions {
            let dimension = parse_dimension(name).ok_or_else(|| {
                ExperimentSpecError::new(
                    ExperimentSpecErrorCode::InvalidBudget,
                    format!("unknown required dimension '{name}'"),
                )
            })?;
            if matches!(
                dimension,
                ResourceDimension::DiskBytes | ResourceDimension::NetworkBytes
            ) {
                return Err(ExperimentSpecError::new(
                    ExperimentSpecErrorCode::ResourceUnobservable,
                    format!(
                        "required dimension {} cannot be observed in 2C",
                        dimension.as_str()
                    ),
                ));
            }
            required_dimensions.insert(dimension);
        }

        let caps = self.budget.caps.to_caps();
        let money_mode = match &self.budget.money_mode {
            ExperimentMoneyMode::TokenResourceOnly => MoneyMode::TokenResourceOnly,
            ExperimentMoneyMode::Money {
                currency_code,
                cap_money_micros,
                pricing,
            } => {
                let snapshot = pricing.to_snapshot();
                if snapshot.currency_code != *currency_code {
                    return Err(ExperimentSpecError::new(
                        ExperimentSpecErrorCode::CurrencyMismatch,
                        format!(
                            "pricing currency '{}' does not match envelope currency '{currency_code}'",
                            snapshot.currency_code
                        ),
                    ));
                }
                MoneyMode::Money {
                    currency_code: currency_code.clone(),
                    cap_money_micros: *cap_money_micros,
                    pricing: snapshot,
                }
            }
        };

        let envelope = BudgetEnvelope {
            caps,
            required_dimensions,
            money_mode,
            max_attempts: self.budget.max_attempts,
            infra_failure_threshold: self
                .budget
                .infra_failure_threshold
                .unwrap_or(DEFAULT_INFRA_FAILURE_THRESHOLD),
            reservation_lease_ms: self
                .budget
                .reservation_lease_ms
                .unwrap_or(DEFAULT_RESERVATION_LEASE_MS),
        };
        // Reuse the 2B envelope invariants (money/cap consistency, required
        // dimensions must be hard-capped, max_attempts >= 1). Map any failure
        // to the stable spec code without leaking the raw message contract.
        envelope.validate().map_err(|error| {
            ExperimentSpecError::new(
                ExperimentSpecErrorCode::InvalidBudget,
                format!("budget envelope rejected: {}", error.code.as_str()),
            )
        })?;
        Ok(envelope)
    }
}

fn parse_dimension(name: &str) -> Option<ResourceDimension> {
    Some(match name {
        "input_tokens" => ResourceDimension::InputTokens,
        "output_tokens" => ResourceDimension::OutputTokens,
        "cache_read_tokens" => ResourceDimension::CacheReadTokens,
        "cache_write_tokens" => ResourceDimension::CacheWriteTokens,
        "wall_time_ms" => ResourceDimension::WallTimeMs,
        "tool_calls" => ResourceDimension::ToolCalls,
        "processes" => ResourceDimension::Processes,
        "disk_bytes" => ResourceDimension::DiskBytes,
        "network_bytes" => ResourceDimension::NetworkBytes,
        "concurrency" => ResourceDimension::Concurrency,
        "money_micros" => ResourceDimension::Money,
        _ => return None,
    })
}

/// Transport-neutral experiment run request. The agent and prompt are added
/// by the CLI/HTTP caller outside the strict spec; the spec itself never
/// carries identity.
#[derive(Debug, Clone)]
pub struct ExperimentRunRequest {
    pub agent_id: String,
    pub prompt: String,
    pub spec: ExperimentRunSpecV1,
}

/// Opaque references to the durable budget scope chain created for a run.
/// These are backend-generated identifiers surfaced for audit only; a caller
/// can never supply them to opt in or override admission.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExperimentScopeRefs {
    pub request_scope_id: String,
    pub trial_scope_id: String,
    pub candidate_scope_id: String,
    pub campaign_scope_id: String,
}

/// Transport-neutral experiment run result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ExperimentRunResult {
    pub schema_version: String,
    pub run_id: String,
    pub session_id: String,
    pub scopes: ExperimentScopeRefs,
    /// Always `"started"` on success; the durable terminal state is observed
    /// separately via snapshot + durable events.
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_json() -> serde_json::Value {
        serde_json::json!({
            "money_mode": { "mode": "token_resource_only" },
            "caps": {
                "input_tokens": 100000,
                "output_tokens": 100000,
                "wall_time_ms": 600000,
                "tool_calls": 100,
                "processes": 10,
                "concurrency": 4
            },
            "required_dimensions": ["input_tokens", "output_tokens"],
            "max_attempts": 1
        })
    }

    fn spec_json() -> serde_json::Value {
        serde_json::json!({
            "schema_version": EXPERIMENT_RUN_SPEC_V1,
            "planning_mode": false,
            "workflow": { "model": "cs@free:ds-v4-flash" },
            "budget": budget_json()
        })
    }

    fn parse(text: String) -> Result<ExperimentRunSpecV1, serde_json::Error> {
        serde_json::from_str(&text)
    }

    #[test]
    fn valid_spec_round_trips_to_envelope() {
        let spec: ExperimentRunSpecV1 = serde_json::from_value(spec_json()).expect("parse");
        let envelope = spec.to_envelope().expect("envelope");
        assert_eq!(envelope.max_attempts, 1);
        assert!(matches!(envelope.money_mode, MoneyMode::TokenResourceOnly));
        assert!(matches!(
            envelope.caps.input_tokens,
            CapLimit::HardCap(100000)
        ));
        assert!(matches!(envelope.caps.disk_bytes, CapLimit::NotApplicable));
        assert!(envelope
            .required_dimensions
            .contains(&ResourceDimension::InputTokens));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let mut value = spec_json();
        value["budget"]["caps"]["bogus"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ExperimentRunSpecV1>(value).is_err());
    }

    #[test]
    fn unknown_top_level_field_is_rejected() {
        let mut value = spec_json();
        value["agent_id"] = serde_json::json!("nope");
        assert!(serde_json::from_value::<ExperimentRunSpecV1>(value).is_err());
    }

    #[test]
    fn wrong_version_fails_before_effect() {
        let mut value = spec_json();
        value["schema_version"] = serde_json::json!("experiment_run_spec.v2");
        let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
        let error = spec.to_envelope().expect_err("must reject");
        assert_eq!(error.code, ExperimentSpecErrorCode::UnsupportedVersion);
    }

    #[test]
    fn max_attempts_not_one_is_rejected() {
        let mut value = spec_json();
        value["budget"]["max_attempts"] = serde_json::json!(2);
        let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
        let error = spec.to_envelope().expect_err("must reject");
        assert_eq!(error.code, ExperimentSpecErrorCode::MaxAttemptsNotOne);
    }

    #[test]
    fn disk_network_hard_cap_is_rejected() {
        for key in ["disk_bytes", "network_bytes"] {
            let mut value = spec_json();
            value["budget"]["caps"][key] = serde_json::json!(1024);
            let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
            let error = spec.to_envelope().expect_err("must reject");
            assert_eq!(error.code, ExperimentSpecErrorCode::ResourceUnobservable);
        }
    }

    #[test]
    fn required_dimension_without_cap_is_rejected() {
        let mut value = spec_json();
        value["budget"]["caps"]["tool_calls"] = serde_json::Value::Null;
        value["budget"]["required_dimensions"] = serde_json::json!(["tool_calls"]);
        let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
        let error = spec.to_envelope().expect_err("must reject");
        assert_eq!(error.code, ExperimentSpecErrorCode::InvalidBudget);
    }

    #[test]
    fn money_mode_requires_matching_currency() {
        let mut value = spec_json();
        value["budget"]["money_mode"] = serde_json::json!({
            "mode": "money",
            "currency_code": "cny",
            "cap_money_micros": 1000000,
            "pricing": {
                "currency_code": "usd",
                "provider_id": "prov",
                "model_id": "model",
                "input_micros_per_million": 1,
                "output_micros_per_million": 2,
                "cache_read_micros_per_million": 0,
                "cache_write_micros_per_million": 0,
                "multiplier_micros": 1000000,
                "source_hash": "hash"
            }
        });
        value["budget"]["caps"]["money_micros"] = serde_json::json!(1000000);
        let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
        let error = spec.to_envelope().expect_err("must reject");
        assert_eq!(error.code, ExperimentSpecErrorCode::CurrencyMismatch);
    }

    #[test]
    fn token_resource_only_with_money_cap_is_rejected() {
        let mut value = spec_json();
        value["budget"]["caps"]["money_micros"] = serde_json::json!(5);
        let spec: ExperimentRunSpecV1 = serde_json::from_value(value).expect("parse");
        let error = spec.to_envelope().expect_err("must reject");
        assert_eq!(error.code, ExperimentSpecErrorCode::InvalidBudget);
    }

    #[test]
    fn deserialize_from_json_text_matches_value_path() {
        let text = serde_json::to_string(&spec_json()).expect("serialize");
        let from_text = parse(text).expect("parse text");
        let from_value: ExperimentRunSpecV1 = serde_json::from_value(spec_json()).expect("parse");
        assert_eq!(
            from_text.schema_version, from_value.schema_version,
            "round-trip parity"
        );
    }
}
