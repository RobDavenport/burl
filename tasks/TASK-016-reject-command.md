---
id: TASK-016
title: Implement `burl reject` (QA → READY/BLOCKED) + attempt policy
priority: high
depends_on: [TASK-003, TASK-006, TASK-007, TASK-014]
---

## Objective
Implement `burl reject TASK-ID --reason "..."` to return a QA task to READY with explicit reasons and attempt tracking.

## Context
Source of truth: `burl.md` section “burl reject” and `qa_max_attempts` policy.

## Requirements
- Preconditions:
  - task must be in QA
  - `--reason` is required (non-empty)
- Behavior:
  - increment `qa_attempts`
  - append reason to “QA Report” with timestamp and actor
  - move QA → READY
  - preserve `branch` and `worktree` fields (do not clean by default)
  - append `reject` event and commit workflow branch if enabled
  - if `workflow_auto_push: true`, push the workflow branch after committing
- Attempt policy:
  - if `qa_attempts >= qa_max_attempts`:
    - default: move to BLOCKED with reason “max QA attempts reached”
    - (optional) `auto_priority_boost_on_retry` bumps priority when returning to READY

## Acceptance Criteria
- [ ] Reject moves task to READY and increments `qa_attempts`.
- [ ] Reject records the reason in QA Report.
- [ ] When attempts exceed the max, task moves to BLOCKED (default policy).

## Implementation Notes
- Locks:
  - per-task lock + `workflow.lock` during edits/moves/commit
- Keep reject pure workflow-state mutation; no git operations required.

## Test Plan
### Integration
- submit task to QA
- reject with a reason
- verify task file is in READY and attempts incremented
- reject repeatedly until max; verify task ends in BLOCKED

## Validation
- `cargo test`
