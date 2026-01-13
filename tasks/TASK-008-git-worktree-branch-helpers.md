---
id: TASK-008
title: Git helpers for task branches + task worktrees
priority: high
depends_on: [TASK-002, TASK-005]
---

## Objective
Implement the git operations needed by `claim`, `submit`, `approve`, and cleanup:
- fetch main
- determine `base_sha`
- create/reuse task branch
- create/attach worktree
- remove worktree
- delete branch

## Context
Source of truth: `burl.md` sections “Git Worktree & Branch Model” and “Task worktree lifecycle”.

## Requirements
- Resolve `base_sha` as `<remote>/<main_branch>` HEAD at claim-time (after fetch).
- Branch naming convention:
  - default: `task-001-<slug>`
- Worktree path convention:
  - default: `.worktrees/task-001-<slug>/`
- Provide primitives that return actionable errors and map git failures to exit code `3`.

## Acceptance Criteria
- [ ] Can create a new branch at `base_sha` and a worktree for it.
- [ ] Can detect and reuse an existing worktree if it already exists and points at the right branch.
- [ ] Can remove a worktree and delete the branch (when safe).

## Implementation Notes
- Prefer git CLI over libgit2 for V1 parity and correctness.
- Suggested commands:
  - fetch: `git fetch <remote> <main_branch>`
  - base sha: `git rev-parse <remote>/<main_branch>`
  - create branch: `git branch <branch> <base_sha>` (or `git checkout -b` inside worktree)
  - add worktree: `git worktree add <path> <branch>`
  - remove worktree: `git worktree remove <path>` (avoid `--force` unless the user explicitly requested a force cleanup)
  - delete branch: `git branch -d <branch>` (avoid `-D` unless the user explicitly requested a force delete)
- Keep destructive commands behind explicit flags/config (PRD security section).
- If the configured `remote` does not exist, fail with a git error (exit `3`) and explain how to fix it (e.g. “set remote in config or add the remote”).

## Test Plan
### Integration (temp git repo)
- Create repo with two commits on main and a remote pointing to itself (or simulate remote via `git remote add origin <path>`)
- Fetch and resolve `origin/main` SHA
- Create task branch + worktree
- Verify `git -C <worktree> rev-parse --abbrev-ref HEAD` equals branch name

## Validation
- `cargo test`
