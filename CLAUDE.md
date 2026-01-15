# Burl (File-Driven eXecution)

## What this repo is

`burl` is a minimal, file-based workflow orchestrator for agentic coding/review pipelines.

## TL;DR

- Spec is `burl.md`; architecture map is `ARCHITECTURE.md`.
- Workflow state is folders + task markdown in the canonical worktree (default `.burl/` on branch `burl`).
- Keep gates deterministic and diff-based (added lines only).

## Start here

- `burl.md` — PRD/spec
- `ARCHITECTURE.md` — repo map + invariants
- `ROADMAP.md` — current vs future features

## Development / verification

Prefer mechanical validation over “looks good”:

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

If `cargo check`/`cargo clippy` fails with `Invalid cross-device link (os error 18)`, treat it as an environment/toolchain issue and use `cargo build` + `cargo test` as the validation gate.

## Implementation notes

- Workflow state is **folders + task markdown** stored in the canonical workflow worktree (default `.burl/` on branch `burl`).
- Scope/stub gates are **diff-based** and must remain deterministic (no scanning full files for TODOs; added lines only).
- Agent execution is subprocess-based and configured via `.burl/.workflow/agents.yaml` (`burl agent …`, `burl watch --dispatch`).
- Keep user-facing behavior (README + `burl.md`) consistent with implementation when changing command semantics.
