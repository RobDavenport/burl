//! Config loading, validation, and utility operations.

use super::model::Config;
use crate::error::{BurlError, Result};
use std::path::Path;

impl Config {
    /// Load config from a YAML file.
    ///
    /// Unknown fields in the YAML are silently ignored for forward compatibility.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the config.yaml file
    ///
    /// # Returns
    ///
    /// * `Ok(Config)` - Successfully loaded and validated config
    /// * `Err(BurlError::UserError)` - Parse error or validation failure
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        let content = std::fs::read_to_string(path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read config file '{}': {}",
                path.display(),
                e
            ))
        })?;

        Self::from_yaml(&content)
    }

    /// Parse config from a YAML string.
    ///
    /// Unknown fields in the YAML are silently ignored for forward compatibility.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let config: Config = serde_yaml::from_str(yaml)
            .map_err(|e| BurlError::UserError(format!("failed to parse config YAML: {}", e)))?;

        config.validate()?;
        Ok(config)
    }

    /// Serialize config to YAML string.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize config to YAML: {}", e)))
    }

    /// Validate config values and return error on invalid values.
    ///
    /// Validation rules:
    /// - `lock_stale_minutes` must be positive
    /// - `qa_max_attempts` must be positive
    /// - `stub_check_extensions` entries must be non-empty and have no leading dots
    pub fn validate(&self) -> Result<()> {
        // Validate lock_stale_minutes
        if self.lock_stale_minutes == 0 {
            return Err(BurlError::UserError(
                "config validation failed: lock_stale_minutes must be greater than 0".to_string(),
            ));
        }

        // Validate qa_max_attempts
        if self.qa_max_attempts == 0 {
            return Err(BurlError::UserError(
                "config validation failed: qa_max_attempts must be greater than 0".to_string(),
            ));
        }

        // Validate stub_check_extensions
        for ext in &self.stub_check_extensions {
            if ext.is_empty() {
                return Err(BurlError::UserError(
                    "config validation failed: stub_check_extensions entries must be non-empty"
                        .to_string(),
                ));
            }
            if ext.starts_with('.') {
                return Err(BurlError::UserError(format!(
                    "config validation failed: stub_check_extensions entries must not have leading dots (found '{}'). Use '{}' instead.",
                    ext,
                    ext.trim_start_matches('.')
                )));
            }
        }

        Ok(())
    }

    /// Get stub_check_extensions normalized to lowercase.
    pub fn normalized_extensions(&self) -> Vec<String> {
        self.stub_check_extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect()
    }
}
