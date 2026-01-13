---
id: TASK-010
title: Diff primitives: changed files + added lines (hunk parsing)
priority: high
depends_on: [TASK-002, TASK-008, TASK-009]
---

## Objective
Implement deterministic diff parsing utilities used by `submit`, `validate`, and `approve`:
- list changed files between `{base}..HEAD`
- parse `git diff -U0` hunks to extract **added lines only** with file + line numbers

## Context
Source of truth: `burl.md` section “Diff commands” and “Stub detection (diff-based)”.

## Requirements
### Changed files
- Provide a helper that returns repo-relative paths from:
  - `git diff --name-only {base}..HEAD`

### Added lines parsing
- Parse unified diff output from:
  - `git diff -U0 {base}..HEAD`
- For each file, capture a list of added lines:
  - file path
  - new-file line number (best-effort via hunk headers)
  - added line content (without leading `+`)
- Ignore diff metadata lines (`+++`, `---`, `diff --git`, etc.).
- Correctly handle:
  - new files (`/dev/null`)
  - file renames (treat as “changed file”; line mapping best-effort)

## Acceptance Criteria
- [ ] Given a known diff fixture, the parser returns the correct set of added lines and line numbers.
- [ ] Parser never flags context/removed lines as “added”.

## Implementation Notes
- Track hunk state via `@@ -old_start,old_len +new_start,new_len @@`.
- Maintain counters:
  - `old_line`, `new_line`
- For each diff line:
  - `+<content>`: record at `new_line`, then `new_line += 1`
  - `-<content>`: `old_line += 1`
  - ` <content>` (rare with `-U0`, but handle anyway): increment both
- Normalize file paths to repo-relative forward-slash form for glob matching.

## Test Plan
### Unit
- Parse a hard-coded diff string that includes:
  - one file edit with 2 added lines
  - one new file with added lines
  - one hunk with both `+` and `-` lines
- Assert resulting line numbers and contents.

## Validation
- `cargo test`

