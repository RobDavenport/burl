---
id: TASK-001
title: Create `fdx` CLI scaffold (Rust) + command skeleton
priority: high
depends_on: []
---

## Objective
Create a minimal Rust CLI binary named `fdx` with a stable subcommand surface area matching the V1 PRD. This task is intentionally “thin”: wiring + placeholders only.

## Context
Source of truth: `fdx.md`, section “CLI Requirements (V1)”.

## Requirements
- Create a Rust binary crate (recommended: a standalone repo; if implemented in a mono-repo, keep it isolated in its own folder).
- The `fdx` binary must provide subcommands (even if some are not yet implemented):
  - `init`, `add`, `status`, `show`, `claim`, `submit`, `validate`, `approve`, `reject`, `worktree`
  - `lock list`, `lock clear`
  - `doctor`, `clean`
- Add a consistent error/exit-code mapping:
  - `0` success
  - `1` user error (bad args, invalid state)
  - `2` validation failure (scope/stubs/build)
  - `3` git operation failure
  - `4` lock acquisition failure
- Commands that are not implemented yet must exit `1` with a clear “not implemented” message (no panics).

## Acceptance Criteria
- [ ] `fdx --help` lists all required commands/subcommands.
- [ ] Running any unimplemented command prints a user-actionable message and exits `1`.
- [ ] `cargo test` passes (even if there are 0 tests yet).

## Implementation Notes
- Recommended crates:
  - CLI: `clap` (derive)
  - Errors: `anyhow` + `thiserror` (or one, but keep messages actionable)
  - Logging: `tracing` + `tracing-subscriber` (optional for V1; stdout/stderr is fine)
- Add a top-level “command dispatcher” module so later tasks can implement subcommands without touching argument parsing.

## Test Plan
### Manual
- `fdx --help`
- `fdx lock --help`
- `fdx init --help`

## Validation
- `cargo test`
- `cargo run -- --help`

