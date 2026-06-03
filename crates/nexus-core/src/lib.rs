pub mod adapter;
pub mod agent;
pub mod error;
pub mod task;

pub use adapter::Adapter;
pub use agent::{Agent, AgentConfig, AgentMetrics, AgentStatus, AgentType};
pub use error::NexusError;
pub use task::{Task, TaskSpec, TaskStatus};

pub type Result<T> = std::result::Result<T, NexusError>;
