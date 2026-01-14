//! File I/O operations for task files.

use super::TaskFile;
use crate::error::{BurlError, Result};
use std::path::Path;

impl TaskFile {
    /// Load a task file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read task file '{}': {}",
                path.display(),
                e
            ))
        })?;
        Self::parse(&content)
    }

    /// Atomically save the task file to disk.
    ///
    /// Uses atomic write (temp file + rename) to ensure the task file
    /// is never left in a corrupted state.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = self.to_string()?;
        crate::fs::atomic_write_file(path, &content)
    }

    /// Serialize the task file to a string.
    ///
    /// The output preserves the YAML frontmatter format and appends the body.
    pub fn to_string(&self) -> Result<String> {
        let frontmatter_yaml = serde_yaml::to_string(&self.frontmatter).map_err(|e| {
            BurlError::UserError(format!("failed to serialize task frontmatter: {}", e))
        })?;

        // Build the complete file content
        let mut output = String::new();
        output.push_str("---\n");
        output.push_str(&frontmatter_yaml);
        output.push_str("---\n");
        output.push_str(&self.body);

        Ok(output)
    }
}
