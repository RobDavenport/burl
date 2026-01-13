# Burl Roadmap

This document tracks what `burl` supports today and where it’s heading next.

## Current features (V1)

- **File/folder workflow state** in a canonical worktree: `.burl/.workflow/{READY,DOING,QA,DONE,BLOCKED}`
- **Bootstrap / idempotent init**: `burl init` creates the workflow branch/worktree and scaffolding
- **Task management**: `burl add`, `burl status`, `burl show`
- **Race-safety** with lock files: workflow/task/claim locks + `burl lock list|clear`
- **Per-task isolation** with Git branches + worktrees under `.worktrees/`: `burl claim`, `burl worktree`
- **Deterministic validation** against stored `base_sha`:
  - scope enforcement (`affects`, `affects_globs`, `must_not_touch`)
  - stub detection on **added lines only** (configurable patterns/extensions)
  - optional build/test hook via `build_command`
- **QA flow**: `burl submit` → `burl validate` → `burl approve` (rebase + `--ff-only`) / `burl reject`
- **Audit log**: append-only NDJSON events under `.burl/.workflow/events/`
- **Maintenance tools**: `burl doctor` (diagnostics/repairs) and `burl clean` (worktree cleanup)

## Future features (V2+)

- **Automation loop**: `burl watch` to auto-claim, run validations, and advance tasks
- **TUI / dashboard**: `burl monitor` (live status, lock ages, QA backlog)
- **Agent execution config**: formalize `agents.yaml` (still deterministic gating; no “self-judging”)
- **Richer validation pipeline**: multi-step validation profiles, per-language hooks, output summarization
- **Smarter conflict detection**: detect overlap via actual diffs/paths, not only declared scopes
- **Workflow layout migration**: safely move/rename workflow branch/worktree via config + migration tooling
- **Remote integrations**: PR creation, CI status checks, GitHub/GitLab linking
