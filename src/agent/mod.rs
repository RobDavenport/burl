//! Agent execution subsystem.

mod binding;
mod config;
pub mod dispatch;
pub mod prompt;

// Re-export public API
pub use binding::{AgentBinding, BindingSource, resolve_agent};
pub use config::{AgentProfile, AgentsConfig};
pub use dispatch::{AgentResult, execute_agent};
