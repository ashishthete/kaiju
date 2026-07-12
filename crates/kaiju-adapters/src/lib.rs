mod binary;
pub mod claude;
pub mod limit;
mod claude_events;
mod claude_transcript;
pub mod codex;
pub mod custom;
pub mod gemini;
mod pricing;
mod registry;
mod trust;

pub use registry::AdapterRegistry;
