---
id: TASK-014
title: Implement `burl validate` (QA) including build/test hook + QA report writing
priority: high
depends_on: [TASK-003, TASK-006, TASK-011, TASK-012, TASK-013]
---

## Objective
Implement `burl validate TASK-ID` to run deterministic checks and record results without changing the bucket.

## Context
Source of truth: `burl.md` sections:
- “burl validate”
- “Build/Test validation”
- “Logging & Observability”

## Requirements
### Preconditions
- Task must be in QA bucket.
- Task must have:
  - a recorded `worktree` path that exists locally
  - a recorded `branch` name
  - a non-null `base_sha`
- Verify git state in the task worktree:
  - current branch matches the recorded `branch`
  - if mismatch/missing: exit `1` and record a QA report entry describing the mismatch

### Validation steps
Run, in order:
1. Scope validation (`{base_sha}..HEAD`)
2. Stub validation (`{base_sha}..HEAD`)
3. Build/test validation if `build_command` is non-empty:
   - run in the task worktree directory
   - non-zero exit → fail
   - capture output summary for QA report

### Output capture limits (avoid huge QA files)
- Capture full stdout/stderr for display, but write only a bounded summary into the task’s QA Report (e.g., last N lines or last N KB) to keep workflow commits small and readable.

### Recording results
- Append a structured entry under “QA Report” in the task file including:
  - timestamp
  - pass/fail per gate
  - short failure reasons (file lists, stub matches, command exit code)
- Append a `validate` event (with summary and pass/fail).
- Commit workflow branch if enabled.
- If `workflow_auto_push: true`, push the workflow branch after committing.

### Exit codes
- Pass: `0`
- Validation failure: `2`
- User/state errors: `1`
- Git failures: `3`
- Lock failures: `4`

## Acceptance Criteria
- [ ] `burl validate` does not move the task out of QA.
- [ ] Failures produce exit `2` and write a QA report entry.
- [ ] A passing validation produces exit `0` and writes a QA report entry.

## Implementation Notes
- Locks:
  - acquire per-task lock for entire validation
  - do not hold `workflow.lock` while running build/test (long-running)
  - acquire `workflow.lock` only to write QA report + event + commit
- Build command execution (make deterministic):
  - Parse `build_command` into an argv array using a simple shell-words parser.
  - Run via `std::process::Command` with explicit argv (no shell), in the task worktree directory.
  - If parsing fails (e.g., unmatched quotes), treat as config/user error (exit `1`) and record the error in the QA report.

## Test Plan
### Integration
- With `build_command` empty: ensure validate runs scope/stubs only.
- With a known failing build command (platform-dependent): ensure exit `2` and QA report records failure.

## Validation
- `cargo test`
