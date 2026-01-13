---
id: TASK-005
title: Implement `fdx init` (workflow worktree + `.workflow/` scaffolding)
priority: high
depends_on: [TASK-001, TASK-002, TASK-003, TASK-004]
---

## Objective
Implement `fdx init` to bootstrap (or reattach) the canonical workflow worktree and on-branch workflow state directory structure.

## Context
Source of truth: `fdx.md` sections:
- “Directory structure”
- “Workflow branch + canonical workflow worktree”
- “fdx init” requirements

## Requirements
### Workflow worktree
- Defaults:
  - workflow branch: `fdx`
  - workflow worktree path: `.fdx/`
- `fdx init` must be **idempotent**.
- Create or attach the workflow worktree such that:
  - `.fdx/` exists and is a valid git worktree
  - `.fdx/` is checked out to the workflow branch
  - if `.fdx/` exists but is not a git worktree (or points at another repo), fail with exit `1` and remediation steps (delete/rename `.fdx/` or re-run init after fixing)

### Workflow state directories (tracked)
Create under `.fdx/.workflow/`:
- `READY/`, `DOING/`, `QA/`, `DONE/`, `BLOCKED/`
- `events/`
- `config.yaml` template (if missing) matching the PRD’s example keys/shape (including validation stub patterns)

### Locks directory (untracked)
- Ensure `.fdx/.workflow/locks/` exists locally.
- Ensure locks are never committed:
  - write `.fdx/.workflow/.gitignore` with `locks/` entry

### Local worktrees directory (untracked)
- Ensure `.worktrees/` exists at repo root (local, untracked).

### Git hygiene
- Recommended: add `.fdx/` and `.worktrees/` to `.git/info/exclude` (so `git status` stays clean on main).

### Workflow branch commit
- If `workflow_auto_commit: true`, `fdx init` should commit the scaffolding changes on the workflow branch.
- If `workflow_auto_push: true`, push the workflow branch after committing.

### Logging note (PRD compliance)
- The PRD requires an `init` event to be logged. Wire this in `TASK-006` (once the event log append helper exists).

## Acceptance Criteria
- [ ] After `fdx init`, the repo contains:
  - `.fdx/.workflow/READY/` (and other buckets)
  - `.fdx/.workflow/config.yaml`
  - `.fdx/.workflow/.gitignore` ignoring `locks/`
  - `.worktrees/`
- [ ] `fdx init` can be run twice with no errors and no destructive changes.

## Implementation Notes
- Prefer using the git CLI for worktree operations for parity with user behavior:
  - create branch + worktree:
    - if branch doesn’t exist: `git worktree add -b fdx .fdx <main_branch>`
    - if branch exists: `git worktree add .fdx fdx` (or `git worktree add -B fdx .fdx <main_branch>` if you want reset semantics; **avoid** destructive behavior by default)
- Any workflow-state write during init should acquire `workflow.lock`.
- If re-running `fdx init` and the workflow worktree has tracked modifications, fail with a user error (exit `1`) and actionable guidance (see PRD transactional preconditions).

## Test Plan
### Integration (temp git repo)
- `git init`, commit an initial file
- run `fdx init`
- verify directories exist
- run `fdx init` again
- verify `.fdx/` is present in `git worktree list`

## Validation
- `cargo test`
