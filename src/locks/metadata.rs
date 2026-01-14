//! Lock metadata structures and utilities.

use crate::error::{BurlError, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Lock metadata stored in lock files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Owner of the lock (e.g., `user@HOST`).
    pub owner: String,

    /// Process ID of the lock holder (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,

    /// Timestamp when the lock was created (RFC3339).
    pub created_at: DateTime<Utc>,

    /// The action being performed (claim/submit/approve/etc.).
    pub action: String,
}

impl LockMetadata {
    /// Create new lock metadata with the current timestamp.
    pub fn new(action: &str) -> Self {
        Self {
            owner: get_owner_string(),
            pid: Some(std::process::id()),
            created_at: Utc::now(),
            action: action.to_string(),
        }
    }

    /// Parse lock metadata from a file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read lock file '{}': {}",
                path.as_ref().display(),
                e
            ))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            BurlError::UserError(format!(
                "failed to parse lock file '{}': {}",
                path.as_ref().display(),
                e
            ))
        })
    }

    /// Serialize lock metadata to JSON string.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize lock metadata: {}", e)))
    }

    /// Calculate the age of the lock.
    pub fn age(&self) -> Duration {
        Utc::now().signed_duration_since(self.created_at)
    }

    /// Format the age as a human-readable string.
    pub fn age_string(&self) -> String {
        let age = self.age();
        let minutes = age.num_minutes();
        let hours = age.num_hours();
        let days = age.num_days();

        if days > 0 {
            format!("{}d {}h", days, hours % 24)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes % 60)
        } else {
            format!("{}m", minutes)
        }
    }

    /// Check if the lock is stale based on the given threshold in minutes.
    pub fn is_stale(&self, stale_minutes: u32) -> bool {
        self.age().num_minutes() > stale_minutes as i64
    }
}

/// Get the owner string for lock metadata.
pub(crate) fn get_owner_string() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!("{}@{}", user, host)
}
