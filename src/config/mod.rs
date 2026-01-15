//! Configuration model for burl.
//!
//! This module defines the Config struct that represents `.burl/.workflow/config.yaml`.
//! It supports forward-compatible YAML parsing (unknown fields are ignored),
//! sensible defaults for optional fields, and validation of config values.

mod model;
mod operations;
pub mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use model::Config;
pub use types::{
    ConflictDetectionMode, ConflictPolicy, MergeStrategy, ValidationCommandStep, ValidationProfile,
};
