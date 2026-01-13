---
id: TASK-002
title: Repo discovery + git runner + config model
priority: high
depends_on: [TASK-001]
---

## Objective
Implement the core “environment resolution” layer: find the Git repo root from any working directory, provide a safe way to run Git commands, and define/load the V1 config model (`config.yaml`).

## Context
Source of truth: `burl.md` sections:
- “Directory structure”
- “Configuration files”
- “Git Worktree & Branch Model”

## Requirements
### Repo discovery
- Determine the repository root even when invoked from:
  - the main worktree
  - the `.burl/` workflow worktree
  - a task worktree under `.worktrees/`
- A failure to find a Git repo must be a clean user error (exit `1`), not a panic.

### Canonical workflow resolution (source of truth)
- All commands must treat the canonical workflow worktree as the single source of truth:
  - workflow worktree: default `.burl/`
  - workflow state dir: default `.burl/.workflow/`
- Commands may be invoked from **any** directory/worktree, but must always read/write workflow state under the canonical workflow worktree (never from the current working directory).
- Provide a helper that resolves:
  - repo root
  - workflow worktree path
  - workflow state dir path
  - and returns a user error (exit `1`) with “run `burl init`” when the workflow worktree/state dir is missing (except for `burl init` itself).

### Bootstrap/layout note (avoid config chicken-and-egg)
- In V1, treat the workflow layout as **fixed defaults**:
  - workflow worktree path: `.burl/`
  - workflow branch: `burl`
  - workflow state dir: `.burl/.workflow/`
- The config fields `workflow_branch` / `workflow_worktree` are still parsed for forward-compat, but **must not** relocate the workflow worktree in V1 unless you also implement explicit migration behavior.

### Git runner
- Provide a helper that executes git commands with:
  - explicit working directory
  - captured stdout/stderr
  - structured error for non-zero exits (mapped to exit code `3`)

### Workflow worktree cleanliness guard (precondition for mutations)
- Implement a reusable precondition check for any command that commits workflow state:
  - `git -C <workflow_worktree> status --porcelain --untracked-files=no` must be empty
  - if not empty: exit `1` with actionable guidance (“commit/stash/revert changes in the workflow worktree”)
- This guard should be called before acquiring `workflow.lock` for state-changing operations (to avoid deadlocks where a dirty workflow worktree blocks progress).

### Config model
Implement a `Config` representation for `.burl/.workflow/config.yaml` with at least:
- workflow: `workflow_branch`, `workflow_worktree`, `workflow_auto_commit`, `workflow_auto_push`
- git: `main_branch`, `remote`, `merge_strategy`, `push_main_on_approve`, `push_task_branch_on_submit`
- locks: `lock_stale_minutes`, `use_global_claim_lock`
- qa: `qa_max_attempts`, `auto_priority_boost_on_retry`
- validation: `build_command`, `stub_patterns`, `stub_check_extensions`
- overlap policy: `conflict_policy`

Config loading rules:
- For commands other than `init`, missing config is a user error (exit `1`) with a fix: “run `burl init`”.
- For `init`, config should be created if missing (later task implements).
- Unknown fields in YAML must be preserved or ignored safely (forward-compat); do not hard-fail on unknown keys.

### Config validation
- Validate config values and fail fast with exit `1` on:
  - invalid `merge_strategy` or `conflict_policy` values
  - invalid `stub_check_extensions` entries (must be non-empty, no leading dots; normalize to lowercase)
  - negative/zero `lock_stale_minutes` or `qa_max_attempts`

## Acceptance Criteria
- [ ] Running `burl` inside a non-git directory prints a clear error and exits `1`.
- [ ] Repo root resolution works from nested worktrees (unit/integration test).
- [ ] Workflow state resolution always targets `.burl/.workflow/` even when run inside a task worktree.
- [ ] Config loads successfully from a YAML file containing extra/unknown keys.

## Implementation Notes
- Repo root detection approaches (pick one):
  1) `git rev-parse --show-toplevel` (simple, reliable)
  2) walk up filesystem looking for `.git` (more complex; can misbehave with worktrees)
- Prefer approach (1) for correctness with worktrees.
- Treat all workflow paths as *repo-root relative* (store/compute absolute paths only at runtime).

## Test Plan
### Unit
- Config parse:
  - loads defaults when optional fields missing
  - tolerates unknown keys
- Repo root resolution:
  - returns error outside git repo

### Integration (temp git repo)
- init a git repo with a commit
- create an extra directory and run “repo root resolve” from it

## Validation
- `cargo test`
