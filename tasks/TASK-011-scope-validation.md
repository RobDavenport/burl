---
id: TASK-011
title: Validation: scope enforcement (affects/affects_globs/must_not_touch)
priority: high
depends_on: [TASK-003, TASK-010]
---

## Objective
Implement deterministic scope validation as defined in the PRD:
- forbid touching `must_not_touch`
- require all changed files to be within declared allowed scope

## Context
Source of truth: `burl.md` section “Deterministic Validation → Scope enforcement”.

## Requirements
### Inputs
- Task frontmatter:
  - `affects` (explicit paths)
  - `affects_globs` (optional)
  - `must_not_touch` (globs)
- Diff summary from `{base}..HEAD`:
  - changed file list

### Rules
- S1: if any changed file matches `must_not_touch` → fail.
- S2: every changed file must match at least one allowed path/glob (`affects ∪ affects_globs`) → fail otherwise.
- New files are only allowed if they match an allowed glob or allowed directory pattern.

### Output
- On failure, produce a structured error containing:
  - violating files
  - which rule they violated
  - the matching glob (when applicable)

## Acceptance Criteria
- [ ] A change outside `affects/affects_globs` fails with a list of offending files.
- [ ] A change matching `must_not_touch` fails even if it is also in allowed scope.
- [ ] A new file under an allowed glob passes.

## Implementation Notes
- Use a glob library with predictable semantics (recommended: `globset`).
- Normalize paths consistently (repo-relative, forward slashes).
- Consider treating explicit `affects` entries as exact-match allow rules.

## Test Plan
### Unit
- Allowed-only: changed file in `src/foo.rs` with `affects: [src/foo.rs]` → pass
- Forbidden: changed file in `src/net/**` with `must_not_touch: [src/net/**]` → fail
- Out-of-scope: changed file not matching any allow → fail
- New file: `affects_globs: [src/player/**]` and new file `src/player/jump.rs` → pass

## Validation
- `cargo test`

