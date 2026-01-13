---
id: TASK-017
title: Implement `fdx doctor` (report inconsistencies + optional safe repair)
priority: medium
depends_on: [TASK-004, TASK-007, TASK-008, TASK-009, TASK-013, TASK-015, TASK-016]
---

## Objective
Implement `fdx doctor` to detect common workflow inconsistencies and provide actionable fixes. Optionally support `fdx doctor --repair --force` for *safe*, non-destructive repairs.

## Context
Source of truth: `fdx.md` section “Recommended recovery command (V1)”.

## Requirements
### `fdx doctor` (read-only)
Report:
- stale locks (based on config)
- orphan lock files (task lock exists but task doesn’t exist)
- tasks in DOING/QA missing `base_sha`
- tasks in DOING/QA with missing worktree directory
- orphan worktrees under `.worktrees/` not referenced by any task
- tasks that reference a branch that does not exist locally
- bucket/metadata mismatches (e.g., READY task with `started_at`/`base_sha` set; DOING task with `submitted_at` set)

### `fdx doctor --repair --force` (safe repairs only)
Allowed repairs:
- clear stale locks (only those confidently stale)
- recreate missing directories (`locks/`, `events/`) if absent
- (optional) rewrite clearly-invalid recorded worktree paths if a valid worktree exists at the conventional location
- fix bucket placement for the explicit crash scenario in the PRD (metadata written but file not moved), using conservative rules:
  - if in READY and has `started_at` (or `branch`/`worktree`/`base_sha`): move to DOING
  - if in DOING and has `submitted_at`: move to QA
  - if in QA and has `completed_at`: move to DONE
  - always use atomic rename; never duplicate task files

When `--repair` mutates workflow state:
- acquire `workflow.lock` for the critical section (moves/writes + commit)
- if `workflow_auto_commit: true`, commit changes on the workflow branch
- if `workflow_auto_push: true`, push the workflow branch after committing

Disallowed repairs (do not do automatically):
- deleting branches
- deleting worktrees
- widening scope or editing user-authored acceptance criteria

## Acceptance Criteria
- [ ] `fdx doctor` exits `0` when healthy, `1` when issues are found.
- [ ] `fdx doctor` output lists specific paths and recommended remediation commands.
- [ ] `fdx doctor --repair --force` clears stale locks and reports what changed.

## Implementation Notes
- Prefer emitting a structured report (human-readable table + summary).
- `fdx doctor` should not require any network access; avoid implicit fetches.

## Test Plan
### Integration
- Create a stale lock file and confirm it’s reported as stale.
- Run `fdx doctor --repair --force` and confirm the lock is removed.

## Validation
- `cargo test`
