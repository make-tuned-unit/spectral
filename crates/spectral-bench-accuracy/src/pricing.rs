//! Model pricing table for cost estimation.
//!
//! Source: Anthropic API pricing page, 2025-01-27.
//! <https://docs.anthropic.com/en/docs/about-claude/models#model-comparison-table>
//!
//! Prices are USD per million tokens (input, output).
//! Unknown model IDs produce `None` — never silently zero.

/// (input $/MTok, output $/MTok)
const PRICING: &[(&str, f64, f64)] = &[
    // Claude 4.6 / 4.5 family
    ("claude-opus-4-6", 15.0, 75.0),
    ("claude-sonnet-4-6", 3.0, 15.0),
    ("claude-haiku-4-5-20251001", 0.80, 4.0),
    // Claude 3.5 family
    ("claude-3-5-sonnet-20241022", 3.0, 15.0),
    ("claude-3-5-haiku-20241022", 0.80, 4.0),
    // Claude 3 family
    ("claude-3-opus-20240229", 15.0, 75.0),
    ("claude-3-sonnet-20240229", 3.0, 15.0),
    ("claude-3-haiku-20240307", 0.25, 1.25),
];

/// Look up pricing for a model ID. Returns `(input_$/MTok, output_$/MTok)`.
fn lookup(model: &str) -> Option<(f64, f64)> {
    PRICING
        .iter()
        .find(|(id, _, _)| *id == model)
        .map(|(_, inp, out)| (*inp, *out))
}

/// Estimate cost in USD for a single API call.
///
/// Returns `None` if the model ID is unknown — caller must record null cost,
/// never silently 0.
pub fn estimate_call_cost(
    model: &str,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
) -> Option<f64> {
    let (inp_rate, out_rate) = lookup(model)?;
    let inp = input_tokens.unwrap_or(0) as f64;
    let out = output_tokens.unwrap_or(0) as f64;
    Some(inp * inp_rate / 1_000_000.0 + out * out_rate / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_returns_cost() {
        let cost = estimate_call_cost("claude-sonnet-4-6", Some(1000), Some(500));
        // 1000 * 3.0/1M + 500 * 15.0/1M = 0.003 + 0.0075 = 0.0105
        assert!((cost.unwrap() - 0.0105).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(estimate_call_cost("local-llama", Some(1000), Some(500)).is_none());
    }

    #[test]
    fn haiku_cheaper_than_sonnet() {
        let haiku =
            estimate_call_cost("claude-haiku-4-5-20251001", Some(1000), Some(1000)).unwrap();
        let sonnet = estimate_call_cost("claude-sonnet-4-6", Some(1000), Some(1000)).unwrap();
        assert!(haiku < sonnet);
    }
}
