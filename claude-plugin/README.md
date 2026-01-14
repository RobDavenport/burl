# Burl Claude Code Plugin

Claude Code integration for [Burl](https://github.com/RobDavenport/burl), a file-based workflow orchestrator for agentic AI development.

## Features

- **Workflow knowledge**: Complete understanding of Burl task states, validation gates, and CLI commands
- **Task planning**: Intelligent assistant that analyzes codebase and creates properly-scoped tasks
- **Context awareness**: Automatic detection of Burl repositories and task worktrees

## Components

| Component | Name | Purpose |
|-----------|------|---------|
| Skill | `burl` | Workflow knowledge with progressive disclosure |
| Agent | `burl-assistant` | Task planning and scope validation |

## Installation

### Local Testing

```bash
claude --plugin-dir /path/to/burl/claude-plugin
```

### Project Installation

Copy to your project's `.claude-plugin/` directory or add as a submodule.

## Usage

### Skill Triggers

The skill activates when you mention:
- "burl", "burl workflow", "burl cli"
- "task states", "scope constraints"
- "affects_globs", "must_not_touch"
- Any Burl command: claim, submit, validate, approve

### Agent Activation

The `burl-assistant` agent triggers when:
- Working in a Burl-enabled repository
- Planning tasks or breaking down features
- Checking submission readiness

## Example Workflow

```
You: "I need to add user authentication"

Claude: [burl-assistant activates]
        Analyzing codebase structure...

        Proposed task:
        - Title: "Implement user authentication"
        - Scope: src/auth/**
        - Forbidden: src/core/**, src/networking/**

        Create this task?

You: "Yes, create it"

Claude: Running: burl add "Implement user authentication" --affects-globs "src/auth/**" --must-not-touch "src/core/**,src/networking/**"

        Task TASK-001 created. Claim it with: burl claim TASK-001
```

## Prerequisites

- Burl CLI installed and in PATH
- Git repository with Burl initialized (`burl init`)

## License

MIT
