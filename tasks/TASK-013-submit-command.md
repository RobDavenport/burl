---
id: TASK-013
title: Implement `burl submit` (DOING → QA) with scope+stub gates
priority: high
depends_on: [TASK-006, TASK-009, TASK-011, TASK-012]
---

## Objective
Implement `burl submit [TASK-ID]` to transition a claimed task from DOING → QA after passing deterministic gates.

## Context
Source of truth: `burl.md` sections:
- “burl submit”
- “Deterministic Validation”
- “Return codes”

## Requirements
- Task must currently be in DOING bucket (folder truth).
- Task must have:
  - a recorded `worktree` path that exists locally
  - a recorded `branch` name
  - a non-null `base_sha`
- Verify git state in the task worktree:
  - current branch matches the recorded `branch`
  - if mismatch/missing: exit `1` with remediation (“run `burl doctor` or re-claim the task”)
- Must require at least one commit on the task branch since `base_sha`.
  - If no commits: user error exit `1` with actionable fix (“commit your changes first”).
- Run validations against `{base_sha}..HEAD`:
  - scope validation
  - stub validation
- On success:
  - if `push_task_branch_on_submit: true`, push the task branch to `<remote>` **before** transitioning DOING → QA
    - if push fails: exit `3` and do not move buckets
  - set `submitted_at`
  - move task file DOING → QA (atomic rename)
  - append `submit` event
  - commit workflow branch if enabled
- If `workflow_auto_push: true`, push the workflow branch after committing.
- On validation failure: exit `2` and do not move buckets.

## Acceptance Criteria
- [ ] Submitting with no commits fails (exit `1`) and remains in DOING.
- [ ] Submitting with scope/stub violations fails (exit `2`) and remains in DOING.
- [ ] Submitting a valid task moves it to QA and sets `submitted_at`.

## Implementation Notes
- Locks:
  - acquire per-task lock for the full operation
  - acquire `workflow.lock` only for task file update + move + event append + commit
- “Has commits?” check recommendation:
  - `git rev-list --count {base_sha}..HEAD` in the task worktree

## Test Plan
### Integration
- init → add → claim → modify file → commit → submit
- verify task file path is now under `.burl/.workflow/QA/`

## Validation
- `cargo test`
