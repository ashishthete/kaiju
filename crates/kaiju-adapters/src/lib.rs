mod binary;
pub mod claude;
mod claude_events;
mod claude_transcript;
pub mod codex;
pub mod custom;
pub mod gemini;
pub mod limit;
mod pricing;
mod registry;
mod shell;
mod trust;

pub use registry::AdapterRegistry;
