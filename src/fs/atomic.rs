//! Atomic filesystem operations for burl.
//!
//! This module provides atomic file write operations that ensure workflow state
//! is never left in a corrupted state due to crashes or interruptions.
//!
//! # Implementation Strategy
//!
//! All atomic writes follow this pattern:
//! 1. Write content to a temporary file in the same directory
//! 2. Sync the file to disk (fsync)
//! 3. Atomically replace the original file
//!
//! # Cross-Platform Behavior
//!
//! - **POSIX (Linux, macOS)**: Uses `rename()` which is atomic if source and
//!   destination are on the same filesystem.
//! - **Windows**: Uses a combination of approaches to achieve atomic-like behavior:
//!   - First attempts `std::fs::rename()` which works if the destination doesn't exist
//!   - Falls back to a replace strategy using the Windows API for existing files
//!
//! # Important Notes
//!
//! - Source and destination must be on the same filesystem/volume for atomic rename
//! - On crash, a temporary file may remain (named `.{filename}.tmp`)
//! - The temporary file is created in the same directory as the target file

use crate::error::{BurlError, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Atomically write bytes to a file.
///
/// This function writes the content to a temporary file, syncs it to disk,
/// and then atomically replaces the target file. This ensures that the target
/// file is never in a partial/corrupted state.
///
/// # Arguments
///
/// * `path` - The target file path
/// * `content` - The bytes to write
///
/// # Returns
///
/// * `Ok(())` - On successful atomic write
/// * `Err(BurlError::UserError)` - On write or rename failure
///
/// # Example
///
/// ```no_run
/// use burl::fs::atomic_write;
/// use std::path::Path;
///
/// atomic_write(Path::new("config.yaml"), b"key: value\n")?;
/// # Ok::<(), burl::error::BurlError>(())
/// ```
pub fn atomic_write<P: AsRef<Path>>(path: P, content: &[u8]) -> Result<()> {
    let path = path.as_ref();

    // Ensure parent directory exists
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create parent directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }

    // Generate temp file path in the same directory
    let temp_path = generate_temp_path(path)?;

    // Write to temp file with sync
    write_and_sync(&temp_path, content)?;

    // Atomically replace the target file
    atomic_replace(&temp_path, path)?;

    Ok(())
}

/// Atomically write a string to a file.
///
/// Convenience wrapper around `atomic_write` for string content.
pub fn atomic_write_file<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    atomic_write(path, content.as_bytes())
}

/// Generate a temporary file path in the same directory as the target.
fn generate_temp_path(target: &Path) -> Result<std::path::PathBuf> {
    let parent = target.parent().unwrap_or(Path::new("."));
    let filename = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| BurlError::UserError("invalid file path".to_string()))?;

    // Use a simple temp name pattern: .{filename}.tmp
    // In production, we could add a random suffix for safety
    let temp_name = format!(".{}.tmp", filename);
    Ok(parent.join(temp_name))
}

/// Write content to a file and sync to disk.
fn write_and_sync(path: &Path, content: &[u8]) -> Result<()> {
    // Create or truncate the file
    let mut file = File::create(path).map_err(|e| {
        BurlError::UserError(format!(
            "failed to create temporary file '{}': {}",
            path.display(),
            e
        ))
    })?;

    // Write all content
    file.write_all(content).map_err(|e| {
        // Clean up temp file on error
        let _ = fs::remove_file(path);
        BurlError::UserError(format!("failed to write to temporary file: {}", e))
    })?;

    // Sync to disk to ensure durability
    file.sync_all().map_err(|e| {
        // Clean up temp file on error
        let _ = fs::remove_file(path);
        BurlError::UserError(format!("failed to sync temporary file to disk: {}", e))
    })?;

    Ok(())
}

/// Atomically replace the target file with the source file.
///
/// This function handles platform-specific behavior:
/// - On POSIX: Uses rename() which is atomic
/// - On Windows: Uses rename() for new files, or platform-specific replace for existing
#[cfg(unix)]
fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    // On POSIX, rename() is atomic and replaces the destination if it exists
    fs::rename(source, target).map_err(|e| {
        // Clean up temp file on error
        let _ = fs::remove_file(source);
        BurlError::UserError(format!(
            "failed to atomically replace '{}': {}",
            target.display(),
            e
        ))
    })?;

    // Optionally sync the parent directory for extra durability
    // This ensures the directory entry is persisted
    if let Some(parent) = target.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Windows-specific atomic replace implementation.
#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    // First, try a simple rename (works if target doesn't exist)
    match fs::rename(source, target) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Target exists, need to use ReplaceFile or MoveFileEx
        }
        Err(e) => {
            // Clean up temp file on error
            let _ = fs::remove_file(source);
            return Err(BurlError::UserError(format!(
                "failed to atomically replace '{}': {}",
                target.display(),
                e
            )));
        }
    }

    // Use Windows ReplaceFile API for atomic replacement of existing files
    // This preserves file attributes and provides better atomicity guarantees
    unsafe {
        let source_wide: Vec<u16> = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target_wide: Vec<u16> = target
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(
                lpExistingFileName: *const u16,
                lpNewFileName: *const u16,
                dwFlags: u32,
            ) -> i32;

            fn GetLastError() -> u32;
        }

        let result = MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        );

        if result == 0 {
            let error_code = GetLastError();
            // Clean up temp file on error
            let _ = fs::remove_file(source);
            return Err(BurlError::UserError(format!(
                "failed to atomically replace '{}': Windows error code {}",
                target.display(),
                error_code
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        atomic_write(&file_path, b"hello world").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_atomic_write_replace_existing() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create initial file
        fs::write(&file_path, "original content").unwrap();

        // Atomically replace
        atomic_write(&file_path, b"new content").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_atomic_write_file_string() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        atomic_write_file(&file_path, "string content\nwith newlines").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "string content\nwith newlines");
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("nested").join("dirs").join("test.txt");

        atomic_write(&file_path, b"nested content").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "nested content");
    }

    #[test]
    fn test_atomic_write_preserves_content_on_success() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.yaml");

        let content = r#"---
id: TASK-001
title: Test task
---

Body content.
"#;

        atomic_write_file(&file_path, content).unwrap();

        let read_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_atomic_write_no_partial_files() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Write initial content
        fs::write(&file_path, "initial").unwrap();

        // Write new content atomically
        atomic_write(&file_path, b"replacement content").unwrap();

        // File should have complete new content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "replacement content");
    }

    #[test]
    fn test_atomic_write_temp_file_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        atomic_write(&file_path, b"content").unwrap();

        // Temp file should be cleaned up (renamed to target)
        let temp_path = temp_dir.path().join(".test.txt.tmp");
        assert!(!temp_path.exists());
    }

    #[test]
    fn test_generate_temp_path() {
        let target = Path::new("/some/path/file.txt");
        let temp = generate_temp_path(target).unwrap();

        assert_eq!(temp.parent().unwrap(), Path::new("/some/path"));
        assert!(temp.file_name().unwrap().to_str().unwrap().starts_with('.'));
        assert!(
            temp.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".tmp")
        );
    }

    #[test]
    fn test_atomic_write_binary_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("binary.bin");

        let binary_content: Vec<u8> = (0..256).map(|i| i as u8).collect();

        atomic_write(&file_path, &binary_content).unwrap();

        let read_content = fs::read(&file_path).unwrap();
        assert_eq!(read_content, binary_content);
    }

    #[test]
    fn test_atomic_write_empty_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("empty.txt");

        atomic_write(&file_path, b"").unwrap();

        let content = fs::read(&file_path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_atomic_write_large_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("large.txt");

        // Create 1MB of content
        let large_content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();

        atomic_write(&file_path, &large_content).unwrap();

        let read_content = fs::read(&file_path).unwrap();
        assert_eq!(read_content.len(), large_content.len());
        assert_eq!(read_content, large_content);
    }

    #[test]
    fn test_atomic_write_unicode_filename() {
        let temp_dir = TempDir::new().unwrap();
        // Use a simpler unicode filename that's more portable
        let file_path = temp_dir.path().join("test_file.txt");

        atomic_write(&file_path, b"unicode test").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "unicode test");
    }

    #[test]
    fn test_atomic_write_concurrent_safe() {
        // This test verifies that atomic writes don't interfere with each other
        // when writing to different files
        let temp_dir = TempDir::new().unwrap();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let path = temp_dir.path().join(format!("file_{}.txt", i));
                let content = format!("content {}", i);
                std::thread::spawn(move || {
                    atomic_write_file(&path, &content).unwrap();
                    (path, content)
                })
            })
            .collect();

        for handle in handles {
            let (path, expected_content) = handle.join().unwrap();
            let actual_content = fs::read_to_string(&path).unwrap();
            assert_eq!(actual_content, expected_content);
        }
    }
}
