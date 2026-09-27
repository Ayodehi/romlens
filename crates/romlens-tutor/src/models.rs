//! The one table of models: what each can do and what it costs (docs/24,
//! decision 9). A provider's own model list says which ids exist; only this
//! table says what they take. Checked against the providers' documentation
//! on 27 September 2026 (platform.claude.com models and pricing;
//! developers.openai.com models and pricing). Check again before relying on
//! a price.

use crate::provider::Protocol;
use crate::transcript::Usage;

/// How a model is asked to think.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thinking {
    /// Anthropic's adaptive thinking, with a summary shown.
    Adaptive,
    /// OpenAI's reasoning, with a summary.
    Reasoning,
    /// Not asked for.
    None,
}

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Price {
    pub fn cost(&self, u: &Usage) -> f64 {
        (u.input as f64 * self.input
            + u.output as f64 * self.output
            + u.cache_read as f64 * self.cache_read
            + u.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInfo {
    pub id: &'static str,
    pub protocol: Protocol,
    pub name: &'static str,
    /// Tokens in, and the most out.
    pub context: u32,
    pub max_output: u32,
    pub vision: bool,
    pub thinking: Thinking,
    /// The effort levels it takes, lowest first; empty when it takes none.
    pub efforts: &'static [&'static str],
    pub default_effort: Option<&'static str>,
    pub price: Price,
    /// Anthropic: a refusal is retried on a fallback model the API picks
    /// (`fallbacks: "default"`).
    pub fallbacks: bool,
}

const ANTHROPIC_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const GPT6_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const GPT6_SMALL_EFFORTS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "claude-opus-5",
        protocol: Protocol::Anthropic,
        name: "Claude Opus 5",
        context: 1_000_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Adaptive,
        efforts: ANTHROPIC_EFFORTS,
        default_effort: Some("high"),
        price: Price {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
        fallbacks: true,
    },
    ModelInfo {
        id: "claude-opus-5-5",
        protocol: Protocol::Anthropic,
        name: "Claude Opus 5.5",
        context: 1_000_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Adaptive,
        efforts: ANTHROPIC_EFFORTS,
        default_effort: Some("high"),
        price: Price {
            input: 4.0,
            output: 20.0,
            cache_read: 0.2,
            cache_write: 5.0,
        },
        fallbacks: true,
    },
    ModelInfo {
        id: "claude-fable-5-1",
        protocol: Protocol::Anthropic,
        name: "Claude Fable 5.1",
        context: 1_000_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Adaptive,
        efforts: ANTHROPIC_EFFORTS,
        default_effort: Some("high"),
        price: Price {
            input: 10.0,
            output: 50.0,
            cache_read: 0.25,
            cache_write: 12.5,
        },
        fallbacks: true,
    },
    ModelInfo {
        id: "claude-sonnet-5",
        protocol: Protocol::Anthropic,
        name: "Claude Sonnet 5",
        context: 1_000_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Adaptive,
        efforts: ANTHROPIC_EFFORTS,
        default_effort: Some("high"),
        price: Price {
            input: 2.0,
            output: 10.0,
            cache_read: 0.2,
            cache_write: 2.5,
        },
        fallbacks: false,
    },
    ModelInfo {
        id: "claude-haiku-4-5",
        protocol: Protocol::Anthropic,
        name: "Claude Haiku 4.5",
        context: 200_000,
        max_output: 64_000,
        vision: true,
        thinking: Thinking::None,
        efforts: &[],
        default_effort: None,
        price: Price {
            input: 1.0,
            output: 5.0,
            cache_read: 0.1,
            cache_write: 1.25,
        },
        fallbacks: false,
    },
    ModelInfo {
        id: "gpt-6-astra",
        protocol: Protocol::Responses,
        name: "GPT-6 Astra",
        context: 1_050_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Reasoning,
        efforts: GPT6_EFFORTS,
        default_effort: Some("medium"),
        price: Price {
            input: 10.0,
            output: 50.0,
            cache_read: 1.0,
            cache_write: 12.5,
        },
        fallbacks: false,
    },
    ModelInfo {
        id: "gpt-6-sol",
        protocol: Protocol::Responses,
        name: "GPT-6 Sol",
        context: 1_050_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Reasoning,
        efforts: GPT6_SMALL_EFFORTS,
        default_effort: Some("medium"),
        price: Price {
            input: 2.0,
            output: 10.0,
            cache_read: 0.2,
            cache_write: 2.5,
        },
        fallbacks: false,
    },
    ModelInfo {
        id: "gpt-6-luna",
        protocol: Protocol::Responses,
        name: "GPT-6 Luna",
        context: 1_050_000,
        max_output: 128_000,
        vision: true,
        thinking: Thinking::Reasoning,
        efforts: GPT6_SMALL_EFFORTS,
        default_effort: Some("medium"),
        price: Price {
            input: 0.1,
            output: 0.5,
            cache_read: 0.01,
            cache_write: 0.125,
        },
        fallbacks: false,
    },
];

/// The model a new conversation starts on, per protocol.
pub fn default_model(p: Protocol) -> Option<&'static str> {
    match p {
        Protocol::Anthropic => Some("claude-opus-5"),
        Protocol::Responses => Some("gpt-6-astra"),
        Protocol::Chat => None,
    }
}

pub fn model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// What is known of a model that may not be in the table (a local one):
/// its context and output limits fall back to modest guesses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capabilities {
    pub context: u32,
    pub max_output: u32,
    pub vision: bool,
    pub thinking: Thinking,
    pub efforts: &'static [&'static str],
    pub default_effort: Option<&'static str>,
    /// `None` for a local model: nothing to pay.
    pub price: Option<Price>,
    pub fallbacks: bool,
}

pub fn capabilities(id: &str, vision_fallback: bool) -> Capabilities {
    match model(id) {
        Some(m) => Capabilities {
            context: m.context,
            max_output: m.max_output,
            vision: m.vision,
            thinking: m.thinking,
            efforts: m.efforts,
            default_effort: m.default_effort,
            price: Some(m.price),
            fallbacks: m.fallbacks,
        },
        None => Capabilities {
            context: 32_768,
            max_output: 8_192,
            vision: vision_fallback,
            thinking: Thinking::None,
            efforts: &[],
            default_effort: None,
            price: None,
            fallbacks: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_whole() {
        for m in MODELS {
            if let Some(e) = m.default_effort {
                assert!(m.efforts.contains(&e), "{}", m.id);
            }
            assert!(m.max_output < m.context, "{}", m.id);
        }
        for p in [Protocol::Anthropic, Protocol::Responses] {
            assert_eq!(model(default_model(p).unwrap()).unwrap().protocol, p);
        }
    }

    #[test]
    fn cost_is_per_million() {
        let u = Usage {
            input: 1_000_000,
            output: 100_000,
            cache_read: 2_000_000,
            cache_write: 0,
        };
        let c = model("claude-opus-5").unwrap().price.cost(&u);
        assert!((c - (5.0 + 2.5 + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn an_unknown_model_is_local() {
        let c = capabilities("qwen3:32b", true);
        assert!(c.price.is_none());
        assert!(c.vision);
    }
}
