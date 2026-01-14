//! Compiled stub pattern matching.

use crate::config::Config;
use crate::error::{BurlError, Result};
use regex::Regex;

/// Compiled stub patterns for efficient matching.
///
/// This struct caches compiled regexes for reuse across multiple lines.
/// Create once per validation run.
pub struct CompiledStubPatterns {
    /// The compiled regex patterns paired with their original string representations.
    patterns: Vec<(Regex, String)>,
    /// Normalized extensions to check (lowercase, no leading dots).
    extensions: Vec<String>,
}

impl std::fmt::Debug for CompiledStubPatterns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledStubPatterns")
            .field(
                "patterns",
                &self.patterns.iter().map(|(_, s)| s).collect::<Vec<_>>(),
            )
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl CompiledStubPatterns {
    /// Compile stub patterns from config.
    ///
    /// # Arguments
    ///
    /// * `config` - The workflow configuration containing stub_patterns and stub_check_extensions
    ///
    /// # Returns
    ///
    /// * `Ok(CompiledStubPatterns)` - Successfully compiled patterns
    /// * `Err(BurlError::UserError)` - If any pattern fails to compile (config error, exit 1)
    ///
    /// # Example
    ///
    /// ```
    /// use burl::config::Config;
    /// use burl::validate::stubs::CompiledStubPatterns;
    ///
    /// let config = Config::default();
    /// let patterns = CompiledStubPatterns::from_config(&config).unwrap();
    /// ```
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut patterns = Vec::with_capacity(config.stub_patterns.len());

        for pattern_str in &config.stub_patterns {
            let regex = Regex::new(pattern_str).map_err(|e| {
                BurlError::UserError(format!(
                    "invalid regex pattern in stub_patterns: '{}' - {}\n\
                     Fix: edit config.yaml and correct or remove this pattern.",
                    pattern_str, e
                ))
            })?;
            patterns.push((regex, pattern_str.clone()));
        }

        Ok(Self {
            patterns,
            extensions: config.normalized_extensions(),
        })
    }

    /// Check if a file extension should be scanned for stubs.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The file path to check (can use forward or back slashes)
    ///
    /// # Returns
    ///
    /// `true` if the file's extension is in the configured `stub_check_extensions`
    pub fn should_check_file(&self, file_path: &str) -> bool {
        // Extract extension from file path
        let ext = match file_path.rsplit('.').next() {
            Some(e) if file_path.contains('.') => e.to_lowercase(),
            _ => return false,
        };

        self.extensions.contains(&ext)
    }

    /// Check if a line matches any stub pattern.
    ///
    /// # Arguments
    ///
    /// * `content` - The line content to check
    ///
    /// # Returns
    ///
    /// `Some(pattern)` if the line matches a stub pattern, `None` otherwise
    pub fn matches_stub(&self, content: &str) -> Option<&str> {
        for (regex, pattern_str) in &self.patterns {
            if regex.is_match(content) {
                return Some(pattern_str);
            }
        }
        None
    }
}
