---
id: TASK-004
title: Locking subsystem (workflow/task/claim) + lock commands
priority: high
depends_on: [TASK-001, TASK-002, TASK-003]
---

## Objective
Implement the lock model required for race-safe workflow mutations:
- global workflow lock
- per-task lock
- optional global claim lock
Plus CLI commands to inspect and clear locks.

## Context
Source of truth: `burl.md` section “Locking model” and “Stale lock recovery”.

## Requirements
### Lock files
- Location: `.burl/.workflow/locks/` (untracked)
- Create locks using **create_new** semantics (exclusive create).
- Lock file body must include:
  - `owner` (e.g. `user@HOST`)
  - `pid` (optional)
  - `created_at` (RFC3339)
  - `action` (claim/submit/approve/etc.)

### Required locks
- Commands that mutate workflow state MUST hold `workflow.lock` for the critical section that writes/moves files under `.burl/.workflow/**` and commits the workflow branch.
- Any command that transitions a task MUST hold the per-task lock (`TASK-001.lock`).
- `burl claim` without explicit task ID should also use `claim.lock` if `use_global_claim_lock: true`.

### Stale locks
- A lock is stale if `now - created_at > lock_stale_minutes`.
- Do not auto-clear stale locks during normal operations.
- Provide:
  - `burl lock list` (shows age + stale flag)
  - `burl lock clear TASK-001 --force`

### Lock naming + clear semantics (remove ambiguity)
- `burl lock list` must list **all** lock files under `.burl/.workflow/locks/`, including:
  - `workflow.lock`
  - `claim.lock`
  - `TASK-###.lock`
- `burl lock clear` must support clearing:
  - a task lock by task ID (`burl lock clear TASK-001 --force`)
  - `workflow.lock` via `burl lock clear workflow --force`
  - `claim.lock` via `burl lock clear claim --force`
- `burl lock clear` should print the cleared lock’s metadata (owner, created_at, age) so the action is auditable.

### Exit codes (locks)
- If a lock cannot be acquired because it already exists, the command must fail with exit code `4` (lock acquisition failure) and a human-readable message pointing to the lock path and metadata.

### Logging note (PRD compliance)
- The PRD requires `lock_clear` events to be logged to the workflow event log.
- Implement event-log wiring for `burl lock clear` in `TASK-006` (once the event log append helper exists).
- After appending the `lock_clear` event:
  - if `workflow_auto_commit: true`, commit the workflow branch
  - if `workflow_auto_push: true`, push the workflow branch

## Acceptance Criteria
- [ ] Two concurrent attempts to acquire the same lock: exactly one succeeds.
- [ ] `burl lock list` identifies stale locks based on config threshold.
- [ ] `burl lock clear` refuses without `--force`.

## Implementation Notes
- Keep lock acquisition/release APIs minimal:
  - `acquire_workflow_lock(action)` → guard object that deletes on drop
  - `acquire_task_lock(task_id, action)` → guard
  - `acquire_claim_lock()` → optional guard
- Lock guards must be resilient: if deletion fails, print a warning but don’t crash.

## Test Plan
### Unit
- Acquire same lock twice in same process → second fails with lock error (exit code `4`).
- Stale detection with synthetic `created_at`.

### Integration
- Spawn two processes both trying to acquire a known lock file; ensure one fails.

## Validation
- `cargo test`
