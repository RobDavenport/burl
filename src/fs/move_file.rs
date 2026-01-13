//! File move helpers.
//!
//! Burl treats the filesystem as the source of truth for workflow state, which
//! means transitions often need to "move" task files between bucket folders.
//!
//! On POSIX filesystems this is normally an atomic `rename(2)`. Some environments
//! (certain mounts, containers, or cross-volume configs) can surface `EXDEV`
//! ("Invalid cross-device link") even when paths look local. For those cases we
//! fall back to a copy + delete strategy.

use crate::error::{BurlError, Result};
use std::fs;
use std::io;
use std::path::Path;

/// Move a single file from `source` to `destination`.
///
/// - Tries `rename()` first (atomic when possible).
/// - Falls back to an atomic write to `destination` + delete of `source` on EXDEV.
pub fn move_file<P: AsRef<Path>, Q: AsRef<Path>>(source: P, destination: Q) -> Result<()> {
    let source = source.as_ref();
    let destination = destination.as_ref();

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create destination directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }

    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device_rename(&e) => move_file_cross_device(source, destination, e),
        Err(e) => Err(BurlError::UserError(format!(
            "failed to move file '{}' to '{}': {}",
            source.display(),
            destination.display(),
            e
        ))),
    }
}

fn move_file_cross_device(
    source: &Path,
    destination: &Path,
    original_error: io::Error,
) -> Result<()> {
    let content = fs::read(source).map_err(|e| {
        BurlError::UserError(format!(
            "failed to read source file '{}' for cross-device move: {} (original rename error: {})",
            source.display(),
            e,
            original_error
        ))
    })?;

    crate::fs::atomic_write(destination, &content).map_err(|e| {
        BurlError::UserError(format!(
            "failed to write destination file '{}' for cross-device move: {} (original rename error: {})",
            destination.display(),
            e,
            original_error
        ))
    })?;

    fs::remove_file(source).map_err(|e| {
        BurlError::UserError(format!(
            "moved file across devices but failed to delete source file '{}': {}",
            source.display(),
            e
        ))
    })?;

    Ok(())
}

fn is_cross_device_rename(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::CrossesDevices || err.raw_os_error() == Some(18)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn move_file_moves_file_and_creates_parent_dirs() {
        let temp = TempDir::new().unwrap();
        let source_dir = temp.path().join("src");
        std::fs::create_dir_all(&source_dir).unwrap();

        let source = source_dir.join("file.txt");
        std::fs::write(&source, b"hello").unwrap();

        let destination = temp.path().join("dest/nested/file.txt");
        move_file(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
    }

    #[test]
    fn move_file_replaces_existing_destination_file() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("destination.txt");

        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&destination, b"old").unwrap();

        move_file(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    }
}
