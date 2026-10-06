use crate::db::PricingConfig;

const TOKENS_PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsageBreakdown {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub audio_input_tokens: i64,
    pub audio_output_tokens: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostBreakdown {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub reasoning: f64,
    pub audio_input: f64,
    pub audio_output: f64,
    pub total: f64,
}

fn non_negative(value: i64) -> f64 {
    value.max(0) as f64
}

pub fn calculate_cost(usage: UsageBreakdown, pricing: &PricingConfig) -> CostBreakdown {
    let input = non_negative(usage.input_tokens);
    let output = non_negative(usage.output_tokens);
    let cache_read = non_negative(usage.cache_read_tokens).min(input);
    let cache_write = non_negative(usage.cache_write_tokens).min((input - cache_read).max(0.0));
    let audio_input =
        non_negative(usage.audio_input_tokens).min((input - cache_read - cache_write).max(0.0));
    let reasoning = non_negative(usage.reasoning_tokens).min(output);
    let audio_output = non_negative(usage.audio_output_tokens).min((output - reasoning).max(0.0));
    let input_base = (input - cache_read - cache_write - audio_input).max(0.0);
    let output_base = (output - reasoning - audio_output).max(0.0);
    let multiplier = if pricing.multiplier.is_finite() && pricing.multiplier >= 0.0 {
        pricing.multiplier
    } else {
        1.0
    };
    let tier = pricing
        .tiers
        .iter()
        .filter(|tier| tier.context_size <= input as u64)
        .max_by_key(|tier| tier.context_size);
    let reasoning_rate = if pricing.reasoning_pricing_mode == "separate" {
        tier.and_then(|value| value.reasoning_per_million)
            .or_else(|| {
                pricing
                    .reasoning_per_million
                    .filter(|value| value.is_finite() && *value >= 0.0)
            })
            .unwrap_or(pricing.output_per_million)
    } else {
        tier.map(|value| value.output_per_million)
            .unwrap_or(pricing.output_per_million)
    };
    let cache_write_rate = tier
        .and_then(|value| {
            (value.cache_write_per_million > 0.0).then_some(value.cache_write_per_million)
        })
        .unwrap_or_else(|| {
            if pricing.cache_write_per_million > 0.0 {
                pricing.cache_write_per_million
            } else {
                pricing.input_per_million
            }
        });
    let audio_input_rate = tier
        .and_then(|value| {
            (value.audio_input_per_million > 0.0).then_some(value.audio_input_per_million)
        })
        .unwrap_or_else(|| {
            if pricing.audio_input_per_million > 0.0 {
                pricing.audio_input_per_million
            } else {
                pricing.input_per_million
            }
        });
    let audio_output_rate = tier
        .and_then(|value| {
            (value.audio_output_per_million > 0.0).then_some(value.audio_output_per_million)
        })
        .unwrap_or_else(|| {
            if pricing.audio_output_per_million > 0.0 {
                pricing.audio_output_per_million
            } else {
                pricing.output_per_million
            }
        });
    let tier = pricing
        .tiers
        .iter()
        .filter(|tier| tier.context_size <= input as u64)
        .max_by_key(|tier| tier.context_size);
    let input_rate = tier
        .map(|tier| tier.input_per_million)
        .unwrap_or(pricing.input_per_million);
    let output_rate = tier
        .map(|tier| tier.output_per_million)
        .unwrap_or(pricing.output_per_million);
    let cache_rate = tier
        .map(|tier| tier.cache_per_million)
        .unwrap_or(pricing.cache_per_million);
    let result = CostBreakdown {
        input: input_base * input_rate / TOKENS_PER_MILLION,
        output: output_base * output_rate / TOKENS_PER_MILLION,
        cache_read: cache_read * cache_rate / TOKENS_PER_MILLION,
        cache_write: cache_write * cache_write_rate / TOKENS_PER_MILLION,
        reasoning: reasoning * reasoning_rate / TOKENS_PER_MILLION,
        audio_input: audio_input * audio_input_rate / TOKENS_PER_MILLION,
        audio_output: audio_output * audio_output_rate / TOKENS_PER_MILLION,
        total: 0.0,
    };
    CostBreakdown {
        total: (result.input
            + result.output
            + result.cache_read
            + result.cache_write
            + result.reasoning
            + result.audio_input
            + result.audio_output)
            * multiplier,
        ..result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialized_tokens_are_not_double_counted() {
        let pricing = PricingConfig {
            input_per_million: 1.0,
            output_per_million: 2.0,
            cache_per_million: 0.1,
            cache_write_per_million: 0.2,
            reasoning_per_million: Some(3.0),
            reasoning_pricing_mode: "separate".into(),
            audio_input_per_million: 4.0,
            audio_output_per_million: 5.0,
            multiplier: 1.0,
            pricing_source: Some("test".into()),
            tiers: Vec::new(),
        };
        let cost = calculate_cost(
            UsageBreakdown {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 20,
                cache_write_tokens: 10,
                reasoning_tokens: 15,
                audio_input_tokens: 5,
                audio_output_tokens: 10,
            },
            &pricing,
        );
        assert!((cost.total - 0.000234).abs() < 1e-12);
    }

    #[test]
    fn missing_reasoning_price_follows_output_price() {
        let pricing = PricingConfig {
            input_per_million: 0.0,
            output_per_million: 4.0,
            reasoning_pricing_mode: "output".into(),
            multiplier: 1.0,
            ..Default::default()
        };
        let cost = calculate_cost(
            UsageBreakdown {
                output_tokens: 100,
                reasoning_tokens: 100,
                ..Default::default()
            },
            &pricing,
        );
        assert!((cost.total - 0.0004).abs() < 1e-12);
    }
    #[test]
    fn malformed_or_overlapping_details_are_clamped() {
        let pricing = PricingConfig {
            input_per_million: 1.0,
            output_per_million: 1.0,
            multiplier: 1.0,
            ..Default::default()
        };
        let cost = calculate_cost(
            UsageBreakdown {
                input_tokens: 2,
                output_tokens: 2,
                cache_read_tokens: 8,
                cache_write_tokens: 8,
                reasoning_tokens: 8,
                audio_input_tokens: 8,
                audio_output_tokens: 8,
            },
            &pricing,
        );
        assert_eq!(cost.total, 0.000002);
    }
}
