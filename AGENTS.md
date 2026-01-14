# Burl (File-Driven eXecution)

This repo is the standalone home for the `burl` tool/spec.

## TL;DR

- Start with `burl.md` (spec) and `ARCHITECTURE.md` (code map).
- Keep validation gates deterministic and diff-based (no model self-judging).
- Prefer `cargo fmt && cargo test && cargo clippy --all-targets -- -D warnings` as the mechanical gate.

## Key Docs

- PRD / source of truth: `burl.md`
- Architecture map: `ARCHITECTURE.md`
- Roadmap: `ROADMAP.md`
- Implementation task breakdown: `tasks/README.md`

## Development Notes

- Target: cross-platform Rust CLI (`burl`).
- Prefer deterministic, diff-based validation gates (no model self-judging).
- Favor atomic filesystem operations; avoid shell-only assumptions.

## Fast validation

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Note: some environments hit `Invalid cross-device link (os error 18)` for `cargo check`/`cargo clippy`. If that happens, use `cargo build` + `cargo test` as the mechanical gate until the toolchain/filesystem issue is resolved.

## Code map

- `src/main.rs`: CLI entry + exit code mapping
- `src/cli/`: clap CLI definitions
- `src/commands/`: one module per command (`init/add/claim/submit/validate/approve/reject/watch/monitor/agent/...`)
- `src/agent/`: agent execution core (V2: config, prompt generation, dispatch)
- `src/fs/`: atomic writes + best-effort moves (`atomic.rs`, `move_file.rs`)
- `src/task/`: task file parse/serialize + mutation helpers
- `src/workflow.rs`: bucket indexing + filename/id helpers
- `src/locks/`: workflow/task/claim locks (RAII)
- `src/diff/`: diff parsing (`changed_files`, `added_lines`)
- `src/validate/`: deterministic gates (`scope`, `stubs`)
