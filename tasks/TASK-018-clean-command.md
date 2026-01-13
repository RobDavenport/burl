---
id: TASK-018
title: Implement `fdx clean` (completed/orphan worktrees cleanup; safe-by-default)
priority: medium
depends_on: [TASK-007, TASK-008, TASK-015, TASK-017]
---

## Objective
Implement `fdx clean` to remove local artifacts safely:
- completed task worktrees
- orphan worktree directories

## Context
Source of truth: `fdx.md` “clean” command note and “Security & Safety Considerations”.

## Requirements
- Default behavior is **dry-run**: print what would be removed.
- Requires `--force` (and/or `--yes`) to actually delete directories.
- Only operate under the configured worktree root (default `.worktrees/`).
- Cleanup candidates:
  - registered git worktrees under `.worktrees/` that are safe to remove (use `git worktree list --porcelain`)
  - directories in `.worktrees/` that are not referenced by any task file (filesystem orphans)
- Optional:
  - `--prune` runs `git worktree prune` after deletions

### Logging
- Append a `clean` event with a summary (removed count, skipped count, `--force`/`--prune` flags).
- If `workflow_auto_commit: true`, commit the workflow branch after writing the event.
- If `workflow_auto_push: true`, push the workflow branch after committing.

When writing the event and committing:
- acquire `workflow.lock` for the critical section (event write + commit)

## Acceptance Criteria
- [ ] `fdx clean` (no flags) performs no deletions and prints a plan.
- [ ] `fdx clean --force` removes only directories under `.worktrees/`.
- [ ] `fdx clean` never deletes branches by default.

## Implementation Notes
- Implement deletion using safe filesystem APIs; never allow `..` traversal.
- Show paths as repo-relative for readability.

## Test Plan
### Integration
- Create a fake orphan directory under `.worktrees/`.
- Run `fdx clean` and assert it reports the dir.
- Run `fdx clean --force` and assert the dir is deleted.

## Validation
- `cargo test`
