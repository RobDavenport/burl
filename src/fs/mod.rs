//! Filesystem utilities for burl.
//!
//! This module provides safe filesystem operations, particularly atomic writes
//! that are essential for maintaining workflow state integrity.

pub mod atomic;

pub use atomic::atomic_write;
pub use atomic::atomic_write_file;
