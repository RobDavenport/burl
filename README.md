# Burl (File-Driven eXecution)

`burl` is a minimal, file-based workflow orchestrator for coding and review pipelines.

## At a glance

- Workflow state is folders plus markdown task files in a dedicated Git worktree (`.burl/` on branch `burl` by default).
- Each claimed task gets its own Git worktree under `.worktrees/`.
- Validation gates are deterministic and diff-based (scope, stubs, optional build/test profile).

## Prerequisites

- You are inside a Git repository (not a submodule) with at least one commit.
- If your default branch is not `main`, set `main_branch` in `.burl/.workflow/config.yaml` after `burl init`.

## Install

```bash
cargo install --path .
```

## Quick workflow

```bash
# One-time setup (idempotent)
burl init

# Create task in READY/
burl add "Implement player jump" --priority high --affects-globs "src/player/**"

# Claim task (READY -> DOING) and create task worktree
burl claim TASK-001

# In task worktree: commit code, then submit to QA
burl submit TASK-001

# Run validation without status transition
burl validate TASK-001

# Finalize
burl approve TASK-001
# or
burl reject TASK-001 --reason "Scope exceeded; touched src/net/**"
```

## New repository bootstrap

```bash
mkdir my-project && cd my-project
git init -b main
echo "# my-project" > README.md
git add README.md
git commit -m "Initial commit"

burl init
```

## Multi-machine or team usage

Durable workflow state lives on the `burl` branch. The `.burl/` directory is just a checkout of that branch.

- Push from machine A:
  - `git push <remote> burl`
  - `git push <remote> <task-branch>`
- Attach from machine B:
  - `git fetch <remote> burl:burl`
  - `burl init`

## Agent dispatch and automation

```bash
# Agent profiles
burl agent list

# Run a configured agent on a claimed task
burl agent run TASK-001
burl agent run TASK-001 --agent claude-code
burl agent run TASK-001 --dry-run

# Workflow dashboard
burl monitor

# Automation loop
burl watch
burl watch --approve
burl watch --dispatch --approve
```

`burl agent run` requires `.burl/.workflow/agents.yaml` and a task in `DOING`.

## Layout

```text
.burl/                 # Canonical workflow worktree (branch: burl)
  .workflow/
    READY/ DOING/ QA/ DONE/ BLOCKED/
    config.yaml
    agents.yaml
    prompts/
    agent-logs/        # Untracked
    events/events.ndjson
    locks/             # Untracked, machine-local

.worktrees/
  task-001-.../        # Per-task Git worktrees
```

## Configuration

Workflow config is `.burl/.workflow/config.yaml`.

Common keys:
- `main_branch`, `remote`
- `build_command` (legacy single-step build/test hook)
- `validation_profiles`, `default_validation_profile`
- `stub_patterns`, `stub_check_extensions`
- `merge_strategy`, `conflict_detection`, `conflict_policy`
- `workflow_auto_commit`, `workflow_auto_push`

Agent profiles are in `.burl/.workflow/agents.yaml`.

For full prompt template and placeholder reference, see `burl.md`.

## Documentation map

- `burl.md`: product requirements and spec
- `ARCHITECTURE.md`: code map and invariants
- `ROADMAP.md`: short- and long-term plan
- `AGENTS.md`: contributor and agent quick reference
- `CLAUDE.md`: repo constraints and implementation notes

## Development validation

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

If `cargo check` or `cargo clippy` fails with `Invalid cross-device link (os error 18)`, treat it as an environment/toolchain issue and use `cargo build` plus `cargo test` as the mechanical gate.
