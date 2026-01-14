//! Event logging subsystem for burl.
//!
//! This module implements append-only event logging to support audit/recovery
//! across machines. Events are stored in NDJSON format (one JSON object per line)
//! in `.burl/.workflow/events/events.ndjson`.
//!
//! # Event Format
//!
//! Each event is a JSON object with the following fields:
//! - `ts`: RFC3339 timestamp
//! - `action`: The action performed (init, add, claim, submit, etc.)
//! - `actor`: The owner string (e.g., `user@HOST`)
//! - `task`: Optional task ID for task-specific events
//! - `details`: Freeform object with action-specific details
//!
//! # Usage
//!
//! Events should be appended while holding `workflow.lock` for commands that
//! commit workflow state, so the log and state move together.
//!
//! ```no_run
//! use burl::events::{Event, EventAction, append_event};
//! use burl::context::WorkflowContext;
//! use serde_json::json;
//!
//! let ctx = WorkflowContext::resolve()?;
//! let event = Event::new(EventAction::Init)
//!     .with_details(json!({"workflow_branch": "burl"}));
//! append_event(&ctx, &event)?;
//! # Ok::<(), burl::error::BurlError>(())
//! ```

use crate::context::WorkflowContext;
use crate::error::{BurlError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// Actions that can be logged as events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// Workflow initialization
    Init,
    /// Task added to READY
    Add,
    /// Task claimed (READY -> DOING)
    Claim,
    /// Task submitted (DOING -> QA)
    Submit,
    /// Task validation run
    Validate,
    /// Task approved (QA -> DONE)
    Approve,
    /// Task rejected (QA -> READY)
    Reject,
    /// Lock cleared manually
    LockClear,
    /// Cleanup operation
    Clean,
    /// Agent dispatch started (V2)
    AgentDispatch,
    /// Agent execution completed (V2)
    AgentComplete,
}

impl std::fmt::Display for EventAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventAction::Init => write!(f, "init"),
            EventAction::Add => write!(f, "add"),
            EventAction::Claim => write!(f, "claim"),
            EventAction::Submit => write!(f, "submit"),
            EventAction::Validate => write!(f, "validate"),
            EventAction::Approve => write!(f, "approve"),
            EventAction::Reject => write!(f, "reject"),
            EventAction::LockClear => write!(f, "lock_clear"),
            EventAction::Clean => write!(f, "clean"),
            EventAction::AgentDispatch => write!(f, "agent_dispatch"),
            EventAction::AgentComplete => write!(f, "agent_complete"),
        }
    }
}

/// An event record for the audit log.
///
/// Events are serialized as single-line JSON objects and appended to
/// the events.ndjson file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// RFC3339 timestamp when the event occurred.
    pub ts: DateTime<Utc>,

    /// The action that was performed.
    pub action: EventAction,

    /// The actor who performed the action (e.g., `user@HOST`).
    pub actor: String,

    /// Optional task ID for task-specific events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,

    /// Freeform details object with action-specific information.
    pub details: Value,
}

impl Event {
    /// Create a new event with the given action.
    ///
    /// The timestamp is set to the current time, and the actor is
    /// determined from the environment (USER@HOSTNAME).
    pub fn new(action: EventAction) -> Self {
        Self {
            ts: Utc::now(),
            action,
            actor: get_actor_string(),
            task: None,
            details: Value::Object(serde_json::Map::new()),
        }
    }

    /// Set the task ID for this event.
    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task = Some(task_id.into());
        self
    }

    /// Set the details object for this event.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    /// Serialize the event to a single-line JSON string.
    ///
    /// This is used for NDJSON format where each line is a complete JSON object.
    pub fn to_ndjson_line(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize event to JSON: {}", e)))
    }
}

/// Get the actor string for event metadata.
fn get_actor_string() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    format!("{}@{}", user, host)
}

/// Get the path to the events file.
pub fn events_file_path(ctx: &WorkflowContext) -> PathBuf {
    ctx.events_dir().join("events.ndjson")
}

/// Append an event to the events log.
///
/// This function appends the event as a single JSON line to the events.ndjson file.
/// The file is created if it doesn't exist. Each append results in one line with
/// a trailing newline.
///
/// # Arguments
///
/// * `ctx` - The workflow context
/// * `event` - The event to append
///
/// # Returns
///
/// * `Ok(())` - Event was successfully appended
/// * `Err(BurlError::UserError)` - Serialization or write failed
///
/// # Important
///
/// If JSON serialization fails, this is treated as a user-visible internal error
/// (exit `1`) and the caller should not proceed with state transitions.
pub fn append_event(ctx: &WorkflowContext, event: &Event) -> Result<()> {
    let events_file = events_file_path(ctx);

    // Serialize the event to a single-line JSON string
    let json_line = event.to_ndjson_line()?;

    // Ensure the events directory exists
    let events_dir = ctx.events_dir();
    if !events_dir.exists() {
        fs::create_dir_all(&events_dir).map_err(|e| {
            BurlError::UserError(format!(
                "failed to create events directory '{}': {}",
                events_dir.display(),
                e
            ))
        })?;
    }

    // Open the file in append mode, creating it if it doesn't exist
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_file)
        .map_err(|e| {
            BurlError::UserError(format!(
                "failed to open events file '{}': {}",
                events_file.display(),
                e
            ))
        })?;

    // Write the JSON line with trailing newline
    writeln!(file, "{}", json_line).map_err(|e| {
        BurlError::UserError(format!(
            "failed to write event to '{}': {}",
            events_file.display(),
            e
        ))
    })?;

    // Sync to disk for durability
    file.sync_all().map_err(|e| {
        BurlError::UserError(format!(
            "failed to sync events file '{}': {}",
            events_file.display(),
            e
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a temporary git repository with workflow structure for testing.
    fn create_test_workflow() -> (TempDir, WorkflowContext) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        // Configure git user for commits
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        // Create initial commit
        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        // Create workflow structure
        let ctx = WorkflowContext::resolve_from(path).unwrap();
        std::fs::create_dir_all(ctx.events_dir()).unwrap();

        (temp_dir, ctx)
    }

    #[test]
    fn test_event_creation() {
        let event = Event::new(EventAction::Init);

        assert_eq!(event.action, EventAction::Init);
        assert!(!event.actor.is_empty());
        assert!(event.task.is_none());
        // Timestamp should be recent (within last minute)
        let age = Utc::now().signed_duration_since(event.ts);
        assert!(age.num_minutes() < 1);
    }

    #[test]
    fn test_event_with_task() {
        let event = Event::new(EventAction::Claim).with_task("TASK-001");

        assert_eq!(event.action, EventAction::Claim);
        assert_eq!(event.task, Some("TASK-001".to_string()));
    }

    #[test]
    fn test_event_with_details() {
        let event = Event::new(EventAction::Init)
            .with_details(json!({"workflow_branch": "burl", "auto_commit": true}));

        assert_eq!(event.details["workflow_branch"], "burl");
        assert_eq!(event.details["auto_commit"], true);
    }

    #[test]
    fn test_event_serialization() {
        let event = Event::new(EventAction::Claim)
            .with_task("TASK-001")
            .with_details(json!({"branch": "task-001-feature"}));

        let json_line = event.to_ndjson_line().unwrap();

        // Should be valid JSON
        let parsed: Event = serde_json::from_str(&json_line).unwrap();
        assert_eq!(parsed.action, EventAction::Claim);
        assert_eq!(parsed.task, Some("TASK-001".to_string()));

        // Should not contain newlines (single line)
        assert!(!json_line.contains('\n'));
    }

    #[test]
    fn test_event_action_serialization() {
        // Verify that actions serialize to snake_case
        let event = Event::new(EventAction::LockClear);
        let json_line = event.to_ndjson_line().unwrap();
        assert!(json_line.contains("\"lock_clear\""));

        let event = Event::new(EventAction::Init);
        let json_line = event.to_ndjson_line().unwrap();
        assert!(json_line.contains("\"init\""));
    }

    #[test]
    fn test_event_without_task_omits_field() {
        let event = Event::new(EventAction::Init);
        let json_line = event.to_ndjson_line().unwrap();

        // Should not contain "task" field when None
        let parsed: serde_json::Value = serde_json::from_str(&json_line).unwrap();
        assert!(parsed.get("task").is_none());
    }

    #[test]
    fn test_append_event_creates_file() {
        let (_temp_dir, ctx) = create_test_workflow();
        let events_file = events_file_path(&ctx);

        // File should not exist yet
        assert!(!events_file.exists());

        // Append an event
        let event = Event::new(EventAction::Init).with_details(json!({"workflow_branch": "burl"}));
        append_event(&ctx, &event).unwrap();

        // File should now exist
        assert!(events_file.exists());

        // Content should be valid NDJSON
        let content = fs::read_to_string(&events_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);

        let parsed: Event = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.action, EventAction::Init);
    }

    #[test]
    fn test_append_event_multiple_lines() {
        let (_temp_dir, ctx) = create_test_workflow();
        let events_file = events_file_path(&ctx);

        // Append first event
        let event1 = Event::new(EventAction::Init);
        append_event(&ctx, &event1).unwrap();

        // Append second event
        let event2 = Event::new(EventAction::Add).with_task("TASK-001");
        append_event(&ctx, &event2).unwrap();

        // File should have two lines
        let content = fs::read_to_string(&events_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Both lines should be valid JSON
        let parsed1: Event = serde_json::from_str(lines[0]).unwrap();
        let parsed2: Event = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(parsed1.action, EventAction::Init);
        assert_eq!(parsed2.action, EventAction::Add);
        assert_eq!(parsed2.task, Some("TASK-001".to_string()));
    }

    #[test]
    fn test_append_event_trailing_newline() {
        let (_temp_dir, ctx) = create_test_workflow();
        let events_file = events_file_path(&ctx);

        let event = Event::new(EventAction::Init);
        append_event(&ctx, &event).unwrap();

        let content = fs::read_to_string(&events_file).unwrap();
        // Content should end with newline
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn test_append_event_creates_events_dir() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .expect("failed to init git repo");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .expect("failed to set git email");

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .output()
            .expect("failed to set git name");

        std::fs::write(path.join("README.md"), "# Test\n").unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .expect("failed to add files");
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .expect("failed to commit");

        let ctx = WorkflowContext::resolve_from(path).unwrap();

        // Events directory should not exist
        assert!(!ctx.events_dir().exists());

        // Append an event
        let event = Event::new(EventAction::Init);
        append_event(&ctx, &event).unwrap();

        // Events directory should now exist
        assert!(ctx.events_dir().exists());
    }

    #[test]
    fn test_event_action_display() {
        assert_eq!(format!("{}", EventAction::Init), "init");
        assert_eq!(format!("{}", EventAction::Add), "add");
        assert_eq!(format!("{}", EventAction::Claim), "claim");
        assert_eq!(format!("{}", EventAction::Submit), "submit");
        assert_eq!(format!("{}", EventAction::Validate), "validate");
        assert_eq!(format!("{}", EventAction::Approve), "approve");
        assert_eq!(format!("{}", EventAction::Reject), "reject");
        assert_eq!(format!("{}", EventAction::LockClear), "lock_clear");
        assert_eq!(format!("{}", EventAction::Clean), "clean");
        assert_eq!(format!("{}", EventAction::AgentDispatch), "agent_dispatch");
        assert_eq!(format!("{}", EventAction::AgentComplete), "agent_complete");
    }

    #[test]
    fn test_get_actor_string() {
        let actor = get_actor_string();
        assert!(actor.contains('@'));
        assert!(!actor.is_empty());
    }

    #[test]
    fn test_events_file_path() {
        let (_temp_dir, ctx) = create_test_workflow();
        let path = events_file_path(&ctx);
        assert!(path.ends_with("events.ndjson"));
        assert!(path.to_string_lossy().contains("events"));
    }

    #[test]
    fn test_event_full_roundtrip() {
        // Create an event with all fields populated
        let event = Event::new(EventAction::LockClear)
            .with_task("TASK-001")
            .with_details(json!({
                "lock_id": "workflow",
                "age_minutes": 120,
                "force": true
            }));

        // Serialize to NDJSON
        let json_line = event.to_ndjson_line().unwrap();

        // Parse back
        let parsed: Event = serde_json::from_str(&json_line).unwrap();

        // Verify all fields
        assert_eq!(parsed.action, EventAction::LockClear);
        assert_eq!(parsed.task, Some("TASK-001".to_string()));
        assert_eq!(parsed.details["lock_id"], "workflow");
        assert_eq!(parsed.details["age_minutes"], 120);
        assert_eq!(parsed.details["force"], true);
    }
}
