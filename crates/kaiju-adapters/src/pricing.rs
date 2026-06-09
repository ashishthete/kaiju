//! Optional, user-supplied token pricing for estimating session cost.
//!
//! Kaiju ships **no** prices — they change and would silently go stale. To show
//! a cost, drop a JSON file at `~/.kaiju/pricing.json` (override with
//! `KAIJU_PRICING`) mapping a model id to its per-million-token rates:
//!
//! ```json
//! {
//!   "claude-opus-4-8":   { "input": 5,  "output": 25, "cache_write": 6.25, "cache_read": 0.5 },
//!   "claude-sonnet-4-6": { "input": 3,  "output": 15, "cache_write": 3.75, "cache_read": 0.3 }
//! }
//! ```
//!
//! Cost is shown only for models the file covers; everything else stays blank.
//! Edits are picked up on daemon restart (the file is read once per process).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::claude_transcript::Usage;

/// Per-million-token rates for one model. `cache_*` default to 0 when omitted.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default)]
    pub cache_read: f64,
}

type PricingTable = HashMap<String, ModelPricing>;

fn pricing_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KAIJU_PRICING") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kaiju").join("pricing.json"))
}

/// Read and parse the pricing file; an empty table when absent or malformed
/// (so a typo in the optional file disables cost rather than crashing).
fn load_table() -> PricingTable {
    let Some(path) = pricing_path() else {
        return PricingTable::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return PricingTable::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Process-wide pricing table, loaded once (restart to pick up edits).
static TABLE: LazyLock<PricingTable> = LazyLock::new(load_table);

/// Resolve a model's rates: exact match, else the longest table key that is a
/// prefix of the model id (so a dated variant like `claude-haiku-4-5-20251001`
/// matches a `claude-haiku-4-5` entry).
fn rates_for<'a>(table: &'a PricingTable, model: &str) -> Option<&'a ModelPricing> {
    if let Some(pricing) = table.get(model) {
        return Some(pricing);
    }
    table
        .iter()
        .filter(|(key, _)| model.starts_with(key.as_str()))
        .max_by_key(|(key, _)| key.len())
        .map(|(_, pricing)| pricing)
}

/// Pure: estimate USD cost for `usage` under `table`. `None` when the usage has
/// no model or the model isn't priced.
pub fn estimate_cost(usage: &Usage, table: &PricingTable) -> Option<f64> {
    let model = usage.model.as_deref()?;
    let rates = rates_for(table, model)?;
    let per_million = 1_000_000.0;
    Some(
        usage.input_tokens as f64 * rates.input / per_million
            + usage.output_tokens as f64 * rates.output / per_million
            + usage.cache_creation_tokens as f64 * rates.cache_write / per_million
            + usage.cache_read_tokens as f64 * rates.cache_read / per_million,
    )
}

/// Estimate cost using the configured pricing file. `None` when no file is
/// present or the session's model isn't priced.
pub fn cost(usage: &Usage) -> Option<f64> {
    estimate_cost(usage, &TABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> PricingTable {
        let mut t = PricingTable::new();
        t.insert(
            "claude-opus-4-8".to_string(),
            ModelPricing {
                input: 5.0,
                output: 25.0,
                cache_write: 6.25,
                cache_read: 0.5,
            },
        );
        t
    }

    #[test]
    fn estimate_cost_sums_each_category() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_creation_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            model: Some("claude-opus-4-8".to_string()),
        };
        // 5 + 25 + 6.25 + 0.5 per million of each.
        assert_eq!(estimate_cost(&usage, &table()), Some(36.75));
    }

    #[test]
    fn prefix_matches_dated_model_variant() {
        let usage = Usage {
            output_tokens: 1_000_000,
            model: Some("claude-opus-4-8-20251101".to_string()),
            ..Usage::default()
        };
        assert_eq!(estimate_cost(&usage, &table()), Some(25.0));
    }

    #[test]
    fn unknown_or_missing_model_has_no_cost() {
        let unpriced = Usage {
            output_tokens: 10,
            model: Some("some-other-model".to_string()),
            ..Usage::default()
        };
        assert_eq!(estimate_cost(&unpriced, &table()), None);

        let no_model = Usage {
            output_tokens: 10,
            ..Usage::default()
        };
        assert_eq!(estimate_cost(&no_model, &table()), None);
    }

    #[test]
    fn empty_table_yields_no_cost() {
        let usage = Usage {
            output_tokens: 10,
            model: Some("claude-opus-4-8".to_string()),
            ..Usage::default()
        };
        assert_eq!(estimate_cost(&usage, &PricingTable::new()), None);
    }
}
