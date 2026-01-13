---
id: TASK-012
title: Validation: stub detection on added lines (diff-based)
priority: high
depends_on: [TASK-002, TASK-003, TASK-010]
---

## Objective
Implement diff-based stub detection that scans **added lines only** for configured patterns.

## Context
Source of truth: `burl.md` section “Deterministic Validation → Stub detection (diff-based)”.

## Requirements
### Inputs
- Config:
  - `stub_patterns` (regex strings)
  - `stub_check_extensions` (file extensions)
- Diff added-lines summary from `{base}..HEAD`

### Rules
- Only consider files whose extension is in `stub_check_extensions`.
- Only scan added lines (not whole files).
- If any added line matches any compiled regex:
  - fail validation with exact file + line + matched content

### Regex engine + config errors (clarify ambiguity)
- Treat `stub_patterns` as Rust `regex` patterns (RE2-like; no look-around).
- If any pattern fails to compile:
  - treat as a config/user error (exit `1`), not a validation failure (`2`)
  - report which pattern is invalid and how to fix it (edit `config.yaml`)

## Acceptance Criteria
- [ ] A pre-existing `TODO` in an unchanged part of a file does not fail validation.
- [ ] A newly-added `TODO` line fails validation and reports the exact location.
- [ ] Stub scanning ignores files outside the configured extension list.

## Implementation Notes
- Compile regexes once per run (cache in a struct).
- Be careful to ignore diff header lines (`+++ b/file`).
- Return errors in a structured form so `burl submit`/`burl validate` can print helpful output and write QA reports.

## Test Plan
### Unit
- Diff fixture includes:
  - an added line: `+ // TODO: implement`
  - an added line in a `.md` file (should be ignored by extension filter)
- Verify only the relevant stub is detected.

## Validation
- `cargo test`
