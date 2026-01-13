---
id: TASK-007
title: Implement `burl add`, `burl status`, `burl show`, `burl worktree`
priority: high
depends_on: [TASK-002, TASK-003, TASK-005, TASK-006]
---

## Objective
Implement the core “task file as durable state” commands for V1:
- create tasks in READY
- inspect tasks/buckets
- show a task’s details
- print a task worktree path (if present)

## Context
Source of truth: `burl.md` sections “Task management” and “Filesystem is truth”.

## Requirements
### `burl add "title" ...`
- Creates a new markdown file in `.burl/.workflow/READY/` named `TASK-{id}-{slug}.md`.
- Auto-assign a new numeric ID (monotonic):
  - scan all buckets for existing `TASK-###` and pick max+1
- Slugify title (lowercase, hyphens, remove punctuation; keep short but stable).
- Security:
  - generated filename must be strictly under `.burl/.workflow/READY/` (no path traversal)
  - slug must be limited to safe characters (e.g., `a-z0-9-`) and a reasonable length
- Write YAML frontmatter matching the PRD schema (fields may be null initially), including:
  - `created` timestamp (RFC3339)
  - `qa_attempts: 0`
- Support the PRD’s add flags (at minimum):
  - `--priority`
  - `--affects` (repeatable)
  - `--affects-globs` (repeatable)
  - `--must-not-touch` (repeatable)
  - `--depends-on` (repeatable)
  - `--tags` (repeatable)
- Write a starter body with headings:
  - Objective
  - Acceptance Criteria (checkbox list)
  - Context
  - Implementation Notes
  - QA Report
- If `workflow_auto_commit: true`, commit the workflow branch after creating the task.
- If `workflow_auto_push: true`, push the workflow branch after committing.
- Log an `add` event.

### Task ID validation (all commands that accept TASK-ID)
- Any command accepting a task ID argument (`show`, `claim`, `submit`, `validate`, `approve`, `reject`, `worktree`, lock ops) must:
  - reject IDs containing `/`, `\\`, or `..`
  - (recommended) require `^TASK-\\d{3,}$` and normalize input to uppercase
  - on failure: exit `1` with an actionable message

### `burl status`
- Print counts of tasks per bucket.
- Highlight:
  - locked tasks (lock file exists)
  - stale locks (based on config)
  - tasks in QA with `qa_attempts` near `qa_max_attempts`
  - stalled tasks (PRD term; define for V1):
    - DOING tasks with `started_at` older than 24 hours
    - QA tasks with `submitted_at` older than 24 hours

### `burl show TASK-001`
- Locate the task in any bucket and print it.
- Include the bucket name (READY/DOING/QA/DONE/BLOCKED) in the output header so status is obvious.
- If task not found, exit `1` with a helpful message listing buckets searched.

### `burl worktree TASK-001`
- Print the recorded `worktree` path from frontmatter.
- If null/missing, exit `1` with message “task has no worktree (not claimed?)”.

## Acceptance Criteria
- [ ] `burl add "Test task"` creates a valid task file in READY.
- [ ] `burl status` reflects the new task in READY count.
- [ ] `burl show TASK-001` prints the task content.
- [ ] `burl worktree TASK-001` errors cleanly before claim.

## Implementation Notes
- All workflow state writes/moves must hold `workflow.lock`.
- Consider a small “task index” helper that:
  - enumerates buckets
  - maps task ID → path
- Do not depend on frontmatter for status; bucket folder is canonical.

## Test Plan
### Integration
- `burl init`
- `burl add "My first task"`
- assert file exists under `.burl/.workflow/READY/`
- run `burl status` and assert READY count is 1
- run `burl show TASK-001` returns 0

### Security regression
- Attempt `burl show ../secrets` (or similar path traversal) and ensure it is rejected with exit `1`.

## Validation
- `cargo test`
