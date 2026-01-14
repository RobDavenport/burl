//! Task-agent binding resolution.
//!
//! This module provides logic for resolving which agent should execute a task.
//!
//! # Resolution Order
//!
//! 1. Explicitly assigned agent (task.frontmatter.agent)
//! 2. Default agent from agents.yaml
//! 3. Error if no agent can be resolved

use crate::agent::config::{AgentProfile, AgentsConfig};
use crate::error::{BurlError, Result};
use crate::task::TaskFile;

/// Resolved agent binding for a task.
#[derive(Debug, Clone)]
pub struct AgentBinding<'a> {
    /// The agent profile.
    pub profile: &'a AgentProfile,
    /// The agent identifier.
    pub agent_id: &'a str,
    /// How the binding was resolved.
    pub binding_source: BindingSource,
}

/// How an agent binding was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    /// Task has explicit agent assignment.
    Explicit,
    /// Using default agent from agents.yaml.
    Default,
}

/// Resolve which agent should execute a task.
///
/// # Arguments
///
/// * `task` - The task file
/// * `config` - The agents configuration
///
/// # Resolution Order
///
/// 1. If task.frontmatter.agent is set and the agent exists, use it
/// 2. If not, use the default agent from agents.yaml
/// 3. If no default agent, return an error
///
/// # Errors
///
/// - Task references a non-existent agent
/// - No agent is assigned and no default is configured
pub fn resolve_agent<'a>(task: &'a TaskFile, config: &'a AgentsConfig) -> Result<AgentBinding<'a>> {
    // Check for explicit assignment
    if let Some(ref agent_id) = task.frontmatter.agent {
        match config.get(agent_id) {
            Some(profile) => {
                return Ok(AgentBinding {
                    profile,
                    agent_id,
                    binding_source: BindingSource::Explicit,
                });
            }
            None => {
                return Err(BurlError::UserError(format!(
                    "task '{}' is assigned to agent '{}', but this agent is not configured in agents.yaml.\n\
                     Available agents: {}",
                    task.frontmatter.id,
                    agent_id,
                    available_agents(config)
                )));
            }
        }
    }

    // Fall back to default agent
    match config.default_agent() {
        Some((agent_id, profile)) => Ok(AgentBinding {
            profile,
            agent_id,
            binding_source: BindingSource::Default,
        }),
        None => {
            if config.has_agents() {
                Err(BurlError::UserError(format!(
                    "task '{}' has no agent assigned and no default agent is configured.\n\
                     Either assign an agent to the task or mark one as default in agents.yaml.\n\
                     Available agents: {}",
                    task.frontmatter.id,
                    available_agents(config)
                )))
            } else {
                Err(BurlError::UserError(format!(
                    "task '{}' has no agent assigned and no agents are configured.\n\
                     Create an agents.yaml file with at least one agent profile.\n\n\
                     Example agents.yaml:\n\
                     agents:\n  \
                       claude-code:\n    \
                         name: \"Claude Code CLI\"\n    \
                         command: \"claude -p {{prompt_file}}\"\n    \
                         default: true",
                    task.frontmatter.id
                )))
            }
        }
    }
}

/// Get a formatted list of available agents for error messages.
fn available_agents(config: &AgentsConfig) -> String {
    if config.agents.is_empty() {
        "(none)".to_string()
    } else {
        config
            .agents
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskFrontmatter;

    fn make_task(agent: Option<&str>) -> TaskFile {
        TaskFile {
            frontmatter: TaskFrontmatter {
                id: "TASK-001".to_string(),
                title: "Test Task".to_string(),
                agent: agent.map(|s| s.to_string()),
                ..Default::default()
            },
            body: String::new(),
        }
    }

    fn make_config_with_agents(agents: &[(&str, bool)]) -> AgentsConfig {
        let mut config = AgentsConfig::default();
        for (name, is_default) in agents {
            config.agents.insert(
                name.to_string(),
                AgentProfile {
                    name: name.to_string(),
                    command: format!("echo {}", name),
                    default: *is_default,
                    ..Default::default()
                },
            );
        }
        config
    }

    #[test]
    fn test_resolve_explicit_agent() {
        let task = make_task(Some("my-agent"));
        let config = make_config_with_agents(&[("my-agent", false), ("other", true)]);

        let binding = resolve_agent(&task, &config).unwrap();

        assert_eq!(binding.agent_id, "my-agent");
        assert_eq!(binding.binding_source, BindingSource::Explicit);
    }

    #[test]
    fn test_resolve_default_agent() {
        let task = make_task(None);
        let config = make_config_with_agents(&[("agent1", false), ("agent2", true)]);

        let binding = resolve_agent(&task, &config).unwrap();

        assert_eq!(binding.agent_id, "agent2");
        assert_eq!(binding.binding_source, BindingSource::Default);
    }

    #[test]
    fn test_resolve_nonexistent_explicit_agent_error() {
        let task = make_task(Some("nonexistent"));
        let config = make_config_with_agents(&[("agent1", true)]);

        let result = resolve_agent(&task, &config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"));
        assert!(err.contains("not configured"));
        assert!(err.contains("agent1")); // available agents
    }

    #[test]
    fn test_resolve_no_default_error() {
        let task = make_task(None);
        let config = make_config_with_agents(&[("agent1", false), ("agent2", false)]);

        let result = resolve_agent(&task, &config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no default agent"));
    }

    #[test]
    fn test_resolve_no_agents_error() {
        let task = make_task(None);
        let config = AgentsConfig::default();

        let result = resolve_agent(&task, &config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no agents are configured"));
        assert!(err.contains("Example agents.yaml"));
    }

    #[test]
    fn test_explicit_overrides_default() {
        let task = make_task(Some("explicit-agent"));
        let config = make_config_with_agents(&[("explicit-agent", false), ("default-agent", true)]);

        let binding = resolve_agent(&task, &config).unwrap();

        // Should use explicit, not default
        assert_eq!(binding.agent_id, "explicit-agent");
        assert_eq!(binding.binding_source, BindingSource::Explicit);
    }

    #[test]
    fn test_binding_has_profile() {
        let task = make_task(None);
        let config = make_config_with_agents(&[("test-agent", true)]);

        let binding = resolve_agent(&task, &config).unwrap();

        assert_eq!(binding.profile.name, "test-agent");
        assert_eq!(binding.profile.command, "echo test-agent");
    }

    #[test]
    fn test_available_agents_formatting() {
        let config = make_config_with_agents(&[("alpha", false), ("beta", true)]);
        let result = available_agents(&config);

        // BTreeMap is sorted, so should be in alphabetical order
        assert!(result.contains("alpha"));
        assert!(result.contains("beta"));
    }

    #[test]
    fn test_available_agents_empty() {
        let config = AgentsConfig::default();
        let result = available_agents(&config);
        assert_eq!(result, "(none)");
    }
}
