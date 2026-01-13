---
id: TASK-015
title: Implement `burl approve` (rebase + validate + ff-only merge + cleanup)
priority: high
depends_on: [TASK-006, TASK-008, TASK-013, TASK-014]
---

## Objective
Implement `burl approve TASK-ID` to safely merge a validated task into main and transition QA → DONE.

## Context
Source of truth: `burl.md` sections:
- “burl approve”
- “Deterministic Validation → Diff base selection”
- “Task worktree lifecycle”

## Requirements
### Preconditions
- Task must be in QA.
- Approval must only happen if validation passes.
- V1 rule (make deterministic): `burl approve` always runs validation internally (do not rely on parsing prior QA reports/events to determine “pass”).
- Task must have:
  - a recorded `worktree` path that exists locally
  - a recorded `branch` name
- Verify git state in the task worktree:
  - current branch matches the recorded `branch` (exit `1` with remediation if not)

### Git steps (strategy-based)
- Implement at least:
  - `merge_strategy: rebase_ff_only` (default): rebase onto `<remote>/<main_branch>`, then `git merge --ff-only`
  - `merge_strategy: ff_only`: skip rebase and require a fast-forward merge into main
  - `merge_strategy: manual`: exit `1` with a clear “not implemented in V1” message

For `rebase_ff_only`:
1. Fetch `<remote>/<main_branch>`.
2. In task worktree: rebase task branch onto `<remote>/<main_branch>`.
   - If conflicts:
     - reject with a deterministic reason (“rebase conflict”)
     - move task QA → READY (default V1 policy; preserve branch/worktree)
3. Run validation against the rebased base:
   - diff base: `<remote>/<main_branch>..HEAD` (not the original `base_sha`)
4. Merge into local `main` using `--ff-only`.
   - If fails: reject with reason “non-FF merge required”
5. If `push_main_on_approve: true`, push `main` to `<remote>`.

For `ff_only`:
1. Fetch `<remote>/<main_branch>`.
2. Verify the task branch is a descendant of `<remote>/<main_branch>` (e.g., `git merge-base --is-ancestor <remote>/<main_branch> HEAD` in the task worktree).
   - If not: reject with reason “branch behind main; rebase required”
3. Run validation against the task branch (diff base `<remote>/<main_branch>..HEAD`) and fail fast on scope/stubs/build/test.
4. (Optional but recommended) fast-forward local `main` to `<remote>/<main_branch>` (`git merge --ff-only <remote>/<main_branch>`).
5. Merge the task branch into local `main` using `--ff-only`.
   - If fails: reject with reason “non-FF merge required”
6. If `push_main_on_approve: true`, push `main` to `<remote>`.

### Workflow state updates
- On success:
  - set `completed_at`
  - move task QA → DONE
  - append `approve` event
  - cleanup task worktree + branch (configurable; default ON)
  - commit workflow branch if enabled
  - if `workflow_auto_push: true`, push the workflow branch after committing

### Failure policy
- If rebase/merge fails, approval must not partially merge changes.
- Default behavior on these failures:
  - append failure details to QA Report
  - run the equivalent of `burl reject --reason "<...>"` (preserve branch/worktree)

### Cleanup failure behavior (avoid ambiguity)
- Cleanup is best-effort:
  - if the merge to `main` succeeded but cleanup (worktree removal / branch delete) fails, still transition QA → DONE and record cleanup failures in the event details (and/or QA Report) so `burl clean` can address leftovers.

## Acceptance Criteria
- [ ] Approving a clean task fast-forwards main and moves task to DONE.
- [ ] Approving with a rebase conflict does not touch main and moves task out of QA with an actionable reason.
- [ ] Approving runs validation against the rebased base, not the original `base_sha`.
- [ ] Worktree cleanup removes the task worktree directory when configured.

## Implementation Notes
- Locks:
  - per-task lock for entire approve
  - do not hold `workflow.lock` during rebase/build/test
  - hold `workflow.lock` only for task file edits + bucket moves + event append + commit
- Verify the workflow worktree has no unexpected tracked modifications before committing workflow state (PRD transactional precondition).
- Ensure you are operating on the correct repos/paths:
  - rebase inside task worktree (`git -C <task_worktree> ...`)
  - merge inside repo root main worktree

## Test Plan
### Integration (happy path)
- init → add → claim → commit → submit → validate → approve
- assert:
  - main contains the commit
  - task file moved to DONE
  - worktree directory removed (if cleanup enabled)

### Integration (conflict path)
- Create a conflicting commit on main after claim.
- Attempt approve; assert:
  - main not merged
  - task moved out of QA with reason recorded

## Validation
- `cargo test`
