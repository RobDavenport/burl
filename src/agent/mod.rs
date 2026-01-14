//! Agent execution subsystem for burl V2.
//!
//! This module provides the core agent orchestration capabilities:
//!
//! - **Config**: Agent profiles and configuration (`agents.yaml`)
//! - **Prompt**: Prompt generation from task context
//! - **Dispatch**: Subprocess execution with timeout and output capture
//! - **Binding**: Task-agent assignment resolution
//!
//! # Design Philosophy
//!
//! Agents are dispatched as subprocesses with configurable command templates.
//! This design supports any CLI-based agent tool (Claude Code, Crush, opencode, etc.)
//! without coupling to a specific protocol like MCP.
//!
//! Validation remains deterministic - agents do not judge other agents' work.
//! The existing scope/stub/build validation gates are unchanged.

mod binding;
mod config;
pub mod dispatch;
pub mod prompt;

// Re-export public API
pub use binding::{AgentBinding, BindingSource, resolve_agent};
pub use config::{AgentProfile, AgentsConfig};
pub use dispatch::{AgentResult, execute_agent};
