use crate::ai::model_catalog::pricing::{calculate_cost, UsageBreakdown};
use serde::{Deserialize, Serialize};

/// Immutable, snake_case usage summary persisted at a workflow task terminal boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowUsageSummary {
    pub version: u32,
    pub terminal_status: String,
    pub duration_ms: Option<i64>,
    pub self_usage: UsageTotals,
    pub with_sub_agents: UsageTotals,
    pub has_sub_agents: bool,
    pub is_partial: bool,
    pub model_breakdowns: Vec<ModelUsageBreakdown>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub audio_input_tokens: i64,
    #[serde(default)]
    pub audio_output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost: Option<f64>,
    pub effective_cost_per_million: Option<f64>,
    pub unpriced_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelUsageBreakdown {
    pub provider_id: Option<i64>,
    pub backend_model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub audio_input_tokens: i64,
    #[serde(default)]
    pub audio_output_tokens: i64,
    pub pricing_status: String,
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub cache_per_million: Option<f64>,
    pub reasoning_per_million: Option<f64>,
    pub multiplier: Option<f64>,
    pub estimated_cost: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct PricingSnapshot {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_per_million: f64,
    pub multiplier: f64,
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn calculate_model_cost(
    input_tokens: i64,
    output_tokens: i64,
    cache_tokens: i64,
    pricing: PricingSnapshot,
) -> f64 {
    calculate_cost(
        UsageBreakdown {
            input_tokens,
            output_tokens,
            cache_read_tokens: cache_tokens,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            audio_input_tokens: 0,
            audio_output_tokens: 0,
        },
        &crate::db::PricingConfig {
            input_per_million: pricing.input_per_million,
            output_per_million: pricing.output_per_million,
            cache_per_million: pricing.cache_per_million,
            reasoning_per_million: None,
            reasoning_pricing_mode: "output".into(),
            multiplier: pricing.multiplier,
            ..Default::default()
        },
    )
    .total
}

impl UsageTotals {
    pub fn from_breakdowns(breakdowns: &[ModelUsageBreakdown]) -> Self {
        let input_tokens: i64 = breakdowns.iter().map(|item| item.input_tokens.max(0)).sum();
        let output_tokens: i64 = breakdowns
            .iter()
            .map(|item| item.output_tokens.max(0))
            .sum();
        let cache_tokens: i64 = breakdowns.iter().map(|item| item.cache_tokens.max(0)).sum();
        let cache_write_tokens: i64 = breakdowns
            .iter()
            .map(|item| item.cache_write_tokens.max(0))
            .sum();
        let reasoning_tokens: i64 = breakdowns
            .iter()
            .map(|item| item.reasoning_tokens.max(0))
            .sum();
        let audio_input_tokens: i64 = breakdowns
            .iter()
            .map(|item| item.audio_input_tokens.max(0))
            .sum();
        let audio_output_tokens: i64 = breakdowns
            .iter()
            .map(|item| item.audio_output_tokens.max(0))
            .sum();
        let total_tokens = input_tokens + output_tokens;
        let unpriced_tokens = breakdowns
            .iter()
            .filter(|item| item.pricing_status != "priced")
            .map(|item| item.input_tokens.max(0) + item.output_tokens.max(0))
            .sum();
        let estimated_cost = if unpriced_tokens == 0 {
            Some(
                breakdowns
                    .iter()
                    .filter_map(|item| item.estimated_cost)
                    .sum(),
            )
        } else {
            None
        };
        let effective_cost_per_million = estimated_cost
            .filter(|_| total_tokens > 0)
            .map(|cost| cost / total_tokens as f64 * 1_000_000.0);

        Self {
            input_tokens,
            output_tokens,
            cache_tokens,
            cache_write_tokens,
            reasoning_tokens,
            audio_input_tokens,
            audio_output_tokens,
            total_tokens,
            estimated_cost,
            effective_cost_per_million,
            unpriced_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_usage_cost_matches_the_confirmed_cached_input_formula() {
        let cost = calculate_model_cost(
            1_500_000,
            250_000,
            500_000,
            PricingSnapshot {
                input_per_million: 2.0,
                cache_per_million: 0.2,
                output_per_million: 8.0,
                multiplier: 1.5,
            },
        );
        assert!((cost - 6.15).abs() < 1e-9);
    }

    #[test]
    fn totals_do_not_count_cache_tokens_twice() {
        let totals = UsageTotals::from_breakdowns(&[ModelUsageBreakdown {
            provider_id: Some(1),
            backend_model: "model".to_string(),
            input_tokens: 100,
            output_tokens: 25,
            cache_tokens: 40,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            audio_input_tokens: 0,
            audio_output_tokens: 0,
            pricing_status: "priced".to_string(),
            input_per_million: Some(1.0),
            output_per_million: Some(2.0),
            cache_per_million: Some(0.1),
            reasoning_per_million: None,
            multiplier: Some(1.0),
            estimated_cost: Some(0.000145),
        }]);

        assert_eq!(totals.total_tokens, 125);
        assert_eq!(totals.cache_tokens, 40);
    }
}
