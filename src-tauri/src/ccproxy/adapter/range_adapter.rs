//! Range adaptation utilities for converting parameter values between different AI protocols
//!
//! Different AI protocols have different valid ranges for parameters like temperature, top_p, etc.
//! This module provides utilities to safely convert values between these ranges.

use crate::ccproxy::types::ChatProtocol;

/// Protocol-specific parameter ranges
pub struct ParameterRanges;

impl ParameterRanges {
    // Temperature ranges for different protocols
    pub const OPENAI_TEMPERATURE_MIN: f32 = 0.0;
    // While the official OpenAI protocol supports a temperature range of 0.0 to 2.0,
    // many models produce chaotic or nonsensical output as the value approaches 2.0.
    // Additionally, support for the full range varies across different providers.
    // To ensure maximum compatibility and output quality, we cap the maximum at 1.0.
    pub const OPENAI_TEMPERATURE_MAX: f32 = 1.0;

    pub const GENERIC_TEMPERATURE_MIN: f32 = 0.0;
    pub const GENERIC_TEMPERATURE_MAX: f32 = 1.0;

    // Top-p ranges (same for all protocols)
    pub const TOP_P_MIN: f32 = 0.0;
    pub const TOP_P_MAX: f32 = 1.0;

    // Presence/frequency penalty ranges (OpenAI only)
    pub const OPENAI_PENALTY_MIN: f32 = -2.0;
    pub const OPENAI_PENALTY_MAX: f32 = 2.0;
}

/// Adapts temperature value from one protocol to another
pub fn adapt_temperature(value: f32, to_protocol: ChatProtocol) -> f32 {
    let (to_min, to_max) = get_temperature_range(to_protocol);

    // Clamp to source range first
    value.clamp(to_min, to_max)
}

/// Gets temperature range for a specific protocol
fn get_temperature_range(protocol: ChatProtocol) -> (f32, f32) {
    match protocol {
        ChatProtocol::OpenAI => (
            ParameterRanges::OPENAI_TEMPERATURE_MIN,
            ParameterRanges::OPENAI_TEMPERATURE_MAX,
        ),
        _ => (
            ParameterRanges::GENERIC_TEMPERATURE_MIN,
            ParameterRanges::GENERIC_TEMPERATURE_MAX,
        ),
    }
}

/// Clamps a value to the valid range for a specific protocol and parameter
pub fn clamp_to_protocol_range(value: f32, protocol: ChatProtocol, parameter: Parameter) -> f32 {
    let (min, max) = match (protocol, parameter) {
        (ChatProtocol::OpenAI, Parameter::Temperature) => (
            ParameterRanges::OPENAI_TEMPERATURE_MIN,
            ParameterRanges::OPENAI_TEMPERATURE_MAX,
        ),
        (_, Parameter::Temperature) => (
            ParameterRanges::GENERIC_TEMPERATURE_MIN,
            ParameterRanges::GENERIC_TEMPERATURE_MAX,
        ),
        (_, Parameter::TopP) => (ParameterRanges::TOP_P_MIN, ParameterRanges::TOP_P_MAX),
        (ChatProtocol::OpenAI, Parameter::PresencePenalty) => (
            ParameterRanges::OPENAI_PENALTY_MIN,
            ParameterRanges::OPENAI_PENALTY_MAX,
        ),
        (ChatProtocol::OpenAI, Parameter::FrequencyPenalty) => (
            ParameterRanges::OPENAI_PENALTY_MIN,
            ParameterRanges::OPENAI_PENALTY_MAX,
        ),
        _ => return value, // No range restriction for unsupported combinations
    };

    value.clamp(min, max)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Parameter {
    Temperature,
    TopP,
    PresencePenalty,
    FrequencyPenalty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temperature_adaptation() {
        // OpenAI (0-2) to Claude (0-1): value 2.0 should become 1.0
        assert_eq!(adapt_temperature(2.0, ChatProtocol::Claude), 1.0);

        // OpenAI (0-2) to Claude (0-1): value 1.0 should become 0.5
        assert_eq!(adapt_temperature(1.0, ChatProtocol::Claude), 0.5);

        // Claude (0-1) to OpenAI (0-2): value 1.0 should become 2.0
        assert_eq!(adapt_temperature(1.0, ChatProtocol::OpenAI), 2.0);

        // Same protocol should return same value
        assert_eq!(adapt_temperature(1.5, ChatProtocol::OpenAI), 1.5);
    }

    #[test]
    fn test_clamp_to_protocol_range() {
        // OpenAI temperature clamping
        assert_eq!(
            clamp_to_protocol_range(3.0, ChatProtocol::OpenAI, Parameter::Temperature),
            2.0
        );
        assert_eq!(
            clamp_to_protocol_range(-1.0, ChatProtocol::OpenAI, Parameter::Temperature),
            0.0
        );

        // Claude temperature clamping
        assert_eq!(
            clamp_to_protocol_range(2.0, ChatProtocol::Claude, Parameter::Temperature),
            1.0
        );
        assert_eq!(
            clamp_to_protocol_range(-0.5, ChatProtocol::Claude, Parameter::Temperature),
            0.0
        );
    }
}
