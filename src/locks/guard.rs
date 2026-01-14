//! RAII lock guard implementation.

use crate::error::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// RAII guard for a lock file.
///
/// When dropped, the lock file is automatically deleted.
/// If deletion fails, a warning is printed but no panic occurs.
#[derive(Debug)]
pub struct LockGuard {
    /// Path to the lock file.
    path: PathBuf,

    /// Whether the lock has been released manually.
    released: bool,
}

impl LockGuard {
    /// Create a new lock guard for the given path.
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            released: false,
        }
    }

    /// Get the path to the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Manually release the lock.
    ///
    /// This is useful when you want to release the lock before the guard
    /// goes out of scope, and want to handle errors explicitly.
    pub fn release(mut self) -> Result<()> {
        use crate::error::BurlError;

        self.released = true;
        fs::remove_file(&self.path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to release lock '{}': {}",
                self.path.display(),
                e
            ))
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.released
            && let Err(e) = fs::remove_file(&self.path)
        {
            eprintln!(
                "Warning: failed to release lock '{}': {}",
                self.path.display(),
                e
            );
        }
    }
}
