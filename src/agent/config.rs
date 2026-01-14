//! Agent configuration schema for burl V2.
//!
//! This module defines the `agents.yaml` configuration file format, which specifies
//! agent profiles for automated task execution.
//!
//! # File Format
//!
//! ```yaml
//! agents:
//!   claude-code:
//!     name: "Claude Code CLI"
//!     command: "claude -p {prompt_file}"
//!     timeout_seconds: 600
//!     environment:
//!       CLAUDE_CODE_AUTO_CONFIRM: "true"
//!     capabilities:
//!       - coding
//!       - refactoring
//!     default: true
//!
//!   custom-script:
//!     name: "Custom Script"
//!     command: "./scripts/agent.sh {task_id} {worktree}"
//!     timeout_seconds: 300
//!
//! defaults:
//!   timeout_seconds: 600
//!
//! prompt_templates:
//!   default: |
//!     # Task: {title}
//!     ## Objective
//!     {objective}
//! ```
//!
//! # Variable Placeholders
//!
//! Command templates support the following placeholders:
//!
//! - `{task_id}` - Task identifier (e.g., "TASK-001")
//! - `{task_file}` - Absolute path to the task markdown file
//! - `{prompt_file}` - Absolute path to the generated prompt file
//! - `{worktree}` - Absolute path to the task worktree
//! - `{branch}` - Task branch name
//! - `{base_sha}` - Base SHA for diff validation
//! - `{title}` - Task title

use crate::error::{BurlError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Default timeout for agent execution in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

/// Configuration for all agents, loaded from `agents.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Agent profiles keyed by identifier.
    #[serde(default)]
    pub agents: BTreeMap<String, AgentProfile>,

    /// Default settings applied to all agents.
    #[serde(default)]
    pub defaults: AgentDefaults,

    /// Prompt templates keyed by name.
    #[serde(default)]
    pub prompt_templates: BTreeMap<String, String>,

    /// Unknown fields preserved for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

/// Default settings for agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDefaults {
    /// Default timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,

    /// Default prompt template name.
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,

    /// Unknown fields preserved for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            prompt_template: default_prompt_template(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

fn default_prompt_template() -> String {
    "default".to_string()
}

/// Profile for a single agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Human-readable name for the agent.
    #[serde(default)]
    pub name: String,

    /// Command template with variable placeholders.
    ///
    /// Placeholders are substituted at execution time:
    /// - `{task_id}` - Task identifier
    /// - `{task_file}` - Path to task markdown file
    /// - `{prompt_file}` - Path to generated prompt file
    /// - `{worktree}` - Path to task worktree
    /// - `{branch}` - Task branch name
    /// - `{base_sha}` - Base SHA for validation
    /// - `{title}` - Task title
    pub command: String,

    /// Timeout in seconds (overrides default if set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,

    /// Environment variables to set for the agent process.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub environment: HashMap<String, String>,

    /// Agent capabilities (informational, for matching tasks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,

    /// Whether this is the default agent.
    #[serde(default)]
    pub default: bool,

    /// Prompt template to use (overrides default if set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,

    /// Unknown fields preserved for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl AgentProfile {
    /// Get the effective timeout for this agent.
    pub fn effective_timeout(&self, defaults: &AgentDefaults) -> u64 {
        self.timeout_seconds.unwrap_or(defaults.timeout_seconds)
    }

    /// Get the effective prompt template name for this agent.
    pub fn effective_prompt_template<'a>(&'a self, defaults: &'a AgentDefaults) -> &'a str {
        self.prompt_template
            .as_deref()
            .unwrap_or(&defaults.prompt_template)
    }
}

impl AgentsConfig {
    /// Load agents config from a YAML file.
    ///
    /// Returns `Ok(None)` if the file does not exist.
    /// Returns `Err` if the file exists but cannot be parsed.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Option<Self>> {
        let path = path.as_ref();

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(path).map_err(|e| {
            BurlError::UserError(format!(
                "failed to read agents config '{}': {}",
                path.display(),
                e
            ))
        })?;

        let config = Self::from_yaml(&content)?;
        Ok(Some(config))
    }

    /// Parse agents config from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let config: AgentsConfig = serde_yaml::from_str(yaml)
            .map_err(|e| BurlError::UserError(format!("failed to parse agents.yaml: {}", e)))?;

        config.validate()?;
        Ok(config)
    }

    /// Serialize config to YAML string.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self)
            .map_err(|e| BurlError::UserError(format!("failed to serialize agents config: {}", e)))
    }

    /// Validate the agents configuration.
    ///
    /// Validation rules:
    /// - Agent identifiers must not be empty
    /// - Command templates must not be empty
    /// - At most one agent can be marked as default
    /// - Default timeout must be positive
    pub fn validate(&self) -> Result<()> {
        // Validate default timeout
        if self.defaults.timeout_seconds == 0 {
            return Err(BurlError::UserError(
                "agents.yaml validation failed: defaults.timeout_seconds must be greater than 0"
                    .to_string(),
            ));
        }

        // Count default agents
        let default_count = self.agents.values().filter(|a| a.default).count();
        if default_count > 1 {
            return Err(BurlError::UserError(
                "agents.yaml validation failed: at most one agent can be marked as default"
                    .to_string(),
            ));
        }

        // Validate each agent
        for (id, agent) in &self.agents {
            if id.is_empty() {
                return Err(BurlError::UserError(
                    "agents.yaml validation failed: agent identifier cannot be empty".to_string(),
                ));
            }

            if agent.command.is_empty() {
                return Err(BurlError::UserError(format!(
                    "agents.yaml validation failed: agent '{}' has empty command",
                    id
                )));
            }

            if let Some(timeout) = agent.timeout_seconds
                && timeout == 0
            {
                return Err(BurlError::UserError(format!(
                    "agents.yaml validation failed: agent '{}' has timeout_seconds of 0",
                    id
                )));
            }

            // Validate prompt template reference
            if let Some(ref template) = agent.prompt_template
                && !template.is_empty()
                && template != "default"
                && !self.prompt_templates.contains_key(template)
            {
                return Err(BurlError::UserError(format!(
                    "agents.yaml validation failed: agent '{}' references unknown prompt_template '{}'",
                    id, template
                )));
            }
        }

        Ok(())
    }

    /// Get the default agent, if one is configured.
    pub fn default_agent(&self) -> Option<(&str, &AgentProfile)> {
        self.agents
            .iter()
            .find(|(_, a)| a.default)
            .map(|(id, a)| (id.as_str(), a))
    }

    /// Get an agent by identifier.
    pub fn get(&self, id: &str) -> Option<&AgentProfile> {
        self.agents.get(id)
    }

    /// Get a prompt template by name, falling back to default if not found.
    pub fn get_prompt_template(&self, name: &str) -> Option<&str> {
        self.prompt_templates.get(name).map(String::as_str)
    }

    /// Check if any agents are configured.
    pub fn has_agents(&self) -> bool {
        !self.agents.is_empty()
    }

    /// Get the number of configured agents.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Iterate over all agents.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AgentProfile)> {
        self.agents.iter().map(|(id, a)| (id.as_str(), a))
    }
}

/// Default prompt template content.
pub fn default_prompt_template_content() -> &'static str {
    r#"# Task: {title}

## Objective
{objective}

## Acceptance Criteria
{acceptance_criteria}

## Context
{context}

## Constraints
- Affected files: {affects}
- Must not touch: {must_not_touch}

## Instructions
Complete the task according to the acceptance criteria above.
When finished, run `burl submit {task_id}` to submit for review.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let yaml = r#"
agents:
  test-agent:
    command: "echo {task_id}"
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();
        assert_eq!(config.agents.len(), 1);
        assert!(config.agents.contains_key("test-agent"));
    }

    #[test]
    fn test_parse_full_config() {
        let yaml = r#"
agents:
  claude-code:
    name: "Claude Code CLI"
    command: "claude -p {prompt_file}"
    timeout_seconds: 900
    environment:
      CLAUDE_CODE_AUTO_CONFIRM: "true"
    capabilities:
      - coding
      - refactoring
    default: true

  custom:
    name: "Custom Script"
    command: "./scripts/agent.sh {task_id}"
    timeout_seconds: 300

defaults:
  timeout_seconds: 600
  prompt_template: "default"

prompt_templates:
  default: |
    # Task: {title}
    {objective}
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();

        assert_eq!(config.agents.len(), 2);

        let claude = config.get("claude-code").unwrap();
        assert_eq!(claude.name, "Claude Code CLI");
        assert_eq!(claude.timeout_seconds, Some(900));
        assert!(claude.default);
        assert_eq!(claude.capabilities, vec!["coding", "refactoring"]);
        assert_eq!(
            claude.environment.get("CLAUDE_CODE_AUTO_CONFIRM"),
            Some(&"true".to_string())
        );

        let custom = config.get("custom").unwrap();
        assert_eq!(custom.timeout_seconds, Some(300));
        assert!(!custom.default);

        assert_eq!(config.defaults.timeout_seconds, 600);
        assert!(config.prompt_templates.contains_key("default"));
    }

    #[test]
    fn test_default_agent() {
        let yaml = r#"
agents:
  first:
    command: "echo first"
  second:
    command: "echo second"
    default: true
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();
        let (id, _) = config.default_agent().unwrap();
        assert_eq!(id, "second");
    }

    #[test]
    fn test_no_default_agent() {
        let yaml = r#"
agents:
  first:
    command: "echo first"
  second:
    command: "echo second"
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();
        assert!(config.default_agent().is_none());
    }

    #[test]
    fn test_multiple_defaults_fails() {
        let yaml = r#"
agents:
  first:
    command: "echo first"
    default: true
  second:
    command: "echo second"
    default: true
"#;
        let result = AgentsConfig::from_yaml(yaml);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at most one agent")
        );
    }

    #[test]
    fn test_empty_command_fails() {
        let yaml = r#"
agents:
  test-agent:
    command: ""
"#;
        let result = AgentsConfig::from_yaml(yaml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty command"));
    }

    #[test]
    fn test_zero_timeout_fails() {
        let yaml = r#"
agents:
  test-agent:
    command: "echo test"
    timeout_seconds: 0
"#;
        let result = AgentsConfig::from_yaml(yaml);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("timeout_seconds of 0")
        );
    }

    #[test]
    fn test_zero_default_timeout_fails() {
        let yaml = r#"
defaults:
  timeout_seconds: 0
agents:
  test-agent:
    command: "echo test"
"#;
        let result = AgentsConfig::from_yaml(yaml);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("defaults.timeout_seconds must be greater than 0")
        );
    }

    #[test]
    fn test_unknown_prompt_template_fails() {
        let yaml = r#"
agents:
  test-agent:
    command: "echo test"
    prompt_template: "nonexistent"
"#;
        let result = AgentsConfig::from_yaml(yaml);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown prompt_template")
        );
    }

    #[test]
    fn test_effective_timeout() {
        let defaults = AgentDefaults {
            timeout_seconds: 600,
            ..Default::default()
        };

        let agent_with_timeout = AgentProfile {
            command: "echo test".to_string(),
            timeout_seconds: Some(300),
            ..Default::default()
        };
        assert_eq!(agent_with_timeout.effective_timeout(&defaults), 300);

        let agent_without_timeout = AgentProfile {
            command: "echo test".to_string(),
            timeout_seconds: None,
            ..Default::default()
        };
        assert_eq!(agent_without_timeout.effective_timeout(&defaults), 600);
    }

    #[test]
    fn test_forward_compatibility() {
        let yaml = r#"
agents:
  test-agent:
    command: "echo test"
    unknown_field: "should be preserved"
    nested:
      another: "value"

defaults:
  timeout_seconds: 600
  future_setting: true

future_top_level: "also preserved"
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();

        // Unknown fields should be preserved
        let agent = config.get("test-agent").unwrap();
        assert!(agent.extra.contains_key("unknown_field"));
        assert!(agent.extra.contains_key("nested"));
        assert!(config.defaults.extra.contains_key("future_setting"));
        assert!(config.extra.contains_key("future_top_level"));

        // Round-trip should preserve unknown fields
        let yaml_out = config.to_yaml().unwrap();
        let config2 = AgentsConfig::from_yaml(&yaml_out).unwrap();
        assert!(config2.extra.contains_key("future_top_level"));
    }

    #[test]
    fn test_empty_config() {
        let yaml = "";
        let config = AgentsConfig::from_yaml(yaml).unwrap();
        assert!(config.agents.is_empty());
        assert_eq!(config.defaults.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn test_iter_agents() {
        let yaml = r#"
agents:
  alpha:
    command: "echo alpha"
  beta:
    command: "echo beta"
"#;
        let config = AgentsConfig::from_yaml(yaml).unwrap();
        let ids: Vec<&str> = config.iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"alpha"));
        assert!(ids.contains(&"beta"));
    }
}
