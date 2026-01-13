# Burl (File-Driven eXecution)

This repo is the standalone home for the `burl` tool/spec.

## Key Docs

- PRD / source of truth: `burl.md`
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

## Code map (V1)

- `src/main.rs`: CLI entry + exit code mapping
- `src/cli/`: clap CLI definitions
- `src/commands/`: one module per command (`init/add/claim/submit/validate/approve/reject/...`)
- `src/fs/`: atomic writes + best-effort moves (`atomic.rs`, `move_file.rs`)
- `src/task.rs`: task file parse/serialize + mutation helpers
- `src/workflow.rs`: bucket indexing + filename/id helpers
- `src/locks.rs`: workflow/task/claim locks (RAII)
- `src/diff.rs`: diff parsing (`changed_files`, `added_lines`)
- `src/validate/`: `scope.rs`, `stubs.rs`
