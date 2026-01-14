//! Agent subprocess dispatch and execution.
//!
//! This module provides subprocess execution for agents with:
//!
//! - Command template variable substitution
//! - Configurable timeout with process termination
//! - Output capture to log files
//! - Environment variable merging
//! - Cross-platform support

mod executor;

pub use executor::{AgentResult, execute_agent};
