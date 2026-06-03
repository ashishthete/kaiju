pub mod agent;
pub mod error;
pub mod adapter;

pub use agent::{Agent, AgentConfig, AgentMetrics, AgentStatus, AgentType};
pub use adapter::Adapter;
pub use error::NexusError;

pub type Result<T> = std::result::Result<T, NexusError>;
