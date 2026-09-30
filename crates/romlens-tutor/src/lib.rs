//! Romlens's tutor (docs/24): a chat over everything Romlens computes, on
//! Anthropic, OpenAI, or an endpoint that speaks OpenAI's protocol on the
//! user's own machine. The core stays free of networking; this crate holds
//! the transcript, the three wire protocols, the model table, the tool loop
//! and the conversation store, and the FFI and the CLI share it.

pub mod agent;
pub mod http;
pub mod images;
pub mod lesson;
pub mod models;
pub mod progress;
pub mod prompt;
pub mod provider;
pub mod quiz;
pub mod review;
pub mod sse;
pub mod store;
pub mod transcript;

pub use models::{Capabilities, ModelInfo, Price, Thinking};
pub use provider::{Delta, Endpoint, HttpRequest, Protocol, Reply, Request, Stop, ToolSpec};
pub use transcript::{Block, ImageRef, Native, Part, Role, Turn, Usage};
