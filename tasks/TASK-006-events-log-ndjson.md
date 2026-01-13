---
id: TASK-006
title: Event log (NDJSON) append helper
priority: medium
depends_on: [TASK-003, TASK-005]
---

## Objective
Implement append-only event logging under `.burl/.workflow/events/` to support audit/recovery across machines.

## Context
Source of truth: `burl.md` section “Logging & Observability”.

## Requirements
- Implement an event record type that can serialize to a single-line JSON object.
- Append events to `.burl/.workflow/events/events.ndjson` (create file if missing).
- Ensure:
  - one JSON object per line
  - trailing newline after each append
- Required fields:
  - `ts` (RFC3339)
  - `action` (`init`/`add`/`claim`/`submit`/`validate`/`approve`/`reject`/`lock_clear`/`clean`)
  - `actor` (owner string)
  - `task` (optional for global events)
  - `details` (object; freeform)

### Wire logging into existing commands
- Update commands implemented earlier to emit required events:
  - `burl init` must append an `init` event after scaffolding is created (and commit/push workflow branch per config).
  - `burl lock clear` must append a `lock_clear` event including which lock was cleared, its age, and whether `--force` was used.
- Ensure event writes follow the PRD’s transactional guidance:
  - for commands that commit workflow state: append the event before committing, while holding `workflow.lock`.
  - for `burl lock clear` specifically: it must be able to clear `workflow.lock`, so logging/committing should be best-effort if `workflow.lock` cannot be acquired until after the lock is cleared.

## Acceptance Criteria
- [ ] An event append results in a valid NDJSON line.
- [ ] Repeated appends produce multiple lines (no overwrites).

## Implementation Notes
- Appends should occur while holding `workflow.lock` for commands that also commit workflow state, so the log and state move together.
- If JSON serialization fails, treat as a user-visible internal error (exit `1`) and do not proceed with state transitions.

## Test Plan
### Unit
- Append two events and verify the file has two lines of valid JSON.

## Validation
- `cargo test`
