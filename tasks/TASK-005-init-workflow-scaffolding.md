---
id: TASK-005
title: Implement `burl init` (workflow worktree + `.workflow/` scaffolding)
priority: high
depends_on: [TASK-001, TASK-002, TASK-003, TASK-004]
---

## Objective
Implement `burl init` to bootstrap (or reattach) the canonical workflow worktree and on-branch workflow state directory structure.

## Context
Source of truth: `burl.md` sections:
- “Directory structure”
- “Workflow branch + canonical workflow worktree”
- “burl init” requirements

## Requirements
### Workflow worktree
- Defaults:
  - workflow branch: `burl`
  - workflow worktree path: `.burl/`
- `burl init` must be **idempotent**.
- Create or attach the workflow worktree such that:
  - `.burl/` exists and is a valid git worktree
  - `.burl/` is checked out to the workflow branch
  - if `.burl/` exists but is not a git worktree (or points at another repo), fail with exit `1` and remediation steps (delete/rename `.burl/` or re-run init after fixing)

### Workflow state directories (tracked)
Create under `.burl/.workflow/`:
- `READY/`, `DOING/`, `QA/`, `DONE/`, `BLOCKED/`
- `events/`
- `config.yaml` template (if missing) matching the PRD’s example keys/shape (including validation stub patterns)

### Locks directory (untracked)
- Ensure `.burl/.workflow/locks/` exists locally.
- Ensure locks are never committed:
  - write `.burl/.workflow/.gitignore` with `locks/` entry

### Local worktrees directory (untracked)
- Ensure `.worktrees/` exists at repo root (local, untracked).

### Git hygiene
- Recommended: add `.burl/` and `.worktrees/` to `.git/info/exclude` (so `git status` stays clean on main).

### Workflow branch commit
- If `workflow_auto_commit: true`, `burl init` should commit the scaffolding changes on the workflow branch.
- If `workflow_auto_push: true`, push the workflow branch after committing.

### Logging note (PRD compliance)
- The PRD requires an `init` event to be logged. Wire this in `TASK-006` (once the event log append helper exists).

## Acceptance Criteria
- [ ] After `burl init`, the repo contains:
  - `.burl/.workflow/READY/` (and other buckets)
  - `.burl/.workflow/config.yaml`
  - `.burl/.workflow/.gitignore` ignoring `locks/`
  - `.worktrees/`
- [ ] `burl init` can be run twice with no errors and no destructive changes.

## Implementation Notes
- Prefer using the git CLI for worktree operations for parity with user behavior:
  - create branch + worktree:
    - if branch doesn’t exist: `git worktree add -b burl .burl <main_branch>`
    - if branch exists: `git worktree add .burl burl` (or `git worktree add -B burl .burl <main_branch>` if you want reset semantics; **avoid** destructive behavior by default)
- Any workflow-state write during init should acquire `workflow.lock`.
- If re-running `burl init` and the workflow worktree has tracked modifications, fail with a user error (exit `1`) and actionable guidance (see PRD transactional preconditions).

## Test Plan
### Integration (temp git repo)
- `git init`, commit an initial file
- run `burl init`
- verify directories exist
- run `burl init` again
- verify `.burl/` is present in `git worktree list`

## Validation
- `cargo test`
