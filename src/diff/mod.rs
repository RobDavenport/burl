//! Diff parsing primitives for burl.
//!
//! This module provides utilities for parsing git diff output to support:
//! - `submit` validation (scope checking)
//! - `validate` stub detection (added lines only)
//! - `approve` validation after rebase
//!
//! The parsing is deterministic and supports:
//! - Changed files list from `git diff --name-only {base}..HEAD`
//! - Added lines with line numbers from `git diff -U0 {base}..HEAD`
//! - New files (from /dev/null)
//! - File renames (best-effort line mapping)
//! - Proper hunk header parsing for accurate line numbers

mod api;
mod helpers;
mod parser;

#[cfg(test)]
mod tests;

// Re-export public API
pub use api::{added_lines, changed_files, AddedLine};
