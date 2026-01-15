# Burl Roadmap

This document tracks what `burl` supports today and where it’s heading next.

## Current features

- **File/folder workflow state** in a canonical worktree: `.burl/.workflow/{READY,DOING,QA,DONE,BLOCKED}`
- **Bootstrap / idempotent init**: `burl init` creates the workflow branch/worktree and scaffolding
- **Task management**: `burl add`, `burl status`, `burl show`
- **Race-safety** with lock files: workflow/task/claim locks + `burl lock list|clear`
- **Per-task isolation** with Git branches + worktrees under `.worktrees/`: `burl claim`, `burl worktree`
- **Deterministic validation** against stored `base_sha`:
  - scope enforcement (`affects`, `affects_globs`, `must_not_touch`)
  - stub detection on **added lines only** (configurable patterns/extensions)
  - legacy single-step build/test hook via `build_command`
  - validation profiles: multi-step command pipelines with per-step conditions (globs/extensions)
- **Diff-aware scope conflict detection**: `conflict_detection: declared|diff|hybrid`
- **QA flow**: `burl submit` → `burl validate` → `burl approve` (rebase + `--ff-only`) / `burl reject`
- **Audit log**: append-only NDJSON events under `.burl/.workflow/events/`
- **Maintenance tools**: `burl doctor` (diagnostics/repairs) and `burl clean` (worktree cleanup)
- **Automation loop**: `burl watch` to auto-claim and process QA
- **TUI / dashboard**: `burl monitor` (alias: `visualizer`)
- **Agent execution**:
  - `agents.yaml` configuration with agent profiles, timeouts, environment, capabilities
  - Template-based prompt generation from task context
  - Subprocess execution with timeout and output capture (machine-local logs)
  - `burl agent run` / `burl agent list` commands
  - `burl watch --dispatch` for fully automated claim→dispatch→validate→approve loops
  - Event logging for agent dispatch/completion

## Future features

- **Workflow layout migration**: safely move/rename workflow branch/worktree via config + migration tooling
- **Remote integrations**: PR creation, CI status checks, GitHub/GitLab linking
