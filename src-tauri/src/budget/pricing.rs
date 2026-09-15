//! Bounded worst-case pricing for the budget admission ledger (Phase 2B).
//!
//! This module converts the existing catalog `PricingConfig` (floating
//! point, per-million rates) into a validated integer [`PricingSnapshot`]
//! and computes deterministic, round-up worst-case money costs for a
//! bounded effect estimate. It never changes the existing actual-usage
//! cost semantics in `ai::model_catalog::pricing`; the ledger never stores
//! floating point counters.
//!
//! Fail-closed rules:
//! - missing, non-finite or negative prices are rejected (`unpriced_model`);
//! - a pricing snapshot bound to a different provider/model/currency than
//!   the resolved request is rejected (`unpriced_model`);
//! - a required output bound that cannot be established is rejected
//!   (`missing_bound`);
//! - token/resource-only profiles never require a price and never carry a
//!   money estimate.

use crate::budget::errors::AdmissionError;
use crate::budget::types::{
    BudgetEnvelope, BudgetVector, CapLimit, MoneyMode, PricingSnapshot, ResourceDimension,
};
use crate::db::PricingConfig;

const MICROS_PER_UNIT: u128 = 1_000_000;
const TOKENS_PER_MILLION: u128 = 1_000_000;

/// Converts a finite, non-negative per-million price into integer micros,
/// always rounding up (conservative for a hard-cap ledger).
fn price_to_micros(value: f64, field: &str) -> Result<u64, AdmissionError> {
    if !value.is_finite() || value < 0.0 {
        return Err(AdmissionError::unpriced_model(format!(
            "pricing field {field} is not a finite non-negative price: {value}"
        )));
    }
    let micros = (value as u128) * MICROS_PER_UNIT;
    let fraction = value - (value as u128) as f64;
    let fraction_micros = if fraction > 0.0 {
        // Round the fractional part up to at least one micro when present.
        (fraction * MICROS_PER_UNIT as f64).ceil() as u128
    } else {
        0
    };
    u64::try_from(micros + fraction_micros).map_err(|_| {
        AdmissionError::unpriced_model(format!(
            "pricing field {field} overflows the integer micro representation"
        ))
    })
}

/// Deterministic audit hash of the original pricing payload.
fn source_hash(pricing: &PricingConfig) -> String {
    use std::hash::{Hash, Hasher};
    let payload = serde_json::to_string(pricing).unwrap_or_else(|_| String::new());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Builds a validated pricing snapshot from the catalog pricing config.
/// Fails closed when any required price is missing, non-finite or negative.
pub fn pricing_snapshot_from_config(
    provider_id: &str,
    model_id: &str,
    currency_code: &str,
    pricing: &PricingConfig,
) -> Result<PricingSnapshot, AdmissionError> {
    if provider_id.trim().is_empty() || model_id.trim().is_empty() {
        return Err(AdmissionError::unpriced_model(
            "resolved provider/model must be known before pricing",
        ));
    }
    let input = price_to_micros(pricing.input_per_million, "input_per_million")?;
    let output = price_to_micros(pricing.output_per_million, "output_per_million")?;
    let cache_read = price_to_micros(pricing.cache_per_million, "cache_per_million")?;
    let cache_write = price_to_micros(pricing.cache_write_per_million, "cache_write_per_million")?;
    let reasoning = match &pricing.reasoning_per_million {
        Some(value) => Some(price_to_micros(*value, "reasoning_per_million")?),
        None => None,
    };
    let multiplier_micros = if pricing.multiplier.is_finite() && pricing.multiplier >= 0.0 {
        price_to_micros(pricing.multiplier, "multiplier")?
    } else {
        return Err(AdmissionError::unpriced_model(
            "pricing multiplier is not a finite non-negative number",
        ));
    };
    let snapshot = PricingSnapshot {
        currency_code: currency_code.to_string(),
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        input_micros_per_million: input,
        output_micros_per_million: output,
        cache_read_micros_per_million: cache_read,
        cache_write_micros_per_million: cache_write,
        reasoning_micros_per_million: reasoning,
        multiplier_micros,
        source_hash: source_hash(pricing),
    };
    snapshot.validate()?;
    Ok(snapshot)
}

/// Computes the worst-case money cost in integer micros for a bounded
/// token estimate, applying the multiplier with deterministic round-up.
/// Reasoning tokens are priced at the higher of the reasoning and output
/// rates (worst case). All arithmetic uses u128 internally and fails
/// closed on overflow instead of wrapping.
pub fn worst_case_money_micros(
    snapshot: &PricingSnapshot,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    max_output_tokens: u64,
) -> Result<u64, AdmissionError> {
    let effective_output_rate = snapshot
        .reasoning_micros_per_million
        .unwrap_or(0)
        .max(snapshot.output_micros_per_million);
    let base_input = (input_tokens as u128)
        .saturating_sub(cache_read_tokens as u128)
        .saturating_sub(cache_write_tokens as u128);
    let component_micros = |tokens: u128, rate: u64| -> Result<u128, AdmissionError> {
        tokens
            .checked_mul(rate as u128)
            .ok_or_else(|| AdmissionError::unpriced_model("token cost multiplication overflowed"))?
            .checked_add(TOKENS_PER_MILLION - 1)
            .ok_or_else(|| AdmissionError::unpriced_model("token cost rounding overflowed"))
            .map(|value| value / TOKENS_PER_MILLION)
    };
    let total = component_micros(base_input, snapshot.input_micros_per_million)?
        .checked_add(component_micros(
            cache_read_tokens as u128,
            snapshot.cache_read_micros_per_million,
        )?)
        .ok_or_else(|| AdmissionError::unpriced_model("token cost addition overflowed"))?
        .checked_add(component_micros(
            cache_write_tokens as u128,
            snapshot.cache_write_micros_per_million,
        )?)
        .ok_or_else(|| AdmissionError::unpriced_model("token cost addition overflowed"))?
        .checked_add(component_micros(
            max_output_tokens as u128,
            effective_output_rate,
        )?)
        .ok_or_else(|| AdmissionError::unpriced_model("token cost addition overflowed"))?;
    let multiplied = total
        .checked_mul(snapshot.multiplier_micros as u128)
        .ok_or_else(|| AdmissionError::unpriced_model("cost multiplier overflowed"))?
        .checked_add(MICROS_PER_UNIT - 1)
        .ok_or_else(|| AdmissionError::unpriced_model("cost multiplier rounding overflowed"))?
        / MICROS_PER_UNIT;
    u64::try_from(multiplied)
        .map_err(|_| AdmissionError::unpriced_model("worst-case cost exceeds the ledger range"))
}

/// Computes the settled money cost for ACTUAL usage tokens under the
/// frozen envelope's pricing snapshot, with the same deterministic
/// round-up as the worst-case reserve. Returns `Ok(0)` for
/// token/resource-only mode. Settlement must never commit a money value
/// of zero in money-budget mode: doing so would silently release the
/// monetary hold and let sequential requests exceed the money cap.
pub fn settlement_money_micros(
    envelope: &BudgetEnvelope,
    tokens: &BudgetVector,
) -> Result<u64, AdmissionError> {
    match &envelope.money_mode {
        MoneyMode::TokenResourceOnly => Ok(0),
        MoneyMode::Money { pricing, .. } => worst_case_money_micros(
            pricing,
            tokens.input_tokens,
            tokens.cache_read_tokens,
            tokens.cache_write_tokens,
            tokens.output_tokens,
        ),
    }
}

/// Bounded inputs for one LLM/embedding effect estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmEffectBound {
    pub provider_id: String,
    pub model_id: String,
    /// Normalized request token estimate (upper bound), including cache
    /// read/write components.
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Explicit hard output bound. `None` is only acceptable when the
    /// envelope does not cap output tokens.
    pub max_output_tokens: Option<u64>,
}

/// Builds the worst-case [`BudgetVector`] for one LLM/embedding effect
/// under the frozen envelope. Money-budget mode requires a pricing
/// snapshot matching the resolved provider/model and currency; missing or
/// invalid pricing fails closed with `unpriced_model` and no outbound
/// effect may be produced by the caller.
pub fn build_llm_estimate(
    envelope: &BudgetEnvelope,
    bound: LlmEffectBound,
) -> Result<BudgetVector, AdmissionError> {
    envelope.validate()?;
    let output_tokens = match envelope.caps.output_tokens {
        CapLimit::HardCap(_) => bound.max_output_tokens.ok_or_else(|| {
            AdmissionError::missing_bound(
                "output tokens are capped but no explicit max output bound could be established",
            )
        })?,
        CapLimit::NotApplicable => 0,
    };
    let mut estimate = BudgetVector {
        input_tokens: bound.input_tokens,
        output_tokens,
        cache_read_tokens: bound.cache_read_tokens,
        cache_write_tokens: bound.cache_write_tokens,
        ..BudgetVector::ZERO
    };
    match &envelope.money_mode {
        MoneyMode::TokenResourceOnly => {
            estimate.money_micros = 0;
        }
        MoneyMode::Money {
            currency_code,
            cap_money_micros: _,
            pricing,
        } => {
            if pricing.currency_code != *currency_code {
                return Err(AdmissionError::unpriced_model(
                    "pricing snapshot currency does not match the envelope currency",
                ));
            }
            if pricing.provider_id != bound.provider_id || pricing.model_id != bound.model_id {
                return Err(AdmissionError::unpriced_model(format!(
                    "no matching price for {}/{} in currency {}",
                    bound.provider_id, bound.model_id, currency_code
                )));
            }
            estimate.money_micros = worst_case_money_micros(
                pricing,
                bound.input_tokens,
                bound.cache_read_tokens,
                bound.cache_write_tokens,
                output_tokens,
            )?;
            estimate = estimate.with_dimension(ResourceDimension::Money, estimate.money_micros)?;
        }
    }
    Ok(estimate)
}

#[cfg(test)]
mod budget_pricing {
    use super::*;
    use crate::budget::errors::AdmissionErrorCode;
    use crate::budget::types::{MoneyMode, PricingSnapshot, ResourceCaps};
    use std::collections::BTreeSet;

    fn caps(money: CapLimit) -> ResourceCaps {
        ResourceCaps {
            input_tokens: CapLimit::HardCap(1_000_000),
            output_tokens: CapLimit::HardCap(100_000),
            cache_read_tokens: CapLimit::HardCap(1_000_000),
            cache_write_tokens: CapLimit::HardCap(1_000_000),
            wall_time_ms: CapLimit::HardCap(60_000),
            tool_calls: CapLimit::HardCap(10),
            processes: CapLimit::HardCap(2),
            disk_bytes: CapLimit::NotApplicable,
            network_bytes: CapLimit::NotApplicable,
            concurrency: CapLimit::HardCap(2),
            money,
        }
    }

    fn pricing() -> PricingConfig {
        PricingConfig {
            input_per_million: 1.0,
            output_per_million: 2.0,
            cache_per_million: 0.1,
            cache_write_per_million: 0.2,
            reasoning_per_million: None,
            reasoning_pricing_mode: "output".into(),
            audio_input_per_million: 0.0,
            audio_output_per_million: 0.0,
            multiplier: 1.0,
            pricing_source: Some("test".into()),
            tiers: Vec::new(),
        }
    }

    fn money_envelope(pricing: &PricingConfig, currency: &str) -> BudgetEnvelope {
        let snapshot =
            pricing_snapshot_from_config("prov", "model", currency, pricing).expect("snapshot");
        let cap = 100_000_000u64;
        BudgetEnvelope {
            caps: caps(CapLimit::HardCap(cap)),
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::Money {
                currency_code: currency.to_string(),
                cap_money_micros: cap,
                pricing: snapshot,
            },
            max_attempts: 1,
            infra_failure_threshold: 3,
            reservation_lease_ms: 60_000,
        }
    }

    fn token_only_envelope() -> BudgetEnvelope {
        BudgetEnvelope {
            caps: caps(CapLimit::NotApplicable),
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::TokenResourceOnly,
            max_attempts: 1,
            infra_failure_threshold: 3,
            reservation_lease_ms: 60_000,
        }
    }

    fn bound() -> LlmEffectBound {
        LlmEffectBound {
            provider_id: "prov".into(),
            model_id: "model".into(),
            input_tokens: 1_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            max_output_tokens: Some(500),
        }
    }

    #[test]
    fn price_conversion_rounds_up_deterministically() {
        let mut config = pricing();
        config.input_per_million = 0.15;
        let snapshot =
            pricing_snapshot_from_config("prov", "model", "usd", &config).expect("snapshot");
        assert_eq!(snapshot.input_micros_per_million, 150_000);
        // 3 tokens at 150000 micros/million rounds up to 1 micro.
        let cost = worst_case_money_micros(&snapshot, 3, 0, 0, 0).expect("cost");
        assert_eq!(cost, 1);
        // Same inputs always produce the same cost.
        let again = worst_case_money_micros(&snapshot, 3, 0, 0, 0).expect("cost");
        assert_eq!(cost, again);
    }

    #[test]
    fn money_mode_computes_bounded_worst_case_cost() {
        let config = pricing();
        let envelope = money_envelope(&config, "usd");
        let estimate = build_llm_estimate(&envelope, bound()).expect("estimate");
        // input 1000 * 1.0/M = 1000 micros; output 500 * 2.0/M = 1000 micros.
        assert_eq!(estimate.money_micros, 2_000);
        assert_eq!(estimate.input_tokens, 1_000);
        assert_eq!(estimate.output_tokens, 500);
    }

    #[test]
    fn cache_tokens_use_cache_rates_without_undercounting() {
        let config = pricing();
        let envelope = money_envelope(&config, "usd");
        let mut input = bound();
        input.cache_read_tokens = 400;
        input.cache_write_tokens = 100;
        let estimate = build_llm_estimate(&envelope, input).expect("estimate");
        // base input 500*1.0/M=500 + read 400*0.1/M=40 + write 100*0.2/M=20
        // + output 500*2.0/M=1000 => 1560 micros.
        assert_eq!(estimate.money_micros, 1_560);
    }

    #[test]
    fn reasoning_priced_at_higher_of_reasoning_and_output_rates() {
        let mut config = pricing();
        config.reasoning_per_million = Some(5.0);
        config.reasoning_pricing_mode = "separate".into();
        let envelope = money_envelope(&config, "usd");
        let estimate = build_llm_estimate(&envelope, bound()).expect("estimate");
        // output 500 tokens at max(5.0, 2.0)/M = 2500 micros + input 1000.
        assert_eq!(estimate.money_micros, 3_500);
    }

    #[test]
    fn multiplier_is_applied_with_round_up() {
        let mut config = pricing();
        config.multiplier = 1.5;
        let envelope = money_envelope(&config, "usd");
        let estimate = build_llm_estimate(&envelope, bound()).expect("estimate");
        assert_eq!(estimate.money_micros, 3_000);
    }

    #[test]
    fn token_only_mode_admits_without_pricing() {
        let envelope = token_only_envelope();
        let estimate = build_llm_estimate(&envelope, bound()).expect("estimate");
        assert_eq!(estimate.money_micros, 0);
        assert_eq!(estimate.input_tokens, 1_000);
    }

    #[test]
    fn negative_or_nonfinite_prices_fail_closed() {
        let mut config = pricing();
        config.input_per_million = -1.0;
        assert_eq!(
            pricing_snapshot_from_config("prov", "model", "usd", &config)
                .unwrap_err()
                .code,
            AdmissionErrorCode::UnpricedModel
        );
        let mut config = pricing();
        config.output_per_million = f64::NAN;
        assert_eq!(
            pricing_snapshot_from_config("prov", "model", "usd", &config)
                .unwrap_err()
                .code,
            AdmissionErrorCode::UnpricedModel
        );
        let mut config = pricing();
        config.multiplier = f64::INFINITY;
        assert_eq!(
            pricing_snapshot_from_config("prov", "model", "usd", &config)
                .unwrap_err()
                .code,
            AdmissionErrorCode::UnpricedModel
        );
    }

    #[test]
    fn model_mismatch_fails_closed_with_unpriced_model() {
        let config = pricing();
        let envelope = money_envelope(&config, "usd");
        let mut mismatched = bound();
        mismatched.model_id = "other-model".into();
        assert_eq!(
            build_llm_estimate(&envelope, mismatched).unwrap_err().code,
            AdmissionErrorCode::UnpricedModel
        );
    }

    #[test]
    fn currency_mismatch_fails_closed() {
        let config = pricing();
        // Snapshot built for "usd" but envelope declares "eur".
        let snapshot =
            pricing_snapshot_from_config("prov", "model", "usd", &config).expect("snapshot");
        let cap = 100_000_000u64;
        let envelope = BudgetEnvelope {
            caps: caps(CapLimit::HardCap(cap)),
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::Money {
                currency_code: "eur".into(),
                cap_money_micros: cap,
                pricing: snapshot,
            },
            max_attempts: 1,
            infra_failure_threshold: 3,
            reservation_lease_ms: 60_000,
        };
        assert_eq!(
            build_llm_estimate(&envelope, bound()).unwrap_err().code,
            AdmissionErrorCode::UnpricedModel
        );
    }

    #[test]
    fn missing_output_bound_fails_closed_when_output_is_capped() {
        let config = pricing();
        let envelope = money_envelope(&config, "usd");
        let mut unbounded = bound();
        unbounded.max_output_tokens = None;
        assert_eq!(
            build_llm_estimate(&envelope, unbounded).unwrap_err().code,
            AdmissionErrorCode::MissingBound
        );
    }

    #[test]
    fn output_bound_not_required_when_output_not_capped() {
        let mut envelope = token_only_envelope();
        envelope.caps.output_tokens = CapLimit::NotApplicable;
        let mut unbounded = bound();
        unbounded.max_output_tokens = None;
        let estimate = build_llm_estimate(&envelope, unbounded).expect("estimate");
        assert_eq!(estimate.output_tokens, 0);
    }

    #[test]
    fn snapshot_validation_rejects_incomplete_binding() {
        let config = pricing();
        let snapshot = pricing_snapshot_from_config("", "model", "usd", &config);
        assert_eq!(
            snapshot.unwrap_err().code,
            AdmissionErrorCode::UnpricedModel
        );
    }

    #[test]
    fn pricing_snapshot_type_round_trips() {
        let config = pricing();
        let snapshot =
            pricing_snapshot_from_config("prov", "model", "usd", &config).expect("snapshot");
        let json = serde_json::to_string(&snapshot).expect("serialize");
        let parsed: PricingSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(snapshot, parsed);
    }
}
