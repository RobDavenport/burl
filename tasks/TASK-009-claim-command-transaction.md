---
id: TASK-009
title: Implement `fdx claim` transaction (READY → DOING)
priority: high
depends_on: [TASK-003, TASK-004, TASK-006, TASK-007, TASK-008]
---

## Objective
Implement `fdx claim [TASK-ID]` with race-safe locking and transactional behavior:
- choose a claimable READY task (or claim explicit ID)
- create branch + worktree at `base_sha`
- write claim metadata
- move the task file READY → DOING

## Context
Source of truth: `fdx.md` sections:
- “Race-safe claiming and transitions”
- “fdx claim (transaction)”
- “Conflict policy when declared scopes overlap”

## Requirements
### Selection
- If TASK-ID is omitted:
  - if `use_global_claim_lock: true`, acquire `claim.lock`
  - pick “next task” deterministically:
    - sort by `priority` (high > medium > low > none)
    - then by numeric ID ascending
    - only consider tasks in READY

### Dependency checks
- If `depends_on` contains tasks not in DONE:
  - fail claim with exit `1` and actionable reason
  - V1 default: do **not** auto-move the task; leave it in READY so it becomes claimable once deps are DONE

### Scope overlap checks (declared)
- Compare the claiming task’s declared allowed scope (`affects` + `affects_globs`) with tasks currently in DOING.
- Use a deterministic, conservative overlap heuristic that is easy to implement:
  - overlap if any explicit `affects` path in task A matches any `affects_globs` pattern in task B (and vice versa)
  - overlap if any explicit `affects` path is identical between tasks
  - overlap if any `affects_globs` pattern is identical between tasks
  - (optional improvement) if both globs are “directory globs” (e.g. `src/foo/**`), treat prefix relationships as overlap (`src/**` overlaps `src/foo/**`)
- Apply `conflict_policy`:
  - `fail`: fail claim (exit `1`) with a list of conflicting tasks
  - `warn`: print warning and continue
  - `ignore`: do nothing

### Re-claim behavior (after reject)
- If the task already has recorded `branch`/`worktree` values (from a prior claim/reject):
  - reuse the existing worktree if it exists and points at the recorded branch
  - do **not** silently change `base_sha` on reuse (PRD policy)
  - if the recorded worktree/branch is missing or invalid, fail with actionable remediation (run `fdx doctor` / recreate worktree) rather than guessing

### Claim transaction steps
1. Acquire per-task lock (`TASK-001.lock`).
2. Resolve `base_sha` (fetch remote/main first).
3. Create/reuse branch and worktree.
4. Verify the workflow worktree has no unexpected tracked modifications (PRD transactional precondition).
5. Acquire `workflow.lock` for workflow-state mutation:
   - atomically update task frontmatter:
     - `assigned_to`, `started_at`, `branch`, `worktree`, `base_sha`
   - atomically move task file READY → DOING (rename)
   - append `claim` event
   - commit workflow branch if enabled
   - if `workflow_auto_push: true`, push the workflow branch after committing
6. Release locks.

### Output
- Print the worktree path on success (so agents can `cd` into it).

### Failure/rollback
- If worktree creation fails after branch creation, delete the branch if it was created in this attempt.
- Never leave partially-written task files (atomic write requirement).

## Acceptance Criteria
- [ ] Claiming an explicit READY task moves it to DOING and sets `base_sha`.
- [ ] Claiming without ID selects deterministically.
- [ ] Two concurrent claim attempts never double-claim the same task.
- [ ] On failure, workflow state remains consistent (task not duplicated; no half-written task file).

## Test Plan
### Integration
- `fdx init` → `fdx add` → `fdx claim TASK-001`:
  - task file moved READY → DOING
  - branch/worktree created
  - `base_sha` non-null and matches `origin/main`

### Concurrency
- Create 2 READY tasks.
- Spawn two processes running `fdx claim` (no ID) concurrently:
  - both should succeed, but claim different tasks
- Spawn two processes both claiming the *same* task ID:
  - exactly one succeeds; other fails with lock exit code `4`

### Overlap policy
- Create two READY tasks with overlapping declared scope and set `conflict_policy: fail`:
  - claim the first task (moves to DOING)
  - claiming the second must fail with a clear conflict list

## Validation
- `cargo test`
